// src/index/btree.rs
//
// B-Tree index for range queries (O(log N))

use crate::error::Result;
use crate::index::{Index, IndexQuery};
use crate::types::{NodeId, PropertyValue};
use std::collections::BTreeMap;

/// B-Tree index - O(log N) range queries
pub struct BTreeIndex {
    /// Ordered map from property value to list of node IDs
    map: BTreeMap<PropertyValue, Vec<NodeId>>,
}

impl BTreeIndex {
    /// Create new B-Tree index
    pub fn new() -> Self {
        BTreeIndex {
            map: BTreeMap::new(),
        }
    }

    /// Get all node IDs for a value
    pub fn get(&self, value: &PropertyValue) -> Option<&Vec<NodeId>> {
        self.map.get(value)
    }

    /// Range query helper
    fn range_query<F>(&self, predicate: F) -> Vec<NodeId>
    where
        F: Fn(&PropertyValue) -> bool,
    {
        self.map
            .iter()
            .filter(|(k, _)| predicate(k))
            .flat_map(|(_, nodes)| nodes.iter().copied())
            .collect()
    }
}

impl Index for BTreeIndex {
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

            IndexQuery::GreaterThan(value) => Ok(self.range_query(|k| k > value)),

            IndexQuery::GreaterThanOrEqual(value) => Ok(self.range_query(|k| k >= value)),

            IndexQuery::LessThan(value) => Ok(self.range_query(|k| k < value)),

            IndexQuery::LessThanOrEqual(value) => Ok(self.range_query(|k| k <= value)),

            IndexQuery::Between(min, max) => Ok(self.range_query(|k| k >= min && k <= max)),

            IndexQuery::FullText(_) => Err(crate::error::NopalError::index_error(
                "BTree index does not support full-text search".to_string(),
            )),
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

impl Default for BTreeIndex {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_btree_index_insert_query() {
        let mut index = BTreeIndex::new();

        let node1 = uuid::Uuid::new_v4();
        let node2 = uuid::Uuid::new_v4();
        let node3 = uuid::Uuid::new_v4();

        // Insert integers
        index.insert(PropertyValue::Int(10), node1).unwrap();
        index.insert(PropertyValue::Int(20), node2).unwrap();
        index.insert(PropertyValue::Int(30), node3).unwrap();

        // Equality
        let result = index
            .query(&IndexQuery::Equals(PropertyValue::Int(20)))
            .unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], node2);

        // Greater than
        let result = index
            .query(&IndexQuery::GreaterThan(PropertyValue::Int(15)))
            .unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&node2));
        assert!(result.contains(&node3));

        // Less than
        let result = index
            .query(&IndexQuery::LessThan(PropertyValue::Int(25)))
            .unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&node1));
        assert!(result.contains(&node2));
    }

    #[test]
    fn test_btree_index_range_queries() {
        let mut index = BTreeIndex::new();

        let nodes: Vec<NodeId> = (0..10).map(|_| uuid::Uuid::new_v4()).collect();

        // Insert 0, 10, 20, ..., 90
        for (i, &node) in nodes.iter().enumerate() {
            index
                .insert(PropertyValue::Int((i * 10) as i64), node)
                .unwrap();
        }

        // Between 20 and 50
        let result = index
            .query(&IndexQuery::Between(
                PropertyValue::Int(20),
                PropertyValue::Int(50),
            ))
            .unwrap();
        assert_eq!(result.len(), 4); // 20, 30, 40, 50

        // Greater than or equal 70
        let result = index
            .query(&IndexQuery::GreaterThanOrEqual(PropertyValue::Int(70)))
            .unwrap();
        assert_eq!(result.len(), 3); // 70, 80, 90

        // Less than or equal 30
        let result = index
            .query(&IndexQuery::LessThanOrEqual(PropertyValue::Int(30)))
            .unwrap();
        assert_eq!(result.len(), 4); // 0, 10, 20, 30
    }

    #[test]
    fn test_btree_index_strings() {
        let mut index = BTreeIndex::new();

        let node_alice = uuid::Uuid::new_v4();
        let node_bob = uuid::Uuid::new_v4();
        let node_charlie = uuid::Uuid::new_v4();

        index
            .insert(PropertyValue::String("Alice".to_string()), node_alice)
            .unwrap();
        index
            .insert(PropertyValue::String("Bob".to_string()), node_bob)
            .unwrap();
        index
            .insert(PropertyValue::String("Charlie".to_string()), node_charlie)
            .unwrap();

        // Lexicographic ordering
        let result = index
            .query(&IndexQuery::GreaterThan(PropertyValue::String(
                "B".to_string(),
            )))
            .unwrap();
        assert_eq!(result.len(), 2); // Bob, Charlie

        let result = index
            .query(&IndexQuery::LessThan(PropertyValue::String(
                "C".to_string(),
            )))
            .unwrap();
        assert_eq!(result.len(), 2); // Alice, Bob
    }

    #[test]
    fn test_btree_index_floats() {
        let mut index = BTreeIndex::new();

        let node1 = uuid::Uuid::new_v4();
        let node2 = uuid::Uuid::new_v4();
        let node3 = uuid::Uuid::new_v4();

        index.insert(PropertyValue::Float(1.5), node1).unwrap();
        index.insert(PropertyValue::Float(2.7), node2).unwrap();
        index.insert(PropertyValue::Float(3.9), node3).unwrap();

        let result = index
            .query(&IndexQuery::Between(
                PropertyValue::Float(2.0),
                PropertyValue::Float(4.0),
            ))
            .unwrap();
        assert_eq!(result.len(), 2); // 2.7, 3.9
    }
}
