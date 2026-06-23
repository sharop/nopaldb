// src/index/storage.rs
//
// Index persistence

use crate::error::Result;
use crate::index::IndexMetadata;
use std::path::Path;

/// Save index metadata to disk
pub fn save_metadata(path: &Path, metadata: &[IndexMetadata]) -> Result<()> {
    let data = bincode::serialize(metadata).map_err(|e| {
        crate::error::NopalError::serialization(format!("Failed to serialize metadata: {}", e))
    })?;

    std::fs::write(path, data).map_err(|e| {
        crate::error::NopalError::custom(format!("Failed to write metadata: {}", e))
    })?;

    Ok(())
}

/// Load index metadata from disk
pub fn load_metadata(path: &Path) -> Result<Vec<IndexMetadata>> {
    let data = std::fs::read(path)
        .map_err(|e| crate::error::NopalError::custom(format!("Failed to read metadata: {}", e)))?;

    let metadata = bincode::deserialize(&data).map_err(|e| {
        crate::error::NopalError::serialization(format!("Failed to deserialize metadata: {}", e))
    })?;

    Ok(metadata)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexType;

    #[test]
    fn test_save_load_metadata() {
        let temp_dir = tempfile::tempdir().unwrap();
        let path = temp_dir.path().join("metadata.bin");

        let metadata = vec![
            IndexMetadata {
                name: "Person_name".to_string(),
                label: "Person".to_string(),
                property: "name".to_string(),
                index_type: IndexType::Hash,
                created_at: chrono::Utc::now(),
                size: 1000,
            },
            IndexMetadata {
                name: "Person_age".to_string(),
                label: "Person".to_string(),
                property: "age".to_string(),
                index_type: IndexType::BTree,
                created_at: chrono::Utc::now(),
                size: 500,
            },
        ];

        // Save
        save_metadata(&path, &metadata).unwrap();

        // Load
        let loaded = load_metadata(&path).unwrap();

        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].name, "Person_name");
        assert_eq!(loaded[1].name, "Person_age");
    }
}
