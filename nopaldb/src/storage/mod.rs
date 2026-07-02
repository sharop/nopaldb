// src/storage/mod.rs

pub mod backend;

use std::sync::Arc;
use std::path::Path;
use std::collections::HashMap;
use sled::Db;
use crate::error::{NopalError, Result};
use crate::types::{Node, Edge, NodeId, EdgeId, PropertyValue};
use crate::mvcc::{VersionedNode, VersionedEdge};
pub use backend::{StorageBackend, StorageEngine, StorageOptions, StorageProfile, StorageTuning};

/// Storage engine basado en sled
/// Storage engine basado en sled.
///
/// Sled es thread-safe internamente (Send + Sync) con MVCC propio.
/// No requiere locking externo — todas las operaciones son concurrentes.
pub struct Storage {
    db: Arc<Db>,
    profile: StorageProfile,
}

fn serialize<T: serde::Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    rmp_serde::to_vec(value)
        .map_err(|e| NopalError::SerializationError(format!("MessagePack serialize error: {}", e)))
}

fn deserialize<'a, T: serde::de::Deserialize<'a>>(bytes: &'a [u8]) -> Result<T> {
    rmp_serde::from_slice(bytes)
        .map_err(|e| NopalError::SerializationError(format!("MessagePack deserialize error: {}", e)))
}

impl Storage {
    #[cfg(feature = "embeddings")]
    fn open_embeddings_tree_sync(&self) -> Result<sled::Tree> {
        Ok(self.db.open_tree("embeddings")?)
    }

    /// Crea una nueva instancia de storage
    pub async fn new(path: impl AsRef<Path>) -> Result<Self> {
        Self::new_with_options(path, StorageOptions::default()).await
    }

    /// Crea una nueva instancia de storage con opciones explícitas.
    pub async fn new_with_options(
        path: impl AsRef<Path>,
        options: StorageOptions,
    ) -> Result<Self> {
        match options.engine {
            StorageEngine::Sled => Self::new_with_profile(path, options.profile).await,
        }
    }

    /// Crea una nueva instancia de storage con perfil de tuning.
    pub async fn new_with_profile(path: impl AsRef<Path>, profile: StorageProfile) -> Result<Self> {
        let tuning = profile.tuning();
        let mut config = sled::Config::new().path(path.as_ref());

        if let Some(cache_capacity_bytes) = tuning.cache_capacity_bytes {
            config = config.cache_capacity(cache_capacity_bytes);
        }
        config = config.flush_every_ms(tuning.flush_every_ms);
        config = config.use_compression(tuning.use_compression);

        let db = config.open().map_err(NopalError::StorageError)?;

        Ok(Self {
            db: Arc::new(db),
            profile,
        })
    }

    /// Crea storage en memoria (útil para tests)
    pub async fn in_memory() -> Result<Self> {
        Self::in_memory_with_options(StorageOptions::default()).await
    }

    /// Crea storage en memoria con opciones explícitas.
    pub async fn in_memory_with_options(options: StorageOptions) -> Result<Self> {
        match options.engine {
            StorageEngine::Sled => Self::in_memory_with_profile(options.profile).await,
        }
    }

    /// Crea storage en memoria con perfil de tuning.
    pub async fn in_memory_with_profile(profile: StorageProfile) -> Result<Self> {
        let tuning = profile.tuning();
        let mut config = sled::Config::new().temporary(true);

        if let Some(cache_capacity_bytes) = tuning.cache_capacity_bytes {
            config = config.cache_capacity(cache_capacity_bytes);
        }
        config = config.flush_every_ms(tuning.flush_every_ms);
        config = config.use_compression(tuning.use_compression);

        let db = config.open().map_err(NopalError::StorageError)?;

        Ok(Self {
            db: Arc::new(db),
            profile,
        })
    }

