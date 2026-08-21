// src/index/fulltext.rs
//
// Full-text search index using Tantivy

use crate::error::{NopalError, Result};
use crate::types::{NodeId, PropertyValue};
use crate::index::{Index, IndexQuery};
use tantivy::*;
use tantivy::schema::*;
use tantivy::query::QueryParser;
use tantivy::collector::TopDocs;
use std::path::PathBuf;

/// Full-text search index powered by Tantivy
pub struct FullTextIndex {
    index: tantivy::Index,
    reader: IndexReader,
    writer: Option<IndexWriter>,
    node_id_field: Field,
    content_field: Field,
}

impl FullTextIndex {
    /// Create new full-text index
    pub fn new(path: Option<String>) -> Result<Self> {
        // Build schema
        let mut schema_builder = Schema::builder();

        let node_id_field = schema_builder.add_text_field("node_id", STRING | STORED);
        let content_field = schema_builder.add_text_field("content", TEXT);

        let schema = schema_builder.build();

        // Create or open the index. `create_in_dir` fails if a tantivy index
        // already exists on disk, which broke reopening a persisted database
        // (the index is rebuilt from metadata on every open). `open_or_create`
        // opens the existing index when present and creates it otherwise.
        let index = if let Some(path) = path {
            let path = PathBuf::from(path);
            std::fs::create_dir_all(&path)
                .map_err(|e| NopalError::index_error(format!("Failed to create index directory: {}", e)))?;
            let dir = tantivy::directory::MmapDirectory::open(&path)
                .map_err(|e| NopalError::index_error(format!("Failed to open index directory: {}", e)))?;
            tantivy::Index::open_or_create(dir, schema.clone())
                .map_err(|e| NopalError::index_error(format!("Failed to open or create index: {}", e)))?
        } else {
            tantivy::Index::create_in_ram(schema.clone())
        };

        // Create reader
        let reader = index.reader_builder()
            .reload_policy(tantivy::ReloadPolicy::OnCommitWithDelay)
            .try_into()
            .map_err(|e| NopalError::index_error(format!("Failed to create reader: {}", e)))?;

        // Create writer (50MB heap)
        let writer = index.writer(50_000_000)
            .map_err(|e| NopalError::index_error(format!("Failed to create writer: {}", e)))?;

        Ok(FullTextIndex {
            index,
            reader,
            writer: Some(writer),
            node_id_field,
            content_field,
        })
    }

    /// Commit pending changes
    fn commit(&mut self) -> Result<()> {
        if let Some(writer) = &mut self.writer {
            writer.commit()
                .map_err(|e| NopalError::index_error(format!("Failed to commit: {}", e)))?;
            // Reload reader so queries see the committed documents immediately
            self.reader.reload()
                .map_err(|e| NopalError::index_error(format!("Failed to reload reader: {}", e)))?;
        }
        Ok(())
    }
}

impl Index for FullTextIndex {
    fn insert(&mut self, value: PropertyValue, node_id: NodeId) -> Result<()> {
        // Only index string values
        let text = match value {
            PropertyValue::String(s) => s,
            _ => return Err(NopalError::index_error(
                "Full-text index only supports string values".to_string()
            )),
        };

        if let Some(writer) = &mut self.writer {
            // Un nodo tiene UN documento en este índice: el índice es por
            // (label, propiedad), así que hay un solo valor por nodo — y
            // `remove` ya borra por node_id ignorando el valor, es decir, ya
            // trata el documento como identificado por el nodo.
            //
            // Sin este delete previo, reindexar un nodo (re-ingesta de la
            // misma fuente, corrección de un texto) AÑADE un segundo
            // documento: el texto viejo sigue matcheando y el índice crece
            // sin techo. Ambos deletes y el add se publican en el mismo
            // commit de abajo, así que una consulta nunca ve los dos.
            let term = Term::from_field_text(self.node_id_field, &node_id.to_string());
            writer.delete_term(term);

            // Create document using tantivy's doc! macro
            let doc = doc!(
                self.node_id_field => node_id.to_string(),
                self.content_field => text
            );

            writer.add_document(doc)
                .map_err(|e| NopalError::index_error(format!("Failed to add document: {}", e)))?;

            // Commit after each insert (could batch for performance)
            self.commit()?;
        }

        Ok(())
    }

