// src/index/btree.rs
//
// B-Tree index for range queries (O(log N))

use crate::error::Result;
use crate::types::{NodeId, PropertyValue};
use crate::index::{Index, IndexQuery};
use std::collections::BTreeMap;
use std::ops::Bound;

/// B-Tree index - O(log N + k) range queries
///
/// ⚠️ Hazard conocido con claves numéricas heterogéneas: el `Ord` de
/// `PropertyValue` coerciona `Int`↔`Float` (`Int(1).cmp(&Float(1.0)) ==
/// Equal`) pero su `PartialEq` derivado NO (`Int(1) != Float(1.0)`), y
/// `BTreeMap` exige `Ord` consistente con `Eq`. Consecuencia: si una misma
/// propiedad mezcla `Int(1)` y `Float(1.0)`, ambas caen en el bucket del
/// primero que se insertó (comportamiento pinneado en
/// `test_heterogeneous_numeric_keys_merge_pinned`). El fix real —
/// normalización canónica de claves numéricas al insertar — es un cambio
/// de comportamiento registrado como ítem aparte en el roadmap; mientras
/// tanto, no mezclar tipos numéricos en una propiedad indexada con BTree.
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

    /// Range seek real sobre el BTreeMap: O(log N + k) en vez del filter
    /// O(N) anterior (que descartaba la ventaja del árbol). Mismo `Ord`,
    /// misma semántica de resultados.
    fn range_query(
        &self,
        start: Bound<&PropertyValue>,
        end: Bound<&PropertyValue>,
    ) -> Vec<NodeId> {
        self.map
            .range((start, end))
            .flat_map(|(_, nodes)| nodes.iter().copied())
            .collect()
    }
}

impl Index for BTreeIndex {
    fn insert(&mut self, value: PropertyValue, node_id: NodeId) -> Result<()> {
        self.map
            .entry(value)
            .or_default()
            .push(node_id);
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
            IndexQuery::Equals(value) => {
                Ok(self.map.get(value).cloned().unwrap_or_default())
            }

            IndexQuery::GreaterThan(value) => {
                Ok(self.range_query(Bound::Excluded(value), Bound::Unbounded))
            }

            IndexQuery::GreaterThanOrEqual(value) => {
                Ok(self.range_query(Bound::Included(value), Bound::Unbounded))
            }

            IndexQuery::LessThan(value) => {
                Ok(self.range_query(Bound::Unbounded, Bound::Excluded(value)))
            }

            IndexQuery::LessThanOrEqual(value) => {
                Ok(self.range_query(Bound::Unbounded, Bound::Included(value)))
            }

            IndexQuery::Between(min, max) => {
                // Inclusivo en ambos extremos (semántica de siempre). Guardia:
                // BTreeMap::range panica con start > end; el filter anterior
                // regresaba vacío — conservamos ese contrato.
                if min > max {
                    return Ok(Vec::new());
                }
                Ok(self.range_query(Bound::Included(min), Bound::Included(max)))
            }

            IndexQuery::FullText(_) => {
                Err(crate::error::NopalError::index_error(
                    "BTree index does not support full-text search".to_string()
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
        let result = index.query(&IndexQuery::Equals(PropertyValue::Int(20))).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], node2);

        // Greater than
        let result = index.query(&IndexQuery::GreaterThan(PropertyValue::Int(15))).unwrap();
        assert_eq!(result.len(), 2);
        assert!(result.contains(&node2));
        assert!(result.contains(&node3));

        // Less than
        let result = index.query(&IndexQuery::LessThan(PropertyValue::Int(25))).unwrap();
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
            index.insert(PropertyValue::Int((i * 10) as i64), node).unwrap();
        }

        // Between 20 and 50
        let result = index.query(&IndexQuery::Between(
            PropertyValue::Int(20),
            PropertyValue::Int(50),
        )).unwrap();
        assert_eq!(result.len(), 4); // 20, 30, 40, 50

        // Greater than or equal 70
        let result = index.query(&IndexQuery::GreaterThanOrEqual(
            PropertyValue::Int(70)
        )).unwrap();
        assert_eq!(result.len(), 3); // 70, 80, 90

        // Less than or equal 30
        let result = index.query(&IndexQuery::LessThanOrEqual(
            PropertyValue::Int(30)
        )).unwrap();
        assert_eq!(result.len(), 4); // 0, 10, 20, 30
    }

    #[test]
    fn test_btree_index_strings() {
        let mut index = BTreeIndex::new();

        let node_alice = uuid::Uuid::new_v4();
        let node_bob = uuid::Uuid::new_v4();
        let node_charlie = uuid::Uuid::new_v4();

        index.insert(PropertyValue::String("Alice".to_string()), node_alice).unwrap();
        index.insert(PropertyValue::String("Bob".to_string()), node_bob).unwrap();
        index.insert(PropertyValue::String("Charlie".to_string()), node_charlie).unwrap();

        // Lexicographic ordering
        let result = index.query(&IndexQuery::GreaterThan(
            PropertyValue::String("B".to_string())
        )).unwrap();
        assert_eq!(result.len(), 2); // Bob, Charlie

        let result = index.query(&IndexQuery::LessThan(
            PropertyValue::String("C".to_string())
        )).unwrap();
        assert_eq!(result.len(), 2); // Alice, Bob
    }

    #[test]
    fn test_btree_between_inverted_returns_empty() {
        // Contrato conservado del filter anterior: min > max → vacío
        // (BTreeMap::range panicaría sin la guardia).
        let mut index = BTreeIndex::new();
        index.insert(PropertyValue::Int(5), uuid::Uuid::new_v4()).unwrap();
        let result = index
            .query(&IndexQuery::Between(PropertyValue::Int(10), PropertyValue::Int(1)))
            .unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_heterogeneous_numeric_keys_merge_pinned() {
        // Pin del hazard documentado en el struct: Ord coerciona Int↔Float
        // pero Eq no; BTreeMap fusiona Int(1) y Float(1.0) en el bucket del
        // primero insertado. Si este test cambia, el fix de normalización
        // canónica llegó — actualizar el doc del struct y el roadmap.
        let mut index = BTreeIndex::new();
        let a = uuid::Uuid::new_v4();
        let b = uuid::Uuid::new_v4();
        index.insert(PropertyValue::Int(1), a).unwrap();
        index.insert(PropertyValue::Float(1.0), b).unwrap();

        // Un solo bucket (bajo la clave Int(1) que llegó primero)…
        assert_eq!(index.size(), 1);
        // …y la búsqueda por CUALQUIERA de las dos formas regresa ambos.
        let by_float = index.query(&IndexQuery::Equals(PropertyValue::Float(1.0))).unwrap();
        assert_eq!(by_float.len(), 2);
        assert!(by_float.contains(&a) && by_float.contains(&b));
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

        let result = index.query(&IndexQuery::Between(
            PropertyValue::Float(2.0),
            PropertyValue::Float(4.0),
        )).unwrap();
        assert_eq!(result.len(), 2); // 2.7, 3.9
    }
}