// src/shacl/shape.rs
//! Definicion de NodeShape, PropertyShape, Target y ConstraintType.

use crate::types::{NodeId, NodeKind, PropertyValue};
use uuid::Uuid;

/// Target de un NodeShape — determina que nodos se validan.
#[derive(Debug, Clone)]
pub enum Target {
    /// sh:targetNode — valida un nodo especifico por su ID.
    Node(NodeId),
    /// sh:targetClass — valida todos los nodos cuyo `label` coincide.
    Class(String),
}

/// Tipo de dato esperado en un constraint sh:datatype.
#[derive(Debug, Clone, PartialEq)]
pub enum DatatypeKind {
    Int,
    Float,
    Str,
    Bool,
    Bytes,
    Null,
}

impl DatatypeKind {
    /// Verifica si un `PropertyValue` satisface este datatype.
    pub fn matches(&self, value: &PropertyValue) -> bool {
        matches!(
            (self, value),
            (DatatypeKind::Int, PropertyValue::Int(_))
                | (DatatypeKind::Float, PropertyValue::Float(_))
                | (DatatypeKind::Str, PropertyValue::String(_))
                | (DatatypeKind::Bool, PropertyValue::Bool(_))
                | (DatatypeKind::Bytes, PropertyValue::Bytes(_))
                | (DatatypeKind::Null, PropertyValue::Null)
        )
    }
}

/// Constraint individual sobre un nodo o propiedad.
///
/// Equivale a una restriccion SHACL Core sin SPARQL ni rutas complejas.
#[derive(Debug, Clone)]
pub enum ConstraintType {
    // --- Cardinalidad ---
    /// sh:minCount — la propiedad/path debe tener al menos N valores.
    MinCount(usize),
    /// sh:maxCount — la propiedad/path debe tener como maximo N valores.
    MaxCount(usize),

    // --- Tipo de dato ---
    /// sh:datatype — todos los valores deben ser del tipo indicado.
    Datatype(DatatypeKind),

    // --- Rangos numericos ---
    /// sh:minInclusive — valor >= limite.
    MinInclusive(f64),
    /// sh:maxInclusive — valor <= limite.
    MaxInclusive(f64),
    /// sh:minExclusive — valor > limite.
    MinExclusive(f64),
    /// sh:maxExclusive — valor < limite.
    MaxExclusive(f64),

    // --- Longitud de strings ---
    /// sh:minLength — longitud de la cadena >= N.
    MinLength(usize),
    /// sh:maxLength — longitud de la cadena <= N.
    MaxLength(usize),

    // --- Patron y enumeracion ---
    /// sh:pattern — la cadena debe coincidir con la expresion regular.
    Pattern(String),
    /// sh:in — el valor debe estar en la lista.
    In(Vec<PropertyValue>),
    /// sh:hasValue — la propiedad/path tiene exactamente este valor.
    HasValue(PropertyValue),

    // --- Tipo ontologico ---
    /// sh:nodeKind — el nodo debe tener el NodeKind indicado.
    NodeKindConstraint(NodeKind),
    /// sh:class — el nodo objetivo debe tener el label indicado.
    Class(String),
}

/// Especificacion de path en un PropertyShape — solo single-hop en v1.
#[derive(Debug, Clone)]
pub enum PathSpec {
    /// Propiedad del nodo: `node.properties[key]`.
    Property(String),
    /// Arista saliente con este edge_type (single-hop).
    Edge(String),
}

impl PathSpec {
    /// Representacion como string para mensajes de error.
    pub fn as_str(&self) -> &str {
        match self {
            PathSpec::Property(s) | PathSpec::Edge(s) => s.as_str(),
        }
    }
}

/// PropertyShape: path + lista de constraints sobre los valores resueltos.
#[derive(Debug, Clone)]
pub struct PropertyShape {
    pub path: PathSpec,
    pub constraints: Vec<ConstraintType>,
}

/// NodeShape: targets + constraints directas + property shapes anidadas.
#[derive(Debug, Clone)]
pub struct Shape {
    /// ID unico del shape (UUID generado al crear o asignado desde el grafo).
    pub id: NodeId,
    /// Nombre legible del shape (para mensajes).
    pub name: String,
    /// Nodos a los que aplica este shape.
    pub targets: Vec<Target>,
    /// Constraints directas sobre el nodo (sh:nodeKind, sh:class, etc.).
    pub constraints: Vec<ConstraintType>,
    /// Property shapes anidadas (sh:property).
    pub property_shapes: Vec<PropertyShape>,
}

impl Shape {
    /// Crea un nuevo NodeShape con ID generado.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            targets: vec![],
            constraints: vec![],
            property_shapes: vec![],
        }
    }

    /// Agrega un target al shape.
    pub fn with_target(mut self, target: Target) -> Self {
        self.targets.push(target);
        self
    }

    /// Agrega un constraint directo al shape.
    pub fn with_constraint(mut self, constraint: ConstraintType) -> Self {
        self.constraints.push(constraint);
        self
    }

    /// Agrega un PropertyShape al shape.
    pub fn with_property_shape(mut self, ps: PropertyShape) -> Self {
        self.property_shapes.push(ps);
        self
    }
}

impl PropertyShape {
    /// Crea un PropertyShape con path y constraints.
    pub fn new(path: PathSpec, constraints: Vec<ConstraintType>) -> Self {
        Self { path, constraints }
    }
}
