// src/index/mod.rs
//
// Index management system for NopalDB

pub mod hash;
pub mod btree;
#[cfg(feature = "fulltext")]
pub mod fulltext;
pub mod storage;
pub mod taxonomy;

use crate::error::Result;
use crate::types::{NodeId, PropertyValue};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

pub use hash::HashIndex;
pub use btree::BTreeIndex;
#[cfg(feature = "fulltext")]
pub use fulltext::FullTextIndex;
pub use taxonomy::TaxonomyIndex;

/// Index type
#[derive(Debug, Clone, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum IndexType {
    /// Hash index for equality lookups (O(1))
    Hash,
    /// B-Tree index for range queries (O(log N))
    BTree,
    /// Full-text search index
    FullText,
    /// OWL subClassOf DAG with transitive closure (BFS incremental)
    Taxonomy,
}

/// Index query operations
#[derive(Debug, Clone)]
pub enum IndexQuery {
    /// Exact match: property = value
    Equals(PropertyValue),

    /// Range: property > value
    GreaterThan(PropertyValue),

    /// Range: property >= value
    GreaterThanOrEqual(PropertyValue),

    /// Range: property < value
    LessThan(PropertyValue),

    /// Range: property <= value
    LessThanOrEqual(PropertyValue),

    /// Range: value1 <= property <= value2
    Between(PropertyValue, PropertyValue),

    /// Full-text search
    FullText(String),
}

/// Generic index trait
pub trait Index: Send + Sync {
    /// Insert a value into the index
    fn insert(&mut self, value: PropertyValue, node_id: NodeId) -> Result<()>;

    /// Remove a value from the index
    fn remove(&mut self, value: &PropertyValue, node_id: NodeId) -> Result<()>;

    /// Query the index
    fn query(&self, query: &IndexQuery) -> Result<Vec<NodeId>>;

    /// Clear all entries
    fn clear(&mut self) -> Result<()>;

    /// Get index size (number of entries)
    fn size(&self) -> usize;

    /// Add a directed relationship between two nodes.
    /// Only meaningful for `TaxonomyIndex` (subClassOf edge: source → child).
    /// All other index types return `Ok(())` by default.
    fn add_relationship(&mut self, source: NodeId, target: NodeId) -> Result<()> {
        let _ = (source, target);
        Ok(())
    }

    /// Downcast to `TaxonomyIndex` for synchronous ontology queries.
    ///
    /// Only `TaxonomyIndex` returns `Some`; all other implementations return `None`.
    fn as_taxonomy(&self) -> Option<&TaxonomyIndex> {
        None
    }
}

/// Index metadata
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexMetadata {
    pub name: String,
    pub label: String,
    pub property: String,
    pub index_type: IndexType,
    #[serde(with = "chrono::serde::ts_seconds")]
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub size: usize,
}

/// Index manager - coordinates all indexes
pub struct IndexManager {
    /// All indexes by name
    indexes: Arc<RwLock<HashMap<String, Box<dyn Index>>>>,

    /// Index metadata
    metadata: Arc<RwLock<HashMap<String, IndexMetadata>>>,

    /// Base path for persistent indexes
    base_path: Option<String>,
}

impl IndexManager {
    /// Create new index manager
    pub fn new(base_path: Option<String>) -> Self {
        // Ensure directory exists if path is provided
        if let Some(path) = &base_path
            && let Err(e) = std::fs::create_dir_all(path)
        {
            log::warn!("Failed to create index directory {}: {}", path, e);
        }

        IndexManager {
            indexes: Arc::new(RwLock::new(HashMap::new())),
            metadata: Arc::new(RwLock::new(HashMap::new())),
            base_path,
        }
    }