    fn remove(&mut self, _value: &PropertyValue, node_id: NodeId) -> Result<()> {
        if let Some(writer) = &mut self.writer {
            let term = Term::from_field_text(self.node_id_field, &node_id.to_string());
            writer.delete_term(term);
            self.commit()?;
        }
        Ok(())
    }

    fn query(&self, query: &IndexQuery) -> Result<Vec<NodeId>> {
        Ok(self
            .query_scored(query)?
            .into_iter()
            .map(|(id, _)| id)
            .collect())
    }

    /// Igual que `query`, conservando el score BM25 que tantivy ya calcula.
    ///
    /// El ranking se ordenaba por score y luego se tiraba el número, así que
    /// quien quisiera saber POR QUÉ un documento quedó arriba tenía que
    /// reconstruirlo por fuera. `query` delega aquí para que no existan dos
    /// recorridos del índice que puedan divergir.
    fn query_scored(&self, query: &IndexQuery) -> Result<Vec<(NodeId, Option<f32>)>> {
        let query_text = match query {
            IndexQuery::FullText(text) => text,
            _ => return Err(NopalError::index_error(
                "Full-text index only supports full-text queries".to_string()
            )),
        };

        let searcher = self.reader.searcher();

        // Parse query
        let query_parser = QueryParser::for_index(&self.index, vec![self.content_field]);
        let query = query_parser.parse_query(query_text)
            .map_err(|e| NopalError::index_error(format!("Failed to parse query: {}", e)))?;

        // Search
        let top_docs = searcher.search(&query, &TopDocs::with_limit(1000).order_by_score())
            .map_err(|e| NopalError::index_error(format!("Search failed: {}", e)))?;

        // Extract node IDs
        let mut node_ids = Vec::new();
        for (score, doc_address) in top_docs {
            // Use turbofish to specify TantivyDocument type
            let retrieved_doc = searcher.doc::<tantivy::TantivyDocument>(doc_address)
                .map_err(|e| NopalError::index_error(format!("Failed to retrieve document: {}", e)))?;

            // Get all values for node_id field
            for field_value in retrieved_doc.get_all(self.node_id_field) {
                // CompactDocValue tiene as_str() method
                if let Some(text) = field_value.as_str()
                    && let Ok(node_id) = uuid::Uuid::parse_str(text) {
                        node_ids.push((node_id, Some(score)));
                        break; // Solo necesitamos el primer valor
                }
            }
        }

        Ok(node_ids)
    }

    fn clear(&mut self) -> Result<()> {
        if let Some(writer) = &mut self.writer {
            writer.delete_all_documents()
                .map_err(|e| NopalError::index_error(format!("Failed to clear: {}", e)))?;
            self.commit()?;
        }
        Ok(())
    }

