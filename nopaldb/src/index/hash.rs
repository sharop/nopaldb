// src/index/hash.rs
//
// Hash-based index for O(1) equality lookups

use crate::error::Result;
use crate::index::{Index, IndexQuery};
use crate::types::{NodeId, PropertyValue};
use std::collections::HashMap;

/// Hash index - O(1) equality lookups
pub struct HashIndex {
    /// Map from property value to list of node IDs
    map: HashMap<PropertyValue, Vec<NodeId>>,
}

impl HashIndex {
    /// Create new hash index
    pub fn new() -> Self {
        HashIndex {
            map: HashMap::new(),
        }
    }

    /// Get all node IDs for a value
    pub fn get(&self, value: &PropertyValue) -> Option<&Vec<NodeId>> {
        self.map.get(value)
    }
}

impl Index for HashIndex {
    fn insert(&mut self, value: PropertyValue, node_id: NodeId) -> Result<()> {
        self.map.entry(value).or_default().push(node_id);
        Ok(())
    }

    fn remove(&mut self, value: &PropertyValue, node_id: NodeId) -> Result<()> {
        if let Some(nodes) = self.map.get_mut(value) {
            nodes.retain(|&id| id != node_id);

            // Remove entry if empty
            if nodes.is_empty() {
                self.map.remove(value);
            }
        }
        Ok(())
    }

    fn query(&self, query: &IndexQuery) -> Result<Vec<NodeId>> {
        match query {
            IndexQuery::Equals(value) => Ok(self.map.get(value).cloned().unwrap_or_default()),
            _ => {
                // Hash index only supports equality
                Err(crate::error::NopalError::index_error(
                    "Hash index only supports equality queries".to_string(),
                ))
            }
        }
    }

    fn clear(&mut self) -> Result<()> {
        self.map.clear();
        Ok(())
    }

    fn size(&self) -> usize {
        self.map.len()
    }
}

impl Default for HashIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_index_insert_query() {
        let mut index = HashIndex::new();

        let node1 = uuid::Uuid::new_v4();
        let node2 = uuid::Uuid::new_v4();
        let node3 = uuid::Uuid::new_v4();

        // Insert
        index
            .insert(PropertyValue::String("Alice".to_string()), node1)
            .unwrap();
        index
            .insert(PropertyValue::String("Bob".to_string()), node2)
            .unwrap();
        index
            .insert(PropertyValue::String("Alice".to_string()), node3)
            .unwrap();

        // Query
        let result = index
            .query(&IndexQuery::Equals(PropertyValue::String(
                "Alice".to_string(),
            )))
            .unwrap();

        assert_eq!(result.len(), 2);
        assert!(result.contains(&node1));
        assert!(result.contains(&node3));

        // Query non-existent
        let result = index
            .query(&IndexQuery::Equals(PropertyValue::String(
                "Charlie".to_string(),
            )))
            .unwrap();
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_hash_index_remove() {
        let mut index = HashIndex::new();

        let node1 = uuid::Uuid::new_v4();
        let node2 = uuid::Uuid::new_v4();

        let value = PropertyValue::String("Alice".to_string());

        index.insert(value.clone(), node1).unwrap();
        index.insert(value.clone(), node2).unwrap();

        // Remove one
        index.remove(&value, node1).unwrap();

        let result = index.query(&IndexQuery::Equals(value.clone())).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], node2);

        // Remove last
        index.remove(&value, node2).unwrap();

        let result = index.query(&IndexQuery::Equals(value)).unwrap();
        assert_eq!(result.len(), 0);
        assert_eq!(index.size(), 0);
    }

    #[test]
    fn test_hash_index_multiple_types() {
        let mut index = HashIndex::new();

        let node1 = uuid::Uuid::new_v4();
        let node2 = uuid::Uuid::new_v4();
        let node3 = uuid::Uuid::new_v4();

        // Different types
        index.insert(PropertyValue::Int(42), node1).unwrap();
        index
            .insert(PropertyValue::String("42".to_string()), node2)
            .unwrap();
        index.insert(PropertyValue::Float(42.0), node3).unwrap();

        // Each type is distinct
        assert_eq!(
            index
                .query(&IndexQuery::Equals(PropertyValue::Int(42)))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            index
                .query(&IndexQuery::Equals(PropertyValue::String("42".to_string())))
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            index
                .query(&IndexQuery::Equals(PropertyValue::Float(42.0)))
                .unwrap()
                .len(),
            1
        );
    }
}
