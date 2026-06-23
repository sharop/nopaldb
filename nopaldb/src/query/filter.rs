// src/query/filter.rs

use crate::types::{Node, PropertyValue};
use std::sync::Arc;

/// Predicado para filtrar nodos
pub type NodePredicate = Arc<dyn Fn(&Node) -> bool + Send + Sync>;

/// Builder para crear filtros comunes
pub struct FilterBuilder;

impl FilterBuilder {
    /// Filtrar por label del nodo
    pub fn label(label: impl Into<String>) -> NodePredicate {
        let label = label.into();
        Arc::new(move |node: &Node| node.label == label)
    }

    /// Filtrar por existencia de propiedad
    pub fn has_property(key: impl Into<String>) -> NodePredicate {
        let key = key.into();
        Arc::new(move |node: &Node| node.properties.contains_key(&key))
    }

    /// Filtrar por valor de propiedad string
    pub fn property_eq(key: impl Into<String>, value: impl Into<String>) -> NodePredicate {
        let key = key.into();
        let value = PropertyValue::String(value.into());
        Arc::new(move |node: &Node| node.properties.get(&key) == Some(&value))
    }

    /// Filtrar por propiedad int mayor que
    pub fn property_gt(key: impl Into<String>, threshold: i64) -> NodePredicate {
        let key = key.into();
        Arc::new(move |node: &Node| {
            if let Some(PropertyValue::Int(val)) = node.properties.get(&key) {
                *val > threshold
            } else {
                false
            }
        })
    }

    /// Combinar dos predicados con AND
    pub fn and(p1: NodePredicate, p2: NodePredicate) -> NodePredicate {
        Arc::new(move |node: &Node| p1(node) && p2(node))
    }

    /// Combinar dos predicados con OR
    pub fn or(p1: NodePredicate, p2: NodePredicate) -> NodePredicate {
        Arc::new(move |node: &Node| p1(node) || p2(node))
    }

    /// Negar un predicado
    pub fn not(p: NodePredicate) -> NodePredicate {
        Arc::new(move |node: &Node| !p(node))
    }
}