    /// Load indices from disk and rebuild them
    pub async fn load_indices(&self, storage: &crate::storage::Storage) -> Result<()> {
        if let Some(base_path) = &self.base_path {
            let metadata_path = std::path::Path::new(base_path).join("metadata.bin");

            if metadata_path.exists() {
                log::info!("Loading index metadata from {:?}", metadata_path);
                match storage::load_metadata(&metadata_path) {
                    Ok(loaded_metadata) => {
                        // Step 1: verify + repair malformed/duplicated metadata before rebuilding.
                        let mut repaired = false;
                        let mut unique_metadata: HashMap<String, IndexMetadata> = HashMap::new();

                        for meta in loaded_metadata {
                            if meta.name.trim().is_empty()
                                || meta.label.trim().is_empty()
                                || meta.property.trim().is_empty()
                            {
                                repaired = true;
                                log::warn!("Skipping malformed index metadata entry: {:?}", meta);
                                continue;
                            }

                            match unique_metadata.get(&meta.name) {
                                None => {
                                    unique_metadata.insert(meta.name.clone(), meta);
                                }
                                Some(prev) => {
                                    repaired = true;
                                    // Keep latest by created_at as repair strategy.
                                    if meta.created_at > prev.created_at {
                                        unique_metadata.insert(meta.name.clone(), meta);
                                    }
                                }
                            }
                        }

                        if unique_metadata.is_empty() {
                            log::warn!("No valid index metadata entries after repair");
                            if repaired {
                                self.save_metadata_internal(&unique_metadata)?;
                            }
                            return Ok(());
                        }

                        // Step 2: create empty runtime indexes.
                        let mut rebuilt_indexes: HashMap<String, Box<dyn Index>> = HashMap::new();
                        let mut rebuilt_meta: HashMap<String, IndexMetadata> = HashMap::new();
                        let mut indexes_by_label: HashMap<String, Vec<(String, String)>> = HashMap::new();
                        // Track taxonomy index names for the post-loop rebuild pass.
                        let mut taxonomy_names: Vec<String> = Vec::new();

                        for (index_name, meta) in &unique_metadata {
                            let index: Box<dyn Index> = match meta.index_type {
                                IndexType::Hash => Box::new(HashIndex::new()),
                                IndexType::BTree => Box::new(BTreeIndex::new()),
                                IndexType::FullText => {
                                    #[cfg(feature = "fulltext")]
                                    {
                                        let path = format!("{}/fulltext_{}", base_path, index_name);
                                        Box::new(FullTextIndex::new(Some(path))?)
                                    }
                                    #[cfg(not(feature = "fulltext"))]
                                    {
                                        return Err(crate::error::NopalError::index_error(
                                            "Full-text indexes require the `fulltext` Cargo feature".to_string()
                                        ));
                                    }
                                }
                                IndexType::Taxonomy => {
                                    taxonomy_names.push(index_name.clone());
                                    Box::new(TaxonomyIndex::new())
                                }
                            };

                            rebuilt_indexes.insert(index_name.clone(), index);
                            rebuilt_meta.insert(index_name.clone(), meta.clone());

                            // Taxonomy uses its own rebuild pass; skip the label-property map.
                            if meta.index_type != IndexType::Taxonomy {
                                indexes_by_label
                                    .entry(meta.label.clone())
                                    .or_default()
                                    .push((index_name.clone(), meta.property.clone()));
                            }
                        }

                        // Step 3: single-pass rebuild across all nodes (incremental by label/property).
                        let nodes = storage.get_all_nodes().await?;
                        let mut counts: HashMap<String, usize> = HashMap::new();
                        for node in &nodes {
                            if let Some(indexes_for_label) = indexes_by_label.get(&node.label) {
                                for (index_name, property) in indexes_for_label {
                                    if let Some(value) = node.properties.get(property)
                                        && let Some(index) = rebuilt_indexes.get_mut(index_name) {
                                            index.insert(value.clone(), node.id)?;
                                            *counts.entry(index_name.clone()).or_insert(0) += 1;
                                    }
                                }
                            }
                        }

                        // Step 3b: rebuild taxonomy indexes (separate pass over nodes + edges).
                        if !taxonomy_names.is_empty() {
                            let all_edges = storage.get_all_edges().await?;
                            for tax_name in &taxonomy_names {
                                if let Some(meta) = unique_metadata.get(tax_name) {
                                    let label = &meta.label;
                                    let edge_type = &meta.property;
                                    // Register all Class nodes.
                                    for node in &nodes {
                                        if &node.label == label
                                            && let Some(idx) = rebuilt_indexes.get_mut(tax_name)
                                        {
                                            idx.insert(
                                                PropertyValue::String(node.label.clone()),
                                                node.id,
                                            )?;
                                            *counts.entry(tax_name.clone()).or_insert(0) += 1;
                                        }
                                    }
                                    // Wire subClassOf edges.
                                    for edge in &all_edges {
                                        if &edge.edge_type == edge_type
                                            && let Some(idx) = rebuilt_indexes.get_mut(tax_name)
                                        {
                                            idx.add_relationship(edge.source, edge.target)?;
                                        }
                                    }
                                    log::info!(
                                        "Rebuilt taxonomy index {} (label={}, edge_type={})",
                                        tax_name, label, edge_type
                                    );
                                }
                            }
                        }

                        // Step 4: repair metadata sizes and persist if needed.
                        for (index_name, meta) in &mut rebuilt_meta {
                            let count = counts.get(index_name).copied().unwrap_or(0);
                            if meta.size != count {
                                repaired = true;
                                meta.size = count;
                            }
                            log::info!("Rebuilt index {} with {} entries", index_name, count);
                        }

                        {
                            let mut indexes = self.indexes.write().await;
                            let mut metadata = self.metadata.write().await;
                            *indexes = rebuilt_indexes;
                            *metadata = rebuilt_meta.clone();
                        }

                        if repaired {
                            log::info!("Persisting repaired index metadata");
                            self.save_metadata_internal(&rebuilt_meta)?;
                        }
                    }
                    Err(e) => {
                        log::error!("Failed to load index metadata: {}", e);
                        // Don't fail the whole startup, just start with empty indices
                    }
                }
            }
        }
        Ok(())
    }