    pub fn backend_name(&self) -> &'static str {
        "sled"
    }

    /// Inserta un nodo
    pub async fn insert_node(&self, node: &Node) -> Result<()> {
        let key = format!("node:{}", node.id);
        let value = serialize(node)?;

        self.db.insert(key.as_bytes(), value)?;

        Ok(())
    }

    /// Obtiene un nodo por ID
    pub async fn get_node(&self, id: NodeId) -> Result<Node> {
        let key = format!("node:{}", id);

        let value = self.db.get(key.as_bytes())?
            .ok_or_else(|| NopalError::NodeNotFound(id.to_string()))?;

        let node: Node = deserialize(&value)?;

        Ok(node)
    }

    /// Elimina un nodo
    pub async fn delete_node(&self, id: NodeId) -> Result<()> {
        let key = format!("node:{}", id);

        self.db.remove(key.as_bytes())?
            .ok_or_else(|| NopalError::NodeNotFound(id.to_string()))?;

        Ok(())
    }

    /// Inserta una arista
    pub async fn insert_edge(&self, edge: &Edge) -> Result<()> {
        let key = edge.id.to_string();
        let value = serialize(edge)?;

        let edges_tree = self.db.open_tree("edges")?;  // ← Tree "edges"
        edges_tree.insert(key.as_bytes(), value)?;

        Ok(())

    }

    /// Obtiene una arista por ID
    pub async fn get_edge(&self, id: EdgeId) -> Result<Edge> {
        let key = id.to_string();

        let edges_tree = self.db.open_tree("edges")?;  // ← Tree "edges"
        let value = edges_tree.get(key.as_bytes())?
            .ok_or_else(|| NopalError::EdgeNotFound(id.to_string()))?;

        let edge: Edge = deserialize(&value)?;
        Ok(edge)
    }

    pub async fn node_exists(&self, id: NodeId) -> Result<bool> {
        let key = format!("node:{}", id);

        Ok(self.db.contains_key(key.as_bytes())?)
    }

    /// Verifica si una arista existe
    pub async fn edge_exists(&self, id: EdgeId) -> Result<bool> {
        let key = id.to_string();

        let edges_tree = self.db.open_tree("edges")?;  // ← Tree "edges"
        Ok(edges_tree.contains_key(key.as_bytes())?)
    }

    /// Elimina una arista del storage
    pub async fn delete_edge(&self, id: EdgeId) -> Result<()> {
        let key = id.to_string();

        let edges_tree = self.db.open_tree("edges")?;  // ← Tree "edges"
        edges_tree.remove(key.as_bytes())?;

        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════════════
    // VERSIONED EDGES — MVCC para aristas
    // Tree "versioned_edges": key = "{edge_id}:v{version}", value = VersionedEdge (MessagePack)
    // Tree "versioned_edges_current": key = edge_id, value = current VersionedEdge
    // ═══════════════════════════════════════════════════════════════════════

    /// Inserta la primera versión de una arista en el historial MVCC.
    /// Debe llamarse justo después de `insert_edge()`.
    pub async fn insert_versioned_edge(&self, edge: &Edge, timestamp: u64) -> Result<()> {
        let versioned = VersionedEdge::new(edge.clone(), timestamp);
        let key = format!("{}:v{:020}", edge.id, versioned.version);
        let value = serialize(&versioned)?;
        let current_value = serialize(&versioned)?;

        let tree = self.db.open_tree("versioned_edges")?;
        tree.insert(key.as_bytes(), value)?;

        let current_tree = self.db.open_tree("versioned_edges_current")?;
        current_tree.insert(edge.id.to_string().as_bytes(), current_value)?;

        Ok(())
    }

    /// Obtiene la versión actual de una arista del historial MVCC.
    pub async fn get_current_versioned_edge(&self, id: EdgeId) -> Result<VersionedEdge> {
        
        let current_tree = self.db.open_tree("versioned_edges_current")?;
        let value = current_tree
            .get(id.to_string().as_bytes())?
            .ok_or_else(|| NopalError::EdgeNotFound(id.to_string()))?;
        let versioned: VersionedEdge = deserialize(&value)?;
        Ok(versioned)
    }

    /// Marca una arista como eliminada: cierra su valid_to en la versión actual.
    /// Debe llamarse justo antes de `delete_edge()`.
    pub async fn mark_edge_deleted(&self, id: EdgeId, timestamp: u64) -> Result<()> {
        let current = self.get_current_versioned_edge(id).await?;
        // Reescribir la entrada del historial con valid_to
        let closed = current.with_valid_to(timestamp);
        let key = format!("{}:v{:020}", id, closed.version);
        let value = serialize(&closed)?;

        let tree = self.db.open_tree("versioned_edges")?;
        tree.insert(key.as_bytes(), value)?;

        // Eliminar la entrada current (la arista ya no está activa)
        let current_tree = self.db.open_tree("versioned_edges_current")?;
        current_tree.remove(id.to_string().as_bytes())?;

        Ok(())
    }

    /// Retorna todas las versiones de una arista, ordenadas de más antigua a más reciente.
    pub async fn get_edge_history(&self, id: EdgeId) -> Result<Vec<VersionedEdge>> {
        let prefix = format!("{}:v", id);
        
        let tree = self.db.open_tree("versioned_edges")?;

        let mut versions: Vec<VersionedEdge> = tree
            .scan_prefix(prefix.as_bytes())
            .filter_map(|r| r.ok())
            .filter_map(|(_, v)| deserialize::<VersionedEdge>(&v).ok())
            .collect();

        versions.sort_by_key(|v| v.version);
        Ok(versions)
    }

    /// Retorna todas las aristas de un tipo específico válidas en `timestamp`.
    /// Escanea el historial MVCC completo — O(total versioned edges).
    pub async fn get_versioned_edges_of_type_at(
        &self,
        edge_type: &str,
        timestamp: u64,
    ) -> Result<Vec<Edge>> {
        
        let tree = self.db.open_tree("versioned_edges")?;

        // Track seen edge_ids to only include the best (latest valid) version per edge
        let mut best: HashMap<EdgeId, VersionedEdge> = HashMap::new();

        for result in tree.iter() {
            let (_, v) = result?;
            if let Ok(ve) = deserialize::<VersionedEdge>(&v)
                && ve.edge_data.edge_type == edge_type
                && ve.is_valid_at(timestamp)
            {
                let entry = best.entry(ve.id).or_insert_with(|| ve.clone());
                if ve.version > entry.version {
                    *entry = ve;
                }
            }
        }

        Ok(best.into_values().map(|ve| ve.edge_data).collect())
    }

    /// Guarda el índice de adyacencia saliente de un nodo
    pub async fn save_adjacency_out(&self, node_id: NodeId, edges: &[EdgeId]) -> Result<()> {
        let key = format!("idx:out:{}", node_id);
        let value = serialize(edges)?;

        self.db.insert(key.as_bytes(), value)?;

        Ok(())
    }

    /// Carga el índice de adyacencia saliente de un nodo
    pub async fn load_adjacency_out(&self, node_id: NodeId) -> Result<Vec<EdgeId>> {
        let key = format!("idx:out:{}", node_id);

        let value = self.db.get(key.as_bytes())?;

        match value {
            Some(v) => {
                let edges: Vec<EdgeId> = deserialize(&v)?;
                Ok(edges)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Guarda el índice de adyacencia entrante de un nodo
    pub async fn save_adjacency_in(&self, node_id: NodeId, edges: &[EdgeId]) -> Result<()> {
        let key = format!("idx:in:{}", node_id);
        let value = serialize(edges)?;

        self.db.insert(key.as_bytes(), value)?;

        Ok(())
    }

    /// Carga el índice de adyacencia entrante de un nodo
    pub async fn load_adjacency_in(&self, node_id: NodeId) -> Result<Vec<EdgeId>> {
        let key = format!("idx:in:{}", node_id);

        let value = self.db.get(key.as_bytes())?;

        match value {
            Some(v) => {
                let edges: Vec<EdgeId> = deserialize(&v)?;
                Ok(edges)
            }
            None => Ok(Vec::new()),
        }
    }

    /// Carga todos los índices de adyacencia (para reconstruir al abrir el grafo)
    pub async fn load_all_adjacency_indices(&self) -> Result<(
        HashMap<NodeId, Vec<EdgeId>>,  // adjacency_out
        HashMap<NodeId, Vec<EdgeId>>,  // adjacency_in
    )> {
        let mut adjacency_out = HashMap::new();
        let mut adjacency_in = HashMap::new();


        // Iterar sobre todas las keys que empiezan con "idx:"
        for item in self.db.scan_prefix(b"idx:") {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);

            if key_str.starts_with("idx:out:") {
                if let Some(node_id_str) = key_str.strip_prefix("idx:out:")
                    && let Ok(node_id) = uuid::Uuid::parse_str(node_id_str) {
                        let edges: Vec<EdgeId> = deserialize(&value)?;
                        adjacency_out.insert(node_id, edges);
                }
            } else if key_str.starts_with("idx:in:")
                && let Some(node_id_str) = key_str.strip_prefix("idx:in:")
                && let Ok(node_id) = uuid::Uuid::parse_str(node_id_str) {
                    let edges: Vec<EdgeId> = deserialize(&value)?;
                    adjacency_in.insert(node_id, edges);
            }
        }

        Ok((adjacency_out, adjacency_in))
    }

    /// Reconstruye índices desde cero escaneando todas las aristas
    pub async fn rebuild_indices(&self) -> Result<(
        HashMap<NodeId, Vec<EdgeId>>,
        HashMap<NodeId, Vec<EdgeId>>,
    )> {
        let mut adjacency_out: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();
        let mut adjacency_in: HashMap<NodeId, Vec<EdgeId>> = HashMap::new();

        let edges_tree = self.db.open_tree("edges")?;

        // Escanear todas las aristas del tree "edges"
        for item in edges_tree.iter() {
            let (_, value) = item?;
            let edge: Edge = deserialize(&value)?;

            // Actualizar índice out
            adjacency_out
                .entry(edge.source)
                .or_default()
                .push(edge.id);

            // Actualizar índice in
            adjacency_in
                .entry(edge.target)
                .or_default()
                .push(edge.id);
        }

        Ok((adjacency_out, adjacency_in))
    }
    /// Guarda un índice de propiedad: clave -> valor -> lista de nodos
    pub async fn save_property_index(&self, property: &str, value: &PropertyValue, node_id: NodeId) -> Result<()> {
        let value_str = match value {
            PropertyValue::String(s) => s.clone(),
            PropertyValue::Int(i) => i.to_string(),
            PropertyValue::Float(f) => f.to_string(),
            PropertyValue::Bool(b) => b.to_string(),
            PropertyValue::Null => "null".to_string(),
            PropertyValue::Bytes(_) | PropertyValue::List(_) | PropertyValue::Object(_) => {
                return Ok(());
            } // No indexamos valores estructurados/binarios en F2
        };

        // Clave del índice: idx:prop:{prop_name}:{prop_value}
        let key = format!("idx:prop:{}:{}", property, value_str);

        // 1. Leer lista actual de nodos para este valor
        let mut nodes: Vec<NodeId> = match self.db.get(key.as_bytes())? {
            Some(v) => deserialize(&v)?,
            None => Vec::new(),
        };

        // 2. Agregar ID si no existe
        if !nodes.contains(&node_id) {
            nodes.push(node_id);

            // 3. Guardar lista actualizada
            let value_bytes = serialize(&nodes)?;
            self.db.insert(key.as_bytes(), value_bytes)?;
        }

        Ok(())
    }

    /// Remueve un NodeId de un índice de propiedad
    pub async fn remove_from_property_index(
        &self,
        property: &str,
        value: &PropertyValue,
        node_id: NodeId,
    ) -> Result<()> {
        let value_str = match value {
            PropertyValue::String(s) => s.clone(),
            PropertyValue::Int(i) => i.to_string(),
            PropertyValue::Float(f) => f.to_string(),
            PropertyValue::Bool(b) => b.to_string(),
            PropertyValue::Null => "null".to_string(),
            PropertyValue::Bytes(_) | PropertyValue::List(_) | PropertyValue::Object(_) => return Ok(()),
        };

        let key = format!("idx:prop:{}:{}", property, value_str);

        let mut nodes: Vec<NodeId> = match self.db.get(key.as_bytes())? {
            Some(v) => deserialize(&v)?,
            None => return Ok(()),
        };

        nodes.retain(|&id| id != node_id);

        if nodes.is_empty() {
            self.db.remove(key.as_bytes())?;
        } else {
            let value_bytes = serialize(&nodes)?;
            self.db.insert(key.as_bytes(), value_bytes)?;
        }

        Ok(())
    }

    /// Obtiene lista de nodos que tienen una propiedad con cierto valor
    pub async fn get_nodes_by_property(&self, property: &str, value: &PropertyValue) -> Result<Vec<NodeId>> {
        let value_str = match value {
            PropertyValue::String(s) => s.clone(),
            PropertyValue::Int(i) => i.to_string(),
            PropertyValue::Float(f) => f.to_string(),
            PropertyValue::Bool(b) => b.to_string(),
            PropertyValue::Null => "null".to_string(),
            _ => return Ok(Vec::new()),
        };

        let key = format!("idx:prop:{}:{}", property, value_str);

        match self.db.get(key.as_bytes())? {
            Some(v) => {
                let nodes: Vec<NodeId> = deserialize(&v)?;
                Ok(nodes)
            }
            None => Ok(Vec::new()),
        }
    }

    // ═════════════════════════════════════════════════════════
    // ✅ MÉTODOS DE EMBEDDINGS
    // ═════════════════════════════════════════════════════════

    /// Comprueba (de forma síncrona) si existe un embedding para `node_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn node_embedding_exists_sync(&self, node_id: crate::types::NodeId, model: &str) -> bool {
        self.try_node_embedding_exists_sync(node_id, model).unwrap_or(false)
    }

    /// Comprueba (sync, con semántica estricta) si existe un embedding para `node_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn try_node_embedding_exists_sync(
        &self,
        node_id: crate::types::NodeId,
        model: &str,
    ) -> Result<bool> {
        let key = format!("{}:{}", node_id, model);
        let tree = self.open_embeddings_tree_sync()?;
        Ok(tree.contains_key(key.as_bytes())?)
    }

    /// Carga (sync) el embedding de `node_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn load_node_embedding_sync(
        &self,
        node_id: NodeId,
        model: &str,
    ) -> Result<crate::embeddings::Embedding> {
        let key = format!("{}:{}", node_id, model);
        let tree = self.open_embeddings_tree_sync()?;
        let value = tree
            .get(key.as_bytes())?
            .ok_or_else(|| NopalError::custom(format!("Embedding not found for node {} model {}", node_id, model)))?;
        let embedding: crate::embeddings::Embedding = deserialize(&value)?;
        Ok(embedding)
    }

    /// Comprueba (sync, con semántica estricta) si existe un embedding para `edge_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn try_edge_embedding_exists_sync(
        &self,
        edge_id: EdgeId,
        model: &str,
    ) -> Result<bool> {
        let key = format!("e:{}:{}", edge_id, model);
        let tree = self.open_embeddings_tree_sync()?;
        Ok(tree.contains_key(key.as_bytes())?)
    }

    /// Carga (sync, estricta) el embedding de `edge_id` y `model`.
    #[cfg(feature = "embeddings")]
    pub fn load_edge_embedding_sync(
        &self,
        edge_id: EdgeId,
        model: &str,
    ) -> Result<crate::embeddings::EdgeEmbedding> {
        let key = format!("e:{}:{}", edge_id, model);
        let tree = self.open_embeddings_tree_sync()?;
        let value = tree
            .get(key.as_bytes())?
            .ok_or_else(|| NopalError::custom(format!("Embedding not found for edge {} model {}", edge_id, model)))?;
        let embedding: crate::embeddings::EdgeEmbedding = deserialize(&value)?;
        Ok(embedding)
    }

    #[cfg(feature = "embeddings")]
    pub async fn save_node_embedding(&self, embedding: &crate::embeddings::Embedding) -> Result<()> {
        let key = format!("{}:{}", embedding.node_id, embedding.model);
        let value = serialize(embedding)?;
        
        let tree = self.db.open_tree("embeddings")?;
        tree.insert(key.as_bytes(), value)?;
        Ok(())
    }

    #[cfg(feature = "embeddings")]
    pub async fn load_node_embedding(&self, node_id: NodeId, model: &str) -> Result<crate::embeddings::Embedding> {
        let key = format!("{}:{}", node_id, model);
        
        let tree = self.db.open_tree("embeddings")?;
        let value = tree.get(key.as_bytes())?
            .ok_or_else(|| NopalError::custom(format!("Embedding not found for node {} model {}", node_id, model)))?;
        let embedding: crate::embeddings::Embedding = deserialize(&value)?;
        Ok(embedding)
    }

    #[cfg(feature = "embeddings")]
    pub async fn save_edge_embedding(&self, embedding: &crate::embeddings::EdgeEmbedding) -> Result<()> {
        // Prefijo "e:" distingue aristas de nodos en el mismo árbol Sled
        let key = format!("e:{}:{}", embedding.edge_id, embedding.model);
        let value = serialize(embedding)?;
        
        let tree = self.db.open_tree("embeddings")?;
        tree.insert(key.as_bytes(), value)?;
        Ok(())
    }

    #[cfg(feature = "embeddings")]
    pub async fn load_edge_embedding(&self, edge_id: EdgeId, model: &str) -> Result<crate::embeddings::EdgeEmbedding> {
        let key = format!("e:{}:{}", edge_id, model);
        
        let tree = self.db.open_tree("embeddings")?;
        let value = tree.get(key.as_bytes())?
            .ok_or_else(|| NopalError::custom(format!("Embedding not found for edge {} model {}", edge_id, model)))?;
        let embedding: crate::embeddings::EdgeEmbedding = deserialize(&value)?;
        Ok(embedding)
    }

    // ───────────────────────────────────────────────────────────
    // E-8: PathReferenceEmbedding — árbol "path_ref_embeddings"
    // ───────────────────────────────────────────────────────────

    #[cfg(feature = "embeddings")]
    fn open_path_ref_tree_sync(&self) -> Result<sled::Tree> {
        Ok(self.db.open_tree("path_ref_embeddings")?)
    }

    /// Persiste una referencia de path embedding (E-8).
    #[cfg(feature = "embeddings")]
    pub async fn save_path_reference_embedding(
        &self,
        emb: &crate::embeddings::PathReferenceEmbedding,
    ) -> Result<()> {
        emb.validate()?;
        let key = crate::embeddings::PathReferenceEmbedding::storage_key(
            &emb.name, &emb.node_model, &emb.edge_model,
        );
        let value = serialize(emb)?;
        
        let tree = self.db.open_tree("path_ref_embeddings")?;
        tree.insert(key.as_bytes(), value)?;
        Ok(())
    }

    /// Carga (sync) una referencia de path embedding por (name, node_model, edge_model).
    #[cfg(feature = "embeddings")]
    pub fn load_path_reference_embedding_sync(
        &self,
        name: &str,
        node_model: &str,
        edge_model: &str,
    ) -> Result<crate::embeddings::PathReferenceEmbedding> {
        let key = crate::embeddings::PathReferenceEmbedding::storage_key(name, node_model, edge_model);
        let tree = self.open_path_ref_tree_sync()?;
        match tree.get(key.as_bytes())? {
            Some(bytes) => {
                let emb: crate::embeddings::PathReferenceEmbedding = deserialize(&bytes)?;
                Ok(emb)
            }
            None => Err(NopalError::QueryExecutionError(format!(
                "PathReferenceEmbedding '{}' (node_model={}, edge_model={}) not found",
                name, node_model, edge_model
            ))),
        }
    }

    /// Comprueba (sync) si existe una referencia de path embedding.
    #[cfg(feature = "embeddings")]
    pub fn path_reference_embedding_exists_sync(
        &self,
        name: &str,
        node_model: &str,
        edge_model: &str,
    ) -> Result<bool> {
        let key = crate::embeddings::PathReferenceEmbedding::storage_key(name, node_model, edge_model);
        let tree = self.open_path_ref_tree_sync()?;
        Ok(tree.contains_key(key.as_bytes())?)
    }

    /// Carga (sync) todas las PathReferenceEmbedding para el par (node_model, edge_model).
    /// Itera el árbol completo y filtra por la clave "name\x00node_model\x00edge_model".
    /// Retorna lista vacía si no hay referencias para ese par de modelos.
    #[cfg(feature = "embeddings")]
    pub fn load_all_path_references_for_models_sync(
        &self,
        node_model: &str,
        edge_model: &str,
    ) -> Result<Vec<crate::embeddings::PathReferenceEmbedding>> {
        let tree = self.open_path_ref_tree_sync()?;
        let mut results = Vec::new();
        for item in tree.iter() {
            let (key_bytes, val_bytes) = item?;
            let key = std::str::from_utf8(&key_bytes)
                .map_err(|e| NopalError::custom(e.to_string()))?;
            // Clave: "name\x00node_model\x00edge_model"
            let parts: Vec<&str> = key.splitn(3, '\x00').collect();
            if parts.len() == 3 && parts[1] == node_model && parts[2] == edge_model {
                let emb: crate::embeddings::PathReferenceEmbedding = deserialize(&val_bytes)?;
                results.push(emb);
            }
        }
        Ok(results)
    }

    /// Retorna todos los embeddings de nodo que pertenecen al modelo `model`.
    /// Las claves de nodo tienen formato `{uuid}:{model}` (sin prefijo `e:`).
    #[cfg(feature = "embeddings")]
    pub async fn load_all_node_embeddings_for_model(
        &self,
        model: &str,
    ) -> Result<Vec<crate::embeddings::Embedding>> {
        let suffix = format!(":{}", model);
        
        let tree = self.db.open_tree("embeddings")?;
        let mut result = Vec::new();
        for item in tree.iter() {
            let (key_bytes, val_bytes) = item?;
            let key = std::str::from_utf8(&key_bytes)
                .map_err(|e| NopalError::custom(e.to_string()))?;
            // Excluir aristas (prefijo "e:") y filtrar por modelo
            if !key.starts_with("e:") && key.ends_with(&suffix) {
                let emb: crate::embeddings::Embedding = deserialize(&val_bytes)?;
                result.push(emb);
            }
        }
        Ok(result)
    }

    // ═════════════════════════════════════════════════════════
    // ✅ MÉTODOS MVCC
    // ═════════════════════════════════════════════════════════

    /// Inserta una versión de nodo (MVCC)
    pub async fn insert_node_version(&self, versioned: &VersionedNode) -> Result<()> {
        

        // 1. Guardar versión
        let version_key = format!("node:{}:v{}", versioned.id, versioned.version);
        let version_value = serialize(versioned)?;

        self.db.insert(version_key.as_bytes(), version_value)?;

        // 2. Actualizar puntero current (si es la versión más reciente)
        if versioned.valid_to.is_none() {
            let current_key = format!("node:{}:current", versioned.id);
            let version_bytes = versioned.version.to_le_bytes();
            self.db.insert(current_key.as_bytes(), version_bytes.as_ref())?;
        }

        // 3. Agregar a lista de versiones
        let versions_key = format!("node:{}:versions", versioned.id);
        let mut versions: Vec<u64> = match self.db.get(versions_key.as_bytes())? {
            Some(v) => deserialize(&v)?,
            None => Vec::new(),
        };

        if !versions.contains(&versioned.version) {
            versions.push(versioned.version);
            versions.sort_unstable();
            versions.reverse(); // Más reciente primero

            let versions_value = serialize(&versions)?;
            self.db.insert(versions_key.as_bytes(), versions_value)?;
        }

        // 4. Indexar por timestamp
        let ts_key = format!("ts:{}", versioned.timestamp);
        let mut node_ids: Vec<NodeId> = match self.db.get(ts_key.as_bytes())? {
            Some(v) => deserialize(&v)?,
            None => Vec::new(),
        };

        if !node_ids.contains(&versioned.id) {
            node_ids.push(versioned.id);
            let ts_value = serialize(&node_ids)?;
            self.db.insert(ts_key.as_bytes(), ts_value)?;
        }

        log::debug!("Inserted node version: {} v{}", versioned.id, versioned.version);

        Ok(())
    }

    /// Obtiene la versión actual de un nodo
    pub async fn get_current_version(&self, id: NodeId) -> Result<u64> {
        let current_key = format!("node:{}:current", id);

        let value = self.db.get(current_key.as_bytes())?
            .ok_or_else(|| NopalError::NodeNotFound(id.to_string()))?;

        let version = u64::from_le_bytes(
            value.as_ref().try_into()
                .map_err(|_| NopalError::Custom("Invalid version format".into()))?
        );

        Ok(version)
    }

    /// Obtiene una versión específica de un nodo
    pub async fn get_node_version(&self, id: NodeId, version: u64) -> Result<VersionedNode> {
        let version_key = format!("node:{}:v{}", id, version);

        let value = self.db.get(version_key.as_bytes())?
            .ok_or_else(|| NopalError::NodeNotFound(
                format!("{}:v{}", id, version)
            ))?;

        let versioned: VersionedNode = deserialize(&value)?;

        Ok(versioned)
    }

    /// Obtiene nodo en un timestamp específico (MVCC as_of)
    pub async fn get_node_at_timestamp(&self, id: NodeId, timestamp: u64) -> Result<VersionedNode> {
        // Obtener lista de versiones
        let versions_key = format!("node:{}:versions", id);

        let versions: Vec<u64> = match self.db.get(versions_key.as_bytes())? {
            Some(v) => deserialize(&v)?,
            None => {
                log::debug!("No versions found for node {}", id);
                return Err(NopalError::NodeNotFound(id.to_string()));
            }
        };

        log::debug!(
            "Searching version for node {} at t={}, available versions: {:?}",
            id, timestamp, versions
        );

        // Buscar versión válida en timestamp (más reciente primero)
        for &version in &versions {
            let versioned = self.get_node_version(id, version).await?;

            log::debug!(
                "  Checking v{}: valid_from={}, valid_to={:?}, is_valid={}",
                version,
                versioned.valid_from,
                versioned.valid_to,
                versioned.is_valid_at(timestamp)
            );

            if versioned.is_valid_at(timestamp) {
                log::debug!("  ✓ Found valid version: v{}", version);
                return Ok(versioned);
            }
        }

        Err(NopalError::Custom(format!(
            "No version of node {} valid at timestamp {}",
            id, timestamp
        )))
    }

    /// Obtiene historial completo de un nodo
    pub async fn get_node_history(&self, id: NodeId) -> Result<Vec<VersionedNode>> {
        let versions_key = format!("node:{}:versions", id);

        let versions: Vec<u64> = match self.db.get(versions_key.as_bytes())? {
            Some(v) => deserialize(&v)?,
            None => return Ok(Vec::new()),
        };

        let mut history = Vec::new();

        for &version in &versions {
            let versioned = self.get_node_version(id, version).await?;
            history.push(versioned);
        }

        Ok(history)
    }

    /// Invalida la versión actual de un nodo
    pub async fn invalidate_current_version(&self, id: NodeId, timestamp: u64) -> Result<()> {
        let current_version = self.get_current_version(id).await?;
        let mut versioned = self.get_node_version(id, current_version).await?;

        versioned.invalidate(timestamp);

        // Guardar versión invalidada
        let version_key = format!("node:{}:v{}", id, current_version);
        let version_value = serialize(&versioned)?;

        self.db.insert(version_key.as_bytes(), version_value)?;

        Ok(())
    }

    // ═══════════════════════════════════════════════════════════════════════
    // GARBAGE COLLECTION - Clean up old MVCC versions
    // ═══════════════════════════════════════════════════════════════════════

    /// Elimina versiones antiguas de nodos según la configuración de GC.
    ///
    /// # Arguments
    /// * `config` - Configuración del garbage collector
    ///
    /// # Returns
    /// Estadísticas de la operación de GC
    ///
    /// # Example
    /// ```ignore
    /// // Eliminar versiones más viejas de 7 días
    /// let config = GCConfig::older_than_days(7);
    /// let stats = storage.gc_old_versions(&config).await?;
    /// println!("Deleted {} versions", stats.versions_deleted);
    /// ```
    pub async fn gc_old_versions(&self, config: &crate::mvcc::GCConfig) -> Result<crate::mvcc::GCStats> {
        use crate::mvcc::GCStats;

        let start = std::time::Instant::now();
        let mut stats = GCStats::default();

        // 1. Encontrar todos los nodos con versiones
        
        let mut node_ids_with_versions: Vec<NodeId> = Vec::new();

        for item in self.db.scan_prefix(b"node:") {
            let (key, _) = item?;
            let key_str = String::from_utf8_lossy(&key);

            // Solo procesar keys de versiones (e.g., "node:uuid:versions")
            if key_str.ends_with(":versions") {
                let node_id_str = key_str
                    .strip_prefix("node:")
                    .and_then(|s| s.strip_suffix(":versions"));

                if let Some(id_str) = node_id_str
                    && let Ok(node_id) = uuid::Uuid::parse_str(id_str) {
                        node_ids_with_versions.push(node_id);
                }
            }
        }
        

        log::debug!("GC: Found {} nodes with versions", node_ids_with_versions.len());

        // 2. Aplicar límite de nodos por ciclo
        let nodes_to_process = if config.max_nodes_per_cycle > 0 {
            node_ids_with_versions.into_iter()
                .take(config.max_nodes_per_cycle)
                .collect::<Vec<_>>()
        } else {
            node_ids_with_versions
        };

        // 3. Para cada nodo, identificar y eliminar versiones elegibles
        for node_id in nodes_to_process {
            stats.nodes_scanned += 1;

            let history = self.get_node_history(node_id).await?;

            if history.len() <= config.min_versions_to_keep {
                // No hay suficientes versiones para eliminar
                continue;
            }

            // Identificar versiones a eliminar (mantener las más recientes)
            let versions_to_keep = config.min_versions_to_keep;
            let mut versions_to_delete: Vec<u64> = Vec::new();

            for (idx, versioned) in history.iter().enumerate() {
                // Siempre mantener las N versiones más recientes
                if idx < versions_to_keep {
                    continue;
                }

                // Verificar si es elegible para GC
                if versioned.is_gc_eligible(config.cutoff_timestamp) {
                    versions_to_delete.push(versioned.version);
                }
            }

            if versions_to_delete.is_empty() {
                continue;
            }

            log::debug!(
                "GC: Node {} - deleting {} versions: {:?}",
                node_id, versions_to_delete.len(), versions_to_delete
            );

            if !config.dry_run {
                // Eliminar las versiones
                let (deleted, bytes_freed) =
                    self.delete_node_versions(node_id, &versions_to_delete).await?;
                stats.versions_deleted += deleted;
                stats.bytes_freed += bytes_freed;
            } else {
                stats.versions_deleted += versions_to_delete.len();
            }
        }

        stats.duration_ms = start.elapsed().as_millis() as u64;

        log::info!(
            "GC complete: scanned {} nodes, deleted {} versions in {}ms{}",
            stats.nodes_scanned,
            stats.versions_deleted,
            stats.duration_ms,
            if config.dry_run { " (DRY RUN)" } else { "" }
        );

        Ok(stats)
    }

    /// Elimina versiones específicas de un nodo
    async fn delete_node_versions(&self, node_id: NodeId, versions: &[u64]) -> Result<(usize, usize)> {
        
        let mut deleted = 0;
        let mut bytes_freed = 0;

        // 1. Eliminar cada versión
        for &version in versions {
            let version_key = format!("node:{}:v{}", node_id, version);
            if let Some(value) = self.db.remove(version_key.as_bytes())? {
                deleted += 1;
                bytes_freed += value.len();
            }
        }

        // 2. Actualizar lista de versiones
        let versions_key = format!("node:{}:versions", node_id);
        if let Some(value) = self.db.get(versions_key.as_bytes())? {
            let mut version_list: Vec<u64> = deserialize(&value)?;

            version_list.retain(|v| !versions.contains(v));

            if version_list.is_empty() {
                self.db.remove(versions_key.as_bytes())?;
            } else {
                let new_value = serialize(&version_list)?;
                self.db.insert(versions_key.as_bytes(), new_value)?;
            }
        }

        Ok((deleted, bytes_freed))
    }

    /// Get all edges (for query executor)
    pub async fn get_all_edges(&self) -> Result<Vec<Edge>> {
        
        let edges_tree = self.db.open_tree("edges")?;
        let mut edges = Vec::new();

        for result in edges_tree.iter() {
            let (_, value) = result?;
            let edge: Edge = deserialize(&value)
                .map_err(|e| NopalError::SerializationError(format!("{}", e)))?;
            edges.push(edge);
        }

        Ok(edges)
    }
    /// Obtiene todos los nodos del storage (para export)
    pub async fn get_all_nodes(&self) -> Result<Vec<Node>> {
        let mut nodes = Vec::new();

        for item in self.db.scan_prefix(b"node:") {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);

            // Skip version and metadata keys
            if key_str.contains(":v")
                || key_str.contains(":current")
                || key_str.contains(":versions") {
                continue;
            }

            let node: Node = deserialize(&value)?;

            nodes.push(node);
        }

        log::debug!("Retrieved {} nodes for export", nodes.len());

        Ok(nodes)
    }

    /// Scan nodes in key order using a cursor and bounded batch size.
    ///
    /// This enables pull-based execution without materializing all nodes in memory.
    /// Returns `(nodes, next_cursor)` where `next_cursor` is the last scanned key.
    pub async fn scan_nodes_batch(
        &self,
        label: Option<&str>,
        start_after: Option<&str>,
        limit: usize,
    ) -> Result<(Vec<Node>, Option<String>)> {
        if limit == 0 {
            return Ok((Vec::new(), start_after.map(|s| s.to_string())));
        }

        let start = start_after
            .map(|s| s.as_bytes().to_vec())
            .unwrap_or_else(|| b"node:".to_vec());

        let mut nodes = Vec::with_capacity(limit);
        let mut last_seen_key: Option<String> = None;

        for item in self.db.range(start..) {
            let (key, value) = item?;

            if !key.starts_with(b"node:") {
                if last_seen_key.is_some() {
                    break;
                }
                continue;
            }

            let key_str = String::from_utf8_lossy(&key).to_string();

            if let Some(cursor) = start_after
                && key_str.as_str() <= cursor {
                    continue;
            }

            // Skip version and metadata keys
            if key_str.contains(":v")
                || key_str.contains(":current")
                || key_str.contains(":versions") {
                continue;
            }

            let node: Node = deserialize(&value)?;
            last_seen_key = Some(key_str);

            if let Some(expected_label) = label
                && node.label != expected_label {
                    continue;
            }

            nodes.push(node);
            if nodes.len() >= limit {
                break;
            }
        }

        let next_cursor = if nodes.len() >= limit {
            last_seen_key
        } else {
            None
        };

        Ok((nodes, next_cursor))
    }

    /// Obtiene todos los nodos versionados del storage (para MVCC export)
    pub async fn get_all_versioned_nodes(&self) -> Result<Vec<crate::mvcc::VersionedNode>> {
        let mut versioned_nodes = Vec::new();

        for item in self.db.scan_prefix(b"node:") {
            let (key, value) = item?;
            let key_str = String::from_utf8_lossy(&key);

            // Only process version keys (e.g., "node:uuid:v1")
            if key_str.contains(":v") && !key_str.contains(":versions") {
                let versioned: crate::mvcc::VersionedNode = deserialize(&value)?;

                versioned_nodes.push(versioned);
            }
        }

        log::debug!("Retrieved {} versioned nodes for export", versioned_nodes.len());

        Ok(versioned_nodes)
    }

    // ═══════════════════════════════════════════════════════════════════════
    // BATCH OPERATIONS - High Performance Bulk Insert
    // ═══════════════════════════════════════════════════════════════════════

    /// Inserta múltiples nodos en una sola operación atómica.
    ///
    /// **IMPORTANTE**: Esta es la forma recomendada para cargas masivas.
    /// Es 100-1000x más rápido que insertar nodos uno por uno.
    pub async fn insert_nodes_batch(&self, nodes: &[Node]) -> Result<Vec<NodeId>> {
        if nodes.is_empty() {
            return Ok(Vec::new());
        }

        let mut batch = sled::Batch::default();
        let mut ids = Vec::with_capacity(nodes.len());

        for node in nodes {
            let key = format!("node:{}", node.id);
            let value = serialize(node)?;
            batch.insert(key.as_bytes(), value);
            ids.push(node.id);
        }

        // Una sola operación de disco para todos los nodos
        self.db.apply_batch(batch)?;

        log::debug!("Batch inserted {} nodes", ids.len());
        Ok(ids)
    }

    /// Inserta múltiples aristas en una sola operación atómica.
    pub async fn insert_edges_batch(&self, edges: &[Edge]) -> Result<Vec<EdgeId>> {
        if edges.is_empty() {
            return Ok(Vec::new());
        }

        let edges_tree = self.db.open_tree("edges")?;

        let mut batch = sled::Batch::default();
        let mut ids = Vec::with_capacity(edges.len());

        //Revisar el tree de edges
        for edge in edges {
            let key = edge.id.to_string();
            let value = serialize(edge)?;
            batch.insert(key.as_bytes(), value);
            ids.push(edge.id);
        }

        edges_tree.apply_batch(batch)?;

        log::debug!("Batch inserted {} edges to edges tree", ids.len());
        Ok(ids)
    }

    /// Guarda múltiples índices de adyacencia en batch
    pub async fn save_adjacency_batch(
        &self,
        out_indices: &[(NodeId, Vec<EdgeId>)],
        in_indices: &[(NodeId, Vec<EdgeId>)],
    ) -> Result<()> {
        
        let mut batch = sled::Batch::default();

        for (node_id, edge_ids) in out_indices {
            let key = format!("idx:out:{}", node_id);
            let value = serialize(edge_ids)?;
            batch.insert(key.as_bytes(), value);
        }

        for (node_id, edge_ids) in in_indices {
            let key = format!("idx:in:{}", node_id);
            let value = serialize(edge_ids)?;
            batch.insert(key.as_bytes(), value);
        }

        self.db.apply_batch(batch)?;

        log::debug!(
            "Batch saved {} out indices and {} in indices",
            out_indices.len(),
            in_indices.len()
        );
        Ok(())
    }

    /// Flush all pending writes to disk
    ///
    /// Forces the underlying sled database to persist all buffered data.
    pub async fn flush(&self) -> Result<()> {
        
        self.db.flush_async()
            .await
            .map_err(|e| NopalError::custom(format!("Storage flush failed: {}", e)))?;
        Ok(())
    }
}

