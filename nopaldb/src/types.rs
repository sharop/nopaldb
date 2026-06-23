// src/types.rs

use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Identificador único para un nodo
pub type NodeId = uuid::Uuid;

/// Identificador único para una arista
pub type EdgeId = uuid::Uuid;

/// Valor de una propiedad
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum PropertyValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Object(Vec<(String, PropertyValue)>),
    List(Vec<PropertyValue>),
}

impl PropertyValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            PropertyValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            PropertyValue::Int(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            PropertyValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            PropertyValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[PropertyValue]> {
        match self {
            PropertyValue::List(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    pub fn as_object(&self) -> Option<&[(String, PropertyValue)]> {
        match self {
            PropertyValue::Object(fields) => Some(fields.as_slice()),
            _ => None,
        }
    }
}

impl PropertyValue {
    /// Rango de tipo para ordenamiento cross-variant.
    fn type_rank(&self) -> u8 {
        match self {
            PropertyValue::Null => 0,
            PropertyValue::Bool(_) => 1,
            PropertyValue::Int(_) => 2,
            PropertyValue::Float(_) => 3,
            PropertyValue::String(_) => 4,
            PropertyValue::Bytes(_) => 5,
            PropertyValue::List(_) => 6,
            PropertyValue::Object(_) => 7,
        }
    }
}

// Implementar Eq (requerido para Hash)
impl Eq for PropertyValue {}

// Implementar Hash (para HashMap)
impl Hash for PropertyValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Hash el discriminante primero
        std::mem::discriminant(self).hash(state);

        match self {
            PropertyValue::Null => {}
            PropertyValue::Bool(b) => b.hash(state),
            PropertyValue::Int(i) => i.hash(state),
            PropertyValue::Float(f) => {
                // Para floats, usamos bits para hash consistente
                f.to_bits().hash(state)
            }
            PropertyValue::String(s) => s.hash(state),
            PropertyValue::Bytes(b) => b.hash(state),
            PropertyValue::List(values) => values.hash(state),
            PropertyValue::Object(fields) => fields.hash(state),
        }
    }
}

// Implementar PartialOrd (para BTree)
impl PartialOrd for PropertyValue {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
// Implementar Ord (para BTree)
impl Ord for PropertyValue {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            // Null es menor que todo
            (PropertyValue::Null, PropertyValue::Null) => Ordering::Equal,
            (PropertyValue::Null, _) => Ordering::Less,
            (_, PropertyValue::Null) => Ordering::Greater,

            // Bool
            (PropertyValue::Bool(a), PropertyValue::Bool(b)) => a.cmp(b),

            // Int
            (PropertyValue::Int(a), PropertyValue::Int(b)) => a.cmp(b),

            // Float
            (PropertyValue::Float(a), PropertyValue::Float(b)) => {
                // Manejo especial para NaN
                if a.is_nan() && b.is_nan() {
                    Ordering::Equal
                } else if a.is_nan() {
                    Ordering::Greater
                } else if b.is_nan() {
                    Ordering::Less
                } else {
                    a.partial_cmp(b).unwrap_or(Ordering::Equal)
                }
            }

            // Coerción numérica Int↔Float — evita que type_rank ordene valores numéricos
            // cruzados incorrectamente (e.g. Float(500_000.0) > Int(900_000) sería true
            // por type_rank pero false por valor numérico).
            (PropertyValue::Int(a), PropertyValue::Float(b)) => {
                (*a as f64).partial_cmp(b).unwrap_or(Ordering::Equal)
            }
            (PropertyValue::Float(a), PropertyValue::Int(b)) => {
                a.partial_cmp(&(*b as f64)).unwrap_or(Ordering::Equal)
            }

            // String
            (PropertyValue::String(a), PropertyValue::String(b)) => a.cmp(b),

            // Bytes
            (PropertyValue::Bytes(a), PropertyValue::Bytes(b)) => a.cmp(b),

            // List
            (PropertyValue::List(a), PropertyValue::List(b)) => a.cmp(b),

            // Object
            (PropertyValue::Object(a), PropertyValue::Object(b)) => a.cmp(b),

            // Tipos diferentes: ordenar por tipo.
            _ => self.type_rank().cmp(&other.type_rank()),
        }
    }
}

/// Propiedades de un nodo como mapa clave-valor
pub type Properties = HashMap<String, PropertyValue>;

/// Clasificación ontológica de un nodo.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub enum NodeKind {
    /// Nodo de datos ordinario (instancia). Valor por defecto.
    #[default]
    Individual,
    /// Clase OWL / concepto en la taxonomía.
    Class,
    /// Propiedad de objeto OWL (arista entre individuos).
    ObjectProperty,
    /// Propiedad de dato OWL (arista a un valor literal).
    DataProperty,
    /// Restricción OWL (nodo anónimo generado por el razonador).
    Restriction,
    /// Propiedad de anotación OWL (metadatos como rdfs:comment).
    AnnotationProperty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: NodeId,
    pub label: String,
    pub properties: Properties,
    /// Clasificación ontológica del nodo.
    pub kind: NodeKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Edge {
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
    pub edge_type: String,
    pub properties: Properties,
}

