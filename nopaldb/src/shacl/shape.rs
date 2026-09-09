// src/shacl/shape.rs
//! Definicion de NodeShape, PropertyShape, Target y ConstraintType.

use uuid::Uuid;
use crate::error::{NopalError, Result};
use crate::types::{NodeId, PropertyValue, NodeKind};
use super::report::Severity;

/// Una regex de `sh:pattern`, compilada una vez al construir la shape.
///
/// Antes se compilaba en cada evaluación y un patrón inválido salía como
/// `Warning` por cada valor; ahora un patrón inválido es error al construir
/// ([`ConstraintType::pattern`]) y la evaluación no compila nada.
#[derive(Debug, Clone)]
pub struct PatternConstraint {
    source: String,
    regex: regex::Regex,
}

impl PatternConstraint {
    /// Compila `source`; `Err` con el patrón y el motivo si no es una regex.
    pub fn new(source: impl Into<String>) -> Result<Self> {
        let source = source.into();
        let regex = regex::Regex::new(&source)
            .map_err(|e| NopalError::custom(format!("sh:pattern: patron regex invalido '{source}': {e}")))?;
        Ok(Self { source, regex })
    }

    /// El patrón tal como se escribió.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// `true` si `text` contiene una coincidencia.
    pub fn is_match(&self, text: &str) -> bool {
        self.regex.is_match(text)
    }
}

impl PartialEq for PatternConstraint {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

/// Target de un NodeShape — determina que nodos se validan.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// sh:targetNode — valida un nodo especifico por su ID.
    Node(NodeId),
    /// sh:targetNode escrito en Turtle: el nodo cuya propiedad `iri` es esta.
    /// Se resuelve al validar (índice de propiedades); si no existe, el
    /// reporte lo dice en vez de callar.
    NodeIri(String),
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

    /// El `DatatypeKind` que el importer Turtle produce para un datatype XSD:
    /// la misma tabla que `import_turtle` usa al guardar literales, para que
    /// `sh:datatype xsd:integer` pregunte por lo que el import escribió.
    /// Todo lo que el importer guarda como texto (xsd:string, xsd:date, lang
    /// strings, datatypes desconocidos) es `Str`.
    pub fn from_xsd(datatype_iri: &str) -> DatatypeKind {
        const XSD: &str = "http://www.w3.org/2001/XMLSchema#";
        let local = datatype_iri.strip_prefix(XSD).unwrap_or(datatype_iri);
        match local {
            "integer" | "int" | "long" | "short" | "byte" | "unsignedInt" | "unsignedLong" | "unsignedShort"
            | "unsignedByte" | "nonNegativeInteger" | "nonPositiveInteger" | "negativeInteger"
            | "positiveInteger" => DatatypeKind::Int,
            "decimal" | "double" | "float" => DatatypeKind::Float,
            "boolean" => DatatypeKind::Bool,
            "hexBinary" | "base64Binary" => DatatypeKind::Bytes,
            _ => DatatypeKind::Str,
        }
    }
}

/// Los `sh:nodeKind` de SHACL, sobre los valores de un path.
///
/// En NopalDB un valor es un nodo (destino de una arista) o un literal (una
/// propiedad); un nodo es blank cuando su `iri` empieza por `_:`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShaclNodeKind {
    Iri,
    BlankNode,
    Literal,
    BlankNodeOrIri,
    IriOrLiteral,
    BlankNodeOrLiteral,
}

impl ShaclNodeKind {
    /// Parsea el local name del término (`IRI`, `BlankNodeOrLiteral`, …).
    pub fn from_local_name(name: &str) -> Option<Self> {
        Some(match name {
            "IRI" => Self::Iri,
            "BlankNode" => Self::BlankNode,
            "Literal" => Self::Literal,
            "BlankNodeOrIRI" => Self::BlankNodeOrIri,
            "IRIOrLiteral" => Self::IriOrLiteral,
            "BlankNodeOrLiteral" => Self::BlankNodeOrLiteral,
            _ => return None,
        })
    }

    /// Nombre SHACL del kind, para mensajes.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Iri => "sh:IRI",
            Self::BlankNode => "sh:BlankNode",
            Self::Literal => "sh:Literal",
            Self::BlankNodeOrIri => "sh:BlankNodeOrIRI",
            Self::IriOrLiteral => "sh:IRIOrLiteral",
            Self::BlankNodeOrLiteral => "sh:BlankNodeOrLiteral",
        }
    }
}