impl StorageBackend for Storage {
    fn backend_name(&self) -> &'static str {
        "sled"
    }

    fn profile(&self) -> StorageProfile {
        self.profile
    }

    fn verify_health(&self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "embeddings")]
    use crate::embeddings::Embedding;
    use crate::types::PropertyValue;
    use crate::mvcc::VersionedNode;

    #[tokio::test]
    async fn test_insert_and_get_node() {
        let storage = Storage::in_memory().await.unwrap();

        let node = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".to_string()))
            .with_property("age", PropertyValue::Int(30));

        storage.insert_node(&node).await.unwrap();

        let retrieved = storage.get_node(node.id).await.unwrap();

        assert_eq!(retrieved.id, node.id);
        assert_eq!(retrieved.label, "Person");
        assert_eq!(retrieved.properties.get("name"), Some(&PropertyValue::String("Alice".to_string())));
    }

    #[tokio::test]
    async fn test_storage_profile_mobile_on_in_memory() {
        let options = StorageOptions {
            engine: StorageEngine::Sled,
            profile: StorageProfile::Mobile,
        };
        let storage = Storage::in_memory_with_options(options).await.unwrap();
        assert_eq!(storage.backend_name(), "sled");
        assert_eq!(storage.profile(), StorageProfile::Mobile);
        assert_eq!(storage.tuning().cache_capacity_bytes, Some(16 * 1024 * 1024));
    }

    #[tokio::test]
    async fn test_delete_node() {
        let storage = Storage::in_memory().await.unwrap();

        let node = Node::new("Test");
        storage.insert_node(&node).await.unwrap();

        storage.delete_node(node.id).await.unwrap();

        let result = storage.get_node(node.id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_insert_and_get_edge() {
        let storage = Storage::in_memory().await.unwrap();

        let node1 = Node::new("Person")
            .with_property("name", PropertyValue::String("German".to_string()))
            .with_property("rol", PropertyValue::String("Assasin".to_string()));

        let node2 = Node::new("Person")
            .with_property("name", PropertyValue::String("Volga".to_string()))
            .with_property("rol", PropertyValue::String("Deidad".to_string()));

        storage.insert_node(&node1).await.unwrap();
        storage.insert_node(&node2).await.unwrap();

        let edge = Edge::new(node1.id, node2.id, "Enemy of".to_string())
            .with_property("damage", PropertyValue::Int(10));

        storage.insert_edge(&edge).await.unwrap();

        let retrieved = storage.get_edge(edge.id).await.unwrap();

        assert_eq!(retrieved.id, edge.id);
        assert_eq!(retrieved.edge_type, "Enemy of".to_string());
        assert_eq!(retrieved.properties.get("damage"), Some(&PropertyValue::Int(10)));
    }

    #[tokio::test]
    async fn test_save_and_load_adjacency() {
        let storage = Storage::in_memory().await.unwrap();

        let node_id = uuid::Uuid::new_v4();
        let edge_ids = vec![
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        ];

        // Guardar índices
        storage.save_adjacency_out(node_id, &edge_ids).await.unwrap();
        storage.save_adjacency_in(node_id, &edge_ids).await.unwrap();

        // Cargar índices
        let loaded_out = storage.load_adjacency_out(node_id).await.unwrap();
        let loaded_in = storage.load_adjacency_in(node_id).await.unwrap();

        assert_eq!(loaded_out, edge_ids);
        assert_eq!(loaded_in, edge_ids);
    }
    #[tokio::test]
    async fn test_mvcc_insert_and_get() {
        let storage = Storage::in_memory().await.unwrap();

        let node = Node::new("Person")
            .with_property("name", PropertyValue::String("Alice".into()))
            .with_property("age", PropertyValue::Int(25));

        let v1 = VersionedNode::new(node, 100);

        storage.insert_node_version(&v1).await.unwrap();

        // Get current version
        let current = storage.get_current_version(v1.id).await.unwrap();
        assert_eq!(current, 1);

        // Get specific version
        let retrieved = storage.get_node_version(v1.id, 1).await.unwrap();
        assert_eq!(retrieved.version, 1);
        assert_eq!(retrieved.timestamp, 100);
    }

    #[tokio::test]
    async fn test_mvcc_version_chain() {
        let storage = Storage::in_memory().await.unwrap();

        // Version 1
        let node1 = Node::new("Person")
            .with_property("age", PropertyValue::Int(25));
        let v1 = VersionedNode::new(node1, 100);
        storage.insert_node_version(&v1).await.unwrap();

        // Invalidate v1
        storage.invalidate_current_version(v1.id, 200).await.unwrap();

        // Version 2
        let node2 = Node::new("Person")
            .with_property("age", PropertyValue::Int(30));
        let v2 = VersionedNode::new_version(&v1, node2, 200);
        storage.insert_node_version(&v2).await.unwrap();

        // Get at different timestamps
        let at_150 = storage.get_node_at_timestamp(v1.id, 150).await.unwrap();
        assert_eq!(at_150.version, 1);

        let at_250 = storage.get_node_at_timestamp(v1.id, 250).await.unwrap();
        assert_eq!(at_250.version, 2);

        // Get history
        let history = storage.get_node_history(v1.id).await.unwrap();
        assert_eq!(history.len(), 2);
    }

    #[tokio::test]
    async fn test_mvcc_time_travel() {
        let storage = Storage::in_memory().await.unwrap();

        let node_id = uuid::Uuid::new_v4();

        // t=100: Create (age=25)
        let n1 = Node::with_id(node_id, "Person")
            .with_property("age", PropertyValue::Int(25));
        let v1 = VersionedNode::new(n1, 100);
        storage.insert_node_version(&v1).await.unwrap();

        // t=200: Update (age=30)
        storage.invalidate_current_version(node_id, 200).await.unwrap();
        let n2 = Node::with_id(node_id, "Person")
            .with_property("age", PropertyValue::Int(30));
        let v2 = VersionedNode::new_version(&v1, n2, 200);
        storage.insert_node_version(&v2).await.unwrap();

        // t=300: Update (age=35)
        storage.invalidate_current_version(node_id, 300).await.unwrap();
        let n3 = Node::with_id(node_id, "Person")
            .with_property("age", PropertyValue::Int(35));
        let v3 = VersionedNode::new_version(&v2, n3, 300);
        storage.insert_node_version(&v3).await.unwrap();

        // Time travel queries
        let at_150 = storage.get_node_at_timestamp(node_id, 150).await.unwrap();
        assert_eq!(
            at_150.node_data.properties.get("age"),
            Some(&PropertyValue::Int(25))
        );

        let at_250 = storage.get_node_at_timestamp(node_id, 250).await.unwrap();
        assert_eq!(
            at_250.node_data.properties.get("age"),
            Some(&PropertyValue::Int(30))
        );

        let at_350 = storage.get_node_at_timestamp(node_id, 350).await.unwrap();
        assert_eq!(
            at_350.node_data.properties.get("age"),
            Some(&PropertyValue::Int(35))
        );
    }

    // Note: The 4 contention tests (test_try_node_embedding_exists_sync_reports_busy_*,
    // test_load_*_sync_reports_busy_under_contention) were removed as part of the
    // P0 RwLock removal. Sled is thread-safe internally and no longer needs an
    // external RwLock, so contention-based "busy" errors no longer occur.
}