    /// Create a new index
    pub async fn create_index(
        &self,
        label: &str,
        property: &str,
        index_type: IndexType,
    ) -> Result<String> {
        let index_name = format!("{}_{}", label, property);

        // Check if index already exists
        let indexes = self.indexes.read().await;
        if indexes.contains_key(&index_name) {
            return Err(crate::error::NopalError::index_error(
                format!("Index {} already exists", index_name)
            ));
        }
        drop(indexes);

        // Create the index
        let index: Box<dyn Index> = match index_type {
            IndexType::Hash => Box::new(HashIndex::new()),
            IndexType::BTree => Box::new(BTreeIndex::new()),
            IndexType::FullText => {
                #[cfg(feature = "fulltext")]
                {
                    let path = self.base_path.as_ref()
                        .map(|p| format!("{}/fulltext_{}", p, index_name));
                    Box::new(FullTextIndex::new(path)?)
                }
                #[cfg(not(feature = "fulltext"))]
                {
                    return Err(crate::error::NopalError::index_error(
                        "Full-text indexes require the `fulltext` Cargo feature".to_string(),
                    ));
                }
            }
            IndexType::Taxonomy => Box::new(TaxonomyIndex::new()),
        };

        // If we are creating an index on an existing graph, we should populate it?
        // Current implementation of CREATE INDEX in executor typically handles population?
        // Actually, executor calls create_index then populates.
        // But here we just create it empty. The executor is responsible for population if it's a new index on existing data.
        // WAIT: The executor logic for CREATE INDEX currently calls `executor.execute_create_index`.
        // Let's check executor logic later. For now, we assume this creates an empty index ready to be populated.

        // Store metadata
        let metadata = IndexMetadata {
            name: index_name.clone(),
            label: label.to_string(),
            property: property.to_string(),
            index_type: index_type.clone(),
            created_at: chrono::Utc::now(),
            size: 0,
        };

        let mut indexes = self.indexes.write().await;
        let mut meta = self.metadata.write().await;

        indexes.insert(index_name.clone(), index);
        meta.insert(index_name.clone(), metadata.clone());

        // Drop locks before saving to avoid deadlock if save is slow (though it's sync file IO)
        drop(indexes);
        // We need metadata map to save
        self.save_metadata_internal(&meta)?;
        drop(meta);

        log::info!("Created {:?} index: {}", index_type, index_name);

        Ok(index_name)
    }

    /// Drop an index
    pub async fn drop_index(&self, index_name: &str) -> Result<()> {
        let mut indexes = self.indexes.write().await;
        let mut metadata = self.metadata.write().await;

        indexes.remove(index_name);
        metadata.remove(index_name);

        self.save_metadata_internal(&metadata)?;

        log::info!("Dropped index: {}", index_name);
        Ok(())
    }

    /// Internal helper to save metadata
    fn save_metadata_internal(&self, metadata: &HashMap<String, IndexMetadata>) -> Result<()> {
        if let Some(base_path) = &self.base_path {
            let metadata_path = std::path::Path::new(base_path).join("metadata.bin");
            let meta_vec: Vec<IndexMetadata> = metadata.values().cloned().collect();
            storage::save_metadata(&metadata_path, &meta_vec)?;
        }
        Ok(())
    }

    /// Insert into index
    pub async fn insert(
        &self,
        index_name: &str,
        value: PropertyValue,
        node_id: NodeId,
    ) -> Result<()> {
        let mut indexes = self.indexes.write().await;

        if let Some(index) = indexes.get_mut(index_name) {
            index.insert(value, node_id)?;

            // Update metadata size
            drop(indexes);
            let mut metadata = self.metadata.write().await;
            if let Some(meta) = metadata.get_mut(index_name) {
                meta.size = meta.size.saturating_add(1);
                // Note: We don't save metadata on every insert for performance
                // We rely on rebuild at startup to get correct size/content
            }
        }

        Ok(())
    }