/// Constraint individual sobre un nodo o propiedad.
///
/// Equivale a una restriccion SHACL Core sin SPARQL ni rutas complejas.
#[derive(Debug, Clone, PartialEq)]
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
    /// sh:pattern — la cadena debe coincidir con la expresion regular
    /// (compilada al construir: [`ConstraintType::pattern`]).
    Pattern(PatternConstraint),
    /// sh:in — el valor debe estar en la lista.
    In(Vec<PropertyValue>),
    /// sh:hasValue — la propiedad/path tiene exactamente este valor. Para un
    /// valor-nodo se compara con su `iri`.
    HasValue(PropertyValue),

    // --- Tipo ontologico ---
    /// `NodeKind` de NopalDB (Individual, Class, …) del nodo: la constraint
    /// programática de siempre. Sobre un literal siempre falla.
    NodeKindConstraint(NodeKind),
    /// sh:nodeKind de SHACL sobre los valores: nodo (IRI/blank) o literal.
    NodeKindShacl(ShaclNodeKind),
    /// sh:class — cada valor-nodo es instancia de la clase (por la taxonomía:
    /// directa o por herencia, y la clase se nombra por label, `prefix:Local`
    /// o IRI; sin taxonomía, por `label`). Un literal siempre falla.
    Class(String),

    // --- Lógicas: cada valor se valida contra shapes ---
    /// sh:and — cada valor conforma con TODAS las shapes.
    And(Vec<Shape>),
    /// sh:or — cada valor conforma con AL MENOS UNA shape; la violación lista
    /// por qué falló cada rama (`nested`).
    Or(Vec<Shape>),
    /// sh:not — ningún valor conforma con la shape.
    Not(Box<Shape>),
    /// sh:xone — cada valor conforma con EXACTAMENTE UNA shape.
    Xone(Vec<Shape>),
    /// sh:node — cada valor conforma con otra shape del mismo documento,
    /// referida por su IRI o su nombre. Una referencia que no resuelve es
    /// violación (nunca silencio).
    Node(String),
}

impl ConstraintType {
    /// `sh:pattern` con la regex compilada; `Err` si el patrón no es válido.
    pub fn pattern(source: impl Into<String>) -> Result<Self> {
        Ok(ConstraintType::Pattern(PatternConstraint::new(source)?))
    }

    /// Las shapes que este constraint aplica a cada valor (combinadores);
    /// vacío para las hojas. `sh:node` se resuelve aparte, por referencia.
    pub fn member_shapes(&self) -> Vec<&Shape> {
        match self {
            ConstraintType::And(v) | ConstraintType::Or(v) | ConstraintType::Xone(v) => v.iter().collect(),
            ConstraintType::Not(s) => vec![s.as_ref()],
            _ => vec![],
        }
    }
}

/// Especificacion de path en un PropertyShape (un solo salto).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PathSpec {
    /// Propiedad del nodo: `node.properties[key]`. Una `List` cuenta un
    /// valor por elemento.
    Property(String),
    /// Arista saliente con este edge_type; los valores son los nodos destino.
    Edge(String),
    /// Un predicado tal como lo escribe Turtle (`sh:path :usa`): NopalDB no
    /// sabe si aterrizó como propiedad o como arista, así que son los valores
    /// de la propiedad con ese nombre MÁS los destinos de las aristas con ese
    /// tipo. Es lo que produce `sh:path` al cargar shapes desde Turtle.
    Predicate(String),
}

impl PathSpec {
    /// Representacion como string para mensajes de error.
    pub fn as_str(&self) -> &str {
        match self {
            PathSpec::Property(s) | PathSpec::Edge(s) | PathSpec::Predicate(s) => s.as_str(),
        }
    }
}

/// Un valor resuelto por un path: un literal (propiedad) o un nodo (destino de
/// arista). Las constraints saben distinguirlos: `sh:class` pregunta por
/// nodos, `sh:datatype` por literales.
#[derive(Debug, Clone, PartialEq)]
pub enum PathValue {
    Literal(PropertyValue),
    Node(NodeId),
}

/// PropertyShape: path + lista de constraints sobre los valores resueltos.
#[derive(Debug, Clone, PartialEq)]
pub struct PropertyShape {
    pub path: PathSpec,
    pub constraints: Vec<ConstraintType>,
    /// `sh:severity` propio; si es `None` hereda la de la shape.
    pub severity: Option<Severity>,
    /// `sh:message` del autor: sustituye al mensaje generado en cada violación.
    pub message: Option<String>,
}

/// NodeShape: targets + constraints directas + property shapes anidadas.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    /// ID unico del shape (UUID generado al crear o asignado desde el grafo).
    pub id: NodeId,
    /// Nombre legible del shape (para mensajes): `sh:name`, o el local name
    /// de su IRI cuando viene de Turtle.
    pub name: String,
    /// IRI del shape cuando se cargó desde Turtle (`:RecetaShape`); `None`
    /// para shapes programáticas.
    pub iri: Option<String>,
    /// Nodos a los que aplica este shape.
    pub targets: Vec<Target>,
    /// Constraints directas sobre el nodo (sh:nodeKind, sh:class, etc.).
    pub constraints: Vec<ConstraintType>,
    /// Property shapes anidadas (sh:property).
    pub property_shapes: Vec<PropertyShape>,
    /// `sh:severity` de las violaciones de esta shape (default `Violation`).
    pub severity: Severity,
    /// `sh:message` del autor para las violaciones de esta shape.
    pub message: Option<String>,
    /// `sh:deactivated true`: la shape se carga pero no valida nada.
    pub deactivated: bool,
}

impl Shape {
    /// Crea un nuevo NodeShape con ID generado.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            iri: None,
            targets: vec![],
            constraints: vec![],
            property_shapes: vec![],
            severity: Severity::Violation,
            message: None,
            deactivated: false,
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
        Self { path, constraints, severity: None, message: None }
    }
}