    fn size(&self) -> usize {
        let searcher = self.reader.searcher();
        searcher.num_docs() as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fulltext_index_basic() {
        let mut index = FullTextIndex::new(None).unwrap();

        let node1 = uuid::Uuid::new_v4();
        let node2 = uuid::Uuid::new_v4();
        let node3 = uuid::Uuid::new_v4();

        // Insert documents
        index.insert(
            PropertyValue::String("fraud detection in financial networks".to_string()),
            node1
        ).unwrap();

        index.insert(
            PropertyValue::String("machine learning for anomaly detection".to_string()),
            node2
        ).unwrap();

        index.insert(
            PropertyValue::String("harbor cay investigation".to_string()),
            node3
        ).unwrap();

        // Search — Tantivy uses OR by default for multi-word queries
        // "fraud detection" matches docs containing "fraud" OR "detection"
        let results = index.query(&IndexQuery::FullText("fraud detection".to_string())).unwrap();
        assert_eq!(results.len(), 2); // Both doc1 (fraud detection) and doc2 (anomaly detection)
        assert!(results.contains(&node1));
        assert!(results.contains(&node2));

        // Single word search
        let results = index.query(&IndexQuery::FullText("detection".to_string())).unwrap();
        assert_eq!(results.len(), 2); // Both fraud detection and anomaly detection

        // Use AND for exact phrase matching: +fraud +detection
        let results = index.query(&IndexQuery::FullText("+fraud +detection".to_string())).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results.contains(&node1));

        let results = index.query(&IndexQuery::FullText("harbor".to_string())).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results.contains(&node3));
    }

    #[test]
    fn test_fulltext_index_boolean() {
        let mut index = FullTextIndex::new(None).unwrap();

        let node1 = uuid::Uuid::new_v4();
        let node2 = uuid::Uuid::new_v4();

        index.insert(
            PropertyValue::String("fraud detection algorithms".to_string()),
            node1
        ).unwrap();

        index.insert(
            PropertyValue::String("fraud prevention systems".to_string()),
            node2
        ).unwrap();

        // Boolean AND
        let results = index.query(&IndexQuery::FullText("fraud AND detection".to_string())).unwrap();
        assert_eq!(results.len(), 1);
        assert!(results.contains(&node1));

        // Boolean OR
        let results = index.query(&IndexQuery::FullText("detection OR prevention".to_string())).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_fulltext_index_remove() {
        let mut index = FullTextIndex::new(None).unwrap();

        let node1 = uuid::Uuid::new_v4();
        let value = PropertyValue::String("test document".to_string());

        index.insert(value.clone(), node1).unwrap();

        let results = index.query(&IndexQuery::FullText("test".to_string())).unwrap();
        assert_eq!(results.len(), 1);

        // Remove
        index.remove(&value, node1).unwrap();

        let results = index.query(&IndexQuery::FullText("test".to_string())).unwrap();
        assert_eq!(results.len(), 0);
    }

    /// Reindexar un nodo REEMPLAZA su documento en vez de añadir otro.
    ///
    /// Se afirma sobre `size()` (los documentos que tantivy realmente guarda)
    /// y no sobre `query()`: los llamadores de arriba deduplican por NodeId,
    /// así que una consulta devuelve un solo hit tenga el índice uno o cinco
    /// documentos — el crecimiento sería invisible hasta que el disco lo grite.
    #[test]
    fn reindexing_a_node_replaces_its_document() {
        let mut index = FullTextIndex::new(None).unwrap();
        let node = uuid::Uuid::new_v4();

        for i in 0..5 {
            index
                .insert(PropertyValue::String(format!("revision {i} del texto")), node)
                .unwrap();
            assert_eq!(index.size(), 1, "tras {} escrituras debe haber 1 documento", i + 1);
        }

        // Solo la última revisión matchea.
        assert_eq!(
            index.query(&IndexQuery::FullText("4".to_string())).unwrap().len(),
            1
        );
        assert_eq!(
            index.query(&IndexQuery::FullText("0".to_string())).unwrap().len(),
            0,
            "el texto de una revisión anterior no puede seguir matcheando"
        );
    }

    /// Dos nodos distintos conviven: el delete previo del reindexado borra por
    /// node_id, así que no puede llevarse por delante el documento de otro.
    #[test]
    fn reindexing_one_node_does_not_touch_another() {
        let mut index = FullTextIndex::new(None).unwrap();
        let (a, b) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());

        index.insert(PropertyValue::String("alfa comun".into()), a).unwrap();
        index.insert(PropertyValue::String("beta comun".into()), b).unwrap();
        index.insert(PropertyValue::String("gamma comun".into()), a).unwrap();

        assert_eq!(index.size(), 2, "un documento por nodo");
        assert_eq!(
            index.query(&IndexQuery::FullText("comun".to_string())).unwrap().len(),
            2
        );
        assert_eq!(
            index.query(&IndexQuery::FullText("beta".to_string())).unwrap(),
            vec![b],
            "el documento del otro nodo sigue intacto"
        );
    }
}