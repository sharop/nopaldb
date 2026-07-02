// src/embeddings/persistence.rs
//
// Persistencia del HnswIndex a disco.
//
// Estrategia: guardamos el id_map (NodeId ↔ DataId) y metadata del índice
// en un archivo bincode. El grafo HNSW se reconstruye desde los embeddings
// almacenados en Sled al cargar.
//
// hnsw_rs tiene `file_dump`/`load_hnsw` nativos pero su API de lifetimes
// (`HnswIo` debe vivir tanto como el `Hnsw` cargado) complicaría el ownership
// dentro de `Graph`. Para un futuro con datasets >1M nodos donde el rebuild
// sea costoso, se puede migrar a persistencia nativa.

use crate::error::NopalError;
use crate::types::NodeId;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// Metadata persistida del HnswIndex.
#[derive(Serialize, Deserialize)]
struct HnswIndexMetadata {
    model: String,
    dimension: usize,
    id_map: HashMap<usize, NodeId>,
    reverse_map: HashMap<NodeId, usize>,
    next_data_id: usize,
}

/// Guarda la metadata del HnswIndex en disco.
///
/// El archivo se crea en `{data_dir}/hnsw_{model}.meta`.
/// El grafo HNSW no se persiste — se reconstruye desde embeddings en Sled.
pub fn save_index_metadata(
    index: &super::HnswIndex,
    data_dir: &Path,
) -> Result<(), NopalError> {
    let meta = HnswIndexMetadata {
        model: index.model().to_string(),
        dimension: index.dimension(),
        id_map: index.id_map().clone(),
        reverse_map: index.reverse_map().clone(),
        next_data_id: index.next_data_id(),
    };

    let filename = data_dir.join(format!("hnsw_{}.meta", index.model()));
    let bytes = bincode::serialize(&meta).map_err(|e| {
        NopalError::custom(format!("HnswPersistence::save: serialization error: {e}"))
    })?;

    std::fs::write(&filename, bytes).map_err(|e| {
        NopalError::IoError(e)
    })?;

    Ok(())
}

/// Carga metadata de un HnswIndex previamente guardado.
///
/// Retorna `None` si el archivo no existe.
pub fn load_index_metadata(
    model: &str,
    data_dir: &Path,
) -> Result<Option<HnswIndexMeta>, NopalError> {
    let filename = data_dir.join(format!("hnsw_{}.meta", model));
    if !filename.exists() {
        return Ok(None);
    }

    let bytes = std::fs::read(&filename).map_err(|e| {
        NopalError::IoError(e)
    })?;

    let meta: HnswIndexMetadata = bincode::deserialize(&bytes).map_err(|e| {
        NopalError::custom(format!("HnswPersistence::load: deserialization error: {e}"))
    })?;

    Ok(Some(HnswIndexMeta {
        model: meta.model,
        dimension: meta.dimension,
        point_count: meta.id_map.len(),
    }))
}

/// Verifica si existe metadata persistida para un modelo dado.
pub fn index_metadata_exists(model: &str, data_dir: &Path) -> bool {
    data_dir.join(format!("hnsw_{}.meta", model)).exists()
}

/// Elimina la metadata persistida de un modelo.
pub fn remove_index_metadata(model: &str, data_dir: &Path) -> Result<(), NopalError> {
    let filename = data_dir.join(format!("hnsw_{}.meta", model));
    if filename.exists() {
        std::fs::remove_file(&filename).map_err(|e| {
            NopalError::IoError(e)
        })?;
    }
    Ok(())
}

/// Información resumida de un HnswIndex persistido.
pub struct HnswIndexMeta {
    pub model: String,
    pub dimension: usize,
    pub point_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_save_and_load_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let vectors = vec![
            (Uuid::new_v4(), vec![1.0, 0.0, 0.0]),
            (Uuid::new_v4(), vec![0.0, 1.0, 0.0]),
        ];
        let index = super::super::HnswIndex::build_batch(vectors, "test-model", 3).unwrap();

        save_index_metadata(&index, dir.path()).unwrap();

        let meta = load_index_metadata("test-model", dir.path()).unwrap();
        assert!(meta.is_some());
        let meta = meta.unwrap();
        assert_eq!(meta.model, "test-model");
        assert_eq!(meta.dimension, 3);
        assert_eq!(meta.point_count, 2);
    }

    #[test]
    fn test_load_nonexistent_returns_none() {
        let dir = tempfile::tempdir().unwrap();
        let meta = load_index_metadata("nonexistent", dir.path()).unwrap();
        assert!(meta.is_none());
    }

    #[test]
    fn test_remove_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let vectors = vec![(Uuid::new_v4(), vec![1.0, 0.0])];
        let index = super::super::HnswIndex::build_batch(vectors, "rm-test", 2).unwrap();

        save_index_metadata(&index, dir.path()).unwrap();
        assert!(index_metadata_exists("rm-test", dir.path()));

        remove_index_metadata("rm-test", dir.path()).unwrap();
        assert!(!index_metadata_exists("rm-test", dir.path()));
    }
}