impl Node {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            id: NodeId::new_v4(),
            label: label.into(),
            properties: HashMap::new(),
            kind: NodeKind::Individual,
        }
    }

    pub fn with_id(id: NodeId, label: impl Into<String>) -> Self {
        Self {
            id,
            label: label.into(),
            properties: HashMap::new(),
            kind: NodeKind::Individual,
        }
    }

    pub fn with_property(mut self, key: impl Into<String>, value: PropertyValue) -> Self {
        self.properties.insert(key.into(), value);
        self
    }

    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.properties = properties;
        self
    }
}

impl Edge {
    pub fn new(source: NodeId, target: NodeId, edge_type: impl Into<String>) -> Self {
        Self {
            id: EdgeId::new_v4(),
            source,
            target,
            edge_type: edge_type.into(),
            properties: HashMap::new(),
        }
    }

    pub fn with_id(
        id: EdgeId,
        source: NodeId,
        target: NodeId,
        edge_type: impl Into<String>,
    ) -> Self {
        Self {
            id,
            source,
            target,
            edge_type: edge_type.into(),
            properties: HashMap::new(),
        }
    }

    pub fn with_property(mut self, key: impl Into<String>, value: PropertyValue) -> Self {
        self.properties.insert(key.into(), value);
        self
    }

    pub fn with_properties(mut self, properties: Properties) -> Self {
        self.properties = properties;
        self
    }
}

/// Destino de una arista en un hipergrafo semántico (feature-gated).
///
/// Con feature desactivada, `Edge.target` sigue siendo `NodeId` (sin costo).
/// Cuando se active en el futuro, `Edge.target` migrará a `EdgeTarget`.
#[cfg(feature = "hypergraph")]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EdgeTarget {
    /// Arista binaria normal: un único nodo destino. Zero overhead semántico.
    Binary(NodeId),
    /// Hiperarista: N participantes (aridad arbitraria).
    Hyper(Vec<NodeId>),
    /// Arista n-aria con roles semánticos: `(rol, nodo)`.
    NaryRole(Vec<(String, NodeId)>),
}

#[cfg(feature = "hypergraph")]
impl From<NodeId> for EdgeTarget {
    fn from(id: NodeId) -> Self {
        EdgeTarget::Binary(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_property_value_ordering() {
        assert!(PropertyValue::Null < PropertyValue::Bool(false));
        assert!(PropertyValue::Bool(false) < PropertyValue::Bool(true));
        assert!(PropertyValue::Int(1) < PropertyValue::Int(2));
        assert!(PropertyValue::Float(1.5) < PropertyValue::Float(2.5));
        assert!(PropertyValue::String("a".to_string()) < PropertyValue::String("b".to_string()));
    }

    #[test]
    fn test_property_value_hash() {
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert(
            PropertyValue::String("key".to_string()),
            vec![uuid::Uuid::new_v4()],
        );

        assert!(map.contains_key(&PropertyValue::String("key".to_string())));
    }

    #[cfg(feature = "hypergraph")]
    mod hypergraph_tests {
        use super::*;

        #[test]
        fn test_edge_target_binary() {
            let id = NodeId::new_v4();
            let t = EdgeTarget::Binary(id);
            if let EdgeTarget::Binary(nid) = t {
                assert_eq!(nid, id);
            } else {
                panic!("Expected Binary");
            }
        }

        #[test]
        fn test_edge_target_from_node_id() {
            let id = NodeId::new_v4();
            let t: EdgeTarget = EdgeTarget::from(id);
            assert!(matches!(t, EdgeTarget::Binary(_)));
        }

        #[test]
        fn test_edge_target_hyper() {
            let ids: Vec<NodeId> = (0..3).map(|_| NodeId::new_v4()).collect();
            let t = EdgeTarget::Hyper(ids.clone());
            if let EdgeTarget::Hyper(v) = t {
                assert_eq!(v, ids);
            } else {
                panic!("Expected Hyper");
            }
        }

        #[test]
        fn test_edge_target_nary_role() {
            let id = NodeId::new_v4();
            let t = EdgeTarget::NaryRole(vec![("buyer".to_string(), id)]);
            if let EdgeTarget::NaryRole(roles) = t {
                assert_eq!(roles[0].0, "buyer");
                assert_eq!(roles[0].1, id);
            } else {
                panic!("Expected NaryRole");
            }
        }
    }
}