    /// Remove from index
    pub async fn remove(
        &self,
        index_name: &str,
        value: &PropertyValue,
        node_id: NodeId,
    ) -> Result<()> {
        let mut indexes = self.indexes.write().await;

        if let Some(index) = indexes.get_mut(index_name) {
            index.remove(value, node_id)?;

            // Update metadata size
            drop(indexes);
            let mut metadata = self.metadata.write().await;
            if let Some(meta) = metadata.get_mut(index_name) {
                meta.size = meta.size.saturating_sub(1);
            }
        }

        Ok(())
    }

    /// Add a directed relationship between two nodes in a named index.
    /// Meaningful only for `IndexType::Taxonomy`; other index types ignore the call.
    pub async fn add_relationship(
        &self,
        index_name: &str,
        source: NodeId,
        target: NodeId,
    ) -> Result<()> {
        let mut indexes = self.indexes.write().await;
        if let Some(index) = indexes.get_mut(index_name) {
            index.add_relationship(source, target)?;
        }
        Ok(())
    }

    /// Query an index
    pub async fn query(
        &self,
        index_name: &str,
        query: &IndexQuery,
    ) -> Result<Vec<NodeId>> {
        let indexes = self.indexes.read().await;

        if let Some(index) = indexes.get(index_name) {
            index.query(query)
        } else {
            Err(crate::error::NopalError::index_error(
                format!("Index not found: {}", index_name)
            ))
        }
    }

    /// Get all indexes for a label
    pub async fn get_indexes_for_label(&self, label: &str) -> Vec<String> {
        let metadata = self.metadata.read().await;
        metadata.values()
            .filter(|m| m.label == label)
            .map(|m| m.name.clone())
            .collect()
    }

    /// Get index metadata
    pub async fn get_metadata(&self, index_name: &str) -> Option<IndexMetadata> {
        let metadata = self.metadata.read().await;
        metadata.get(index_name).cloned()
    }

    /// List all indexes
    pub async fn list_indexes(&self) -> Vec<IndexMetadata> {
        let metadata = self.metadata.read().await;
        metadata.values().cloned().collect()
    }

    /// Get index for property (if exists)
    pub async fn find_index(&self, label: &str, property: &str) -> Option<String> {
        let index_name = format!("{}_{}", label, property);
        let indexes = self.indexes.read().await;
        if indexes.contains_key(&index_name) {
            Some(index_name)
        } else {
            None
        }
    }

    /// Return a cloned snapshot of the first [`TaxonomyIndex`] found, for
    /// synchronous use in query evaluation (e.g. `instanceOf` / `subClassOf`).
    ///
    /// Uses `try_read()` to avoid blocking; returns `None` if no taxonomy index
    /// exists or if the lock is momentarily contended.
    pub fn get_taxonomy_sync(&self) -> Option<TaxonomyIndex> {
        let indexes = self.indexes.try_read().ok()?;
        for index in indexes.values() {
            if let Some(tax) = index.as_taxonomy() {
                return Some(tax.clone());
            }
        }
        None
    }

    /// Return a cloned [`TaxonomyIndex`] if one exists, or an empty one if not.
    ///
    /// Used by `Graph::import_turtle()` to obtain a mutable taxonomy to update
    /// during import, before calling [`set_taxonomy`] to persist the result.
    pub fn get_or_create_taxonomy(&self) -> TaxonomyIndex {
        self.get_taxonomy_sync().unwrap_or_default()
    }

    /// Store (or replace) the given [`TaxonomyIndex`] in the manager under the
    /// reserved key `"_taxonomy"`.
    ///
    /// This does not persist metadata to disk — taxonomy content is rebuilt from
    /// graph data on startup via `load_indices`.
    pub async fn set_taxonomy(&self, taxonomy: TaxonomyIndex) {
        let mut indexes = self.indexes.write().await;
        indexes.insert("_taxonomy".to_string(), Box::new(taxonomy));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_index_manager_create() {
        let manager = IndexManager::new(None);

        // Create hash index
        let index_name = manager.create_index("Person", "name", IndexType::Hash)
            .await
            .unwrap();

        assert_eq!(index_name, "Person_name");

        // Verify metadata
        let meta = manager.get_metadata(&index_name).await.unwrap();
        assert_eq!(meta.label, "Person");
        assert_eq!(meta.property, "name");
        assert_eq!(meta.index_type, IndexType::Hash);
    }

    #[tokio::test]
    async fn test_index_manager_duplicate() {
        let manager = IndexManager::new(None);

        manager.create_index("Person", "name", IndexType::Hash)
            .await
            .unwrap();

        // Try to create duplicate
        let result = manager.create_index("Person", "name", IndexType::Hash).await;
        assert!(result.is_err());
    }
}
