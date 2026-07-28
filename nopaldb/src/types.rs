// src/types.rs

use std::collections::HashMap;
use std::cmp::Ordering;
use std::hash::{Hash, Hasher};
use serde::{Serialize, Deserialize};

/// Identificador único para un nodo
pub type NodeId = uuid::Uuid;

/// Identificador único para una arista
pub type EdgeId = uuid::Uuid;

/// Valor de una propiedad — la lingua franca de valores del motor.
///
/// Todo valor que entra al grafo, se evalúa en NQL, se indexa o cruza una
/// frontera (Python, MCP, Arrow) pasa por este enum.
///
/// # Conversión
///
/// Hay `From` para los primitivos comunes, así que los literales funcionan
/// directo en [`Node::with_property`]/[`Edge::with_property`]:
///
/// ```
/// use nopaldb::{Node, PropertyValue};
/// let n = Node::new("Person")
///     .with_property("name", "Alice")   // &str  → String
///     .with_property("age", 30)         // i32   → Int
///     .with_property("score", 2.5)      // f64   → Float
///     .with_property("active", true);   // bool  → Bool
/// assert_eq!(n.properties["age"], PropertyValue::Int(30));
/// ```
///
/// Para extraer: `as_str`/`as_i64`/`as_f64`/`as_bool`/`as_list`/`as_object`,
/// [`PropertyValue::as_number`] (coerciona `Int`→`f64`) y
/// [`PropertyValue::to_display_string`] (render humano; también `Display`).
/// Hay puente con `serde_json::Value` en ambas direcciones (`From`).
///
/// **`From` deliberadamente excluidos**: `u64`/`usize`/`i128` (pueden
/// desbordar `i64`; el caller debe convertir explícitamente y decidir el
/// truncamiento) y un `From<Vec<T>>` genérico (colisionaría con
/// `Vec<u8>`→`Bytes` el día que existiera `From<u8>`).
///
/// # Restricciones de evolución
///
/// - **Este enum NO es `#[non_exhaustive]`** y hay `match` exhaustivos sobre
///   él en decenas de sitios internos más los consumidores publicados en
///   crates.io/PyPI: **agregar una variante es un cambio semver-major**.
///   Opciones registradas para cuando se necesite: marcar `#[non_exhaustive]`
///   (rompe una sola vez, peaje por adelantado) o una variante envoltorio
///   `Extended(...)`.
/// - **`#[serde(untagged)]` hace que el ORDEN de declaración importe**: la
///   deserialización prueba variantes en orden. `Int` antes que `Float`
///   (los enteros JSON parsean como `Int`) y `Bytes` antes que `List`
///   (un arreglo JSON de enteros pequeños deserializa como `Bytes`, no
///   `List` — quirk existente, pinneado en tests). Reordenar variantes es
///   un cambio silencioso de formato de datos.
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

    /// Valor numérico como f64, coercionando `Int` (a diferencia de `as_f64`).
    pub fn as_number(&self) -> Option<f64> {
        match self {
            PropertyValue::Float(f) => Some(*f),
            PropertyValue::Int(i) => Some(*i as f64),
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
            PropertyValue::Null      => 0,
            PropertyValue::Bool(_)   => 1,
            PropertyValue::Int(_)    => 2,
            PropertyValue::Float(_)  => 3,
            PropertyValue::String(_) => 4,
            PropertyValue::Bytes(_)  => 5,
            PropertyValue::List(_)   => 6,
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

// ─── Conversión ──────────────────────────────────────────────────────────────
// API única de conversión del tipo-contrato. Ver el doc del enum para los
// `From` deliberadamente excluidos y las restricciones de evolución.

impl From<bool> for PropertyValue {
    fn from(v: bool) -> Self {
        PropertyValue::Bool(v)
    }
}

impl From<i64> for PropertyValue {
    fn from(v: i64) -> Self {
        PropertyValue::Int(v)
    }
}

/// Los literales enteros de Rust son `i32` por omisión; sin este impl,
/// `with_property("edad", 30)` exigiría anotar `30i64`.
impl From<i32> for PropertyValue {
    fn from(v: i32) -> Self {
        PropertyValue::Int(v as i64)
    }
}

impl From<u32> for PropertyValue {
    fn from(v: u32) -> Self {
        PropertyValue::Int(v as i64)
    }
}

impl From<f64> for PropertyValue {
    fn from(v: f64) -> Self {
        PropertyValue::Float(v)
    }
}

impl From<f32> for PropertyValue {
    fn from(v: f32) -> Self {
        PropertyValue::Float(v as f64)
    }
}

impl From<&str> for PropertyValue {
    fn from(v: &str) -> Self {
        PropertyValue::String(v.to_string())
    }
}

impl From<String> for PropertyValue {
    fn from(v: String) -> Self {
        PropertyValue::String(v)
    }
}

impl From<Vec<u8>> for PropertyValue {
    fn from(v: Vec<u8>) -> Self {
        PropertyValue::Bytes(v)
    }
}

impl From<Vec<PropertyValue>> for PropertyValue {
    fn from(v: Vec<PropertyValue>) -> Self {
        PropertyValue::List(v)
    }
}

impl From<Vec<(String, PropertyValue)>> for PropertyValue {
    fn from(v: Vec<(String, PropertyValue)>) -> Self {
        PropertyValue::Object(v)
    }
}

/// `None` → `Null`. Un `None` literal necesita anotación (`None::<&str>`)
/// porque el tipo interno no se puede inferir.
impl<T: Into<PropertyValue>> From<Option<T>> for PropertyValue {
    fn from(v: Option<T>) -> Self {
        match v {
            Some(inner) => inner.into(),
            None => PropertyValue::Null,
        }
    }
}

impl PropertyValue {
    /// Render humano recursivo: `Null`→`"null"`, `Float` no finito→`"null"`,
    /// `Bytes`→`"<N bytes>"`, `List`→`"[a, b]"`, `Object`→`"{k: v}"`.
    ///
    /// **NO es estable entre versiones y NUNCA debe usarse para claves de
    /// índice ni formatos persistidos** — el índice de propiedades en disco
    /// (`storage/mod.rs`) tiene su propia stringificación congelada, con
    /// semántica deliberadamente distinta (p. ej. NaN), para que una
    /// sustitución accidental sea detectable en tests.
    pub fn to_display_string(&self) -> String {
        match self {
            PropertyValue::Null => "null".to_string(),
            PropertyValue::Bool(b) => b.to_string(),
            PropertyValue::Int(i) => i.to_string(),
            PropertyValue::Float(f) => {
                if f.is_finite() {
                    f.to_string()
                } else {
                    "null".to_string()
                }
            }
            PropertyValue::String(s) => s.clone(),
            PropertyValue::Bytes(b) => format!("<{} bytes>", b.len()),
            PropertyValue::List(items) => {
                let inner: Vec<String> = items.iter().map(|v| v.to_display_string()).collect();
                format!("[{}]", inner.join(", "))
            }
            PropertyValue::Object(fields) => {
                let inner: Vec<String> = fields
                    .iter()
                    .map(|(k, v)| format!("{}: {}", k, v.to_display_string()))
                    .collect();
                format!("{{{}}}", inner.join(", "))
            }
        }
    }
}

/// Delegado a [`PropertyValue::to_display_string`] — mismas advertencias:
/// render humano, no estable, jamás para claves persistidas.
impl std::fmt::Display for PropertyValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_display_string())
    }
}

// Puente JSON. Matches exhaustivos a propósito: una variante nueva del enum
// debe fallar compilación aquí, no serializarse mal en silencio.

impl From<&PropertyValue> for serde_json::Value {
    fn from(pv: &PropertyValue) -> Self {
        match pv {
            PropertyValue::Null => serde_json::Value::Null,
            PropertyValue::Bool(b) => serde_json::Value::Bool(*b),
            PropertyValue::Int(i) => serde_json::Value::Number((*i).into()),
            // NaN/±inf no existen en JSON → null (igual que serde_json::json!)
            PropertyValue::Float(f) => serde_json::Number::from_f64(*f)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            PropertyValue::String(s) => serde_json::Value::String(s.clone()),
            // Igual que lo que Serialize (untagged) ya produce para Bytes:
            // arreglo de números. Pinneado en tests: Value::from(&pv) ==
            // serde_json::to_value(&pv) para toda variante EXCEPTO Object —
            // untagged serializa Vec<(String, PV)> como arreglo de pares
            // ([["k",1]]) y ni siquiera puede deserializar un objeto JSON;
            // el puente produce a propósito un objeto JSON real, que es la
            // forma natural de intercambio y la única que roundtripea.
            PropertyValue::Bytes(b) => {
                serde_json::Value::Array(b.iter().map(|x| (*x as u64).into()).collect())
            }
            PropertyValue::List(items) => {
                serde_json::Value::Array(items.iter().map(serde_json::Value::from).collect())
            }
            PropertyValue::Object(fields) => serde_json::Value::Object(
                fields
                    .iter()
                    .map(|(k, v)| (k.clone(), serde_json::Value::from(v)))
                    .collect(),
            ),
        }
    }
}

impl From<PropertyValue> for serde_json::Value {
    fn from(pv: PropertyValue) -> Self {
        match pv {
            // Por valor: mueve String/Vec sin clonar.
            PropertyValue::String(s) => serde_json::Value::String(s),
            PropertyValue::List(items) => {
                serde_json::Value::Array(items.into_iter().map(serde_json::Value::from).collect())
            }
            PropertyValue::Object(fields) => serde_json::Value::Object(
                fields
                    .into_iter()
                    .map(|(k, v)| (k, serde_json::Value::from(v)))
                    .collect(),
            ),
            other => serde_json::Value::from(&other),
        }
    }
}

impl From<&serde_json::Value> for PropertyValue {
    fn from(v: &serde_json::Value) -> Self {
        match v {
            serde_json::Value::Null => PropertyValue::Null,
            serde_json::Value::Bool(b) => PropertyValue::Bool(*b),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => PropertyValue::Int(i),
                None => PropertyValue::Float(n.as_f64().unwrap_or(0.0)),
            },
            serde_json::Value::String(s) => PropertyValue::String(s.clone()),
            // SIEMPRE List — nunca adivinar Bytes desde un arreglo JSON.
            // (Lossiness documentada: Bytes→JSON→PropertyValue regresa como
            // List(Int); la misma ambigüedad que ya tiene serde untagged.)
            serde_json::Value::Array(items) => {
                PropertyValue::List(items.iter().map(PropertyValue::from).collect())
            }
            serde_json::Value::Object(map) => PropertyValue::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), PropertyValue::from(v)))
                    .collect(),
            ),
        }
    }
}

impl From<serde_json::Value> for PropertyValue {
    fn from(v: serde_json::Value) -> Self {
        match v {
            serde_json::Value::String(s) => PropertyValue::String(s),
            serde_json::Value::Array(items) => {
                PropertyValue::List(items.into_iter().map(PropertyValue::from).collect())
            }
            serde_json::Value::Object(map) => PropertyValue::Object(
                map.into_iter()
                    .map(|(k, v)| (k, PropertyValue::from(v)))
                    .collect(),
            ),
            other => PropertyValue::from(&other),
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

    /// Agrega una propiedad. Acepta literales directamente vía
    /// `Into<PropertyValue>`: `.with_property("edad", 30)`.
    pub fn with_property(mut self, key: impl Into<String>, value: impl Into<PropertyValue>) -> Self {
        self.properties.insert(key.into(), value.into());
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

    /// Agrega una propiedad. Acepta literales directamente vía
    /// `Into<PropertyValue>`: `.with_property("cantidad", 18)`.
    pub fn with_property(mut self, key: impl Into<String>, value: impl Into<PropertyValue>) -> Self {
        self.properties.insert(key.into(), value.into());
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
    fn test_from_matrix() {
        assert_eq!(PropertyValue::from(true), PropertyValue::Bool(true));
        assert_eq!(PropertyValue::from(30i64), PropertyValue::Int(30));
        assert_eq!(PropertyValue::from(30i32), PropertyValue::Int(30));
        assert_eq!(PropertyValue::from(30u32), PropertyValue::Int(30));
        assert_eq!(PropertyValue::from(2.5f64), PropertyValue::Float(2.5));
        assert_eq!(PropertyValue::from(2.5f32), PropertyValue::Float(2.5));
        assert_eq!(PropertyValue::from("a"), PropertyValue::String("a".into()));
        assert_eq!(PropertyValue::from("a".to_string()), PropertyValue::String("a".into()));
        assert_eq!(PropertyValue::from(vec![1u8, 2]), PropertyValue::Bytes(vec![1, 2]));
        assert_eq!(
            PropertyValue::from(vec![PropertyValue::Int(1)]),
            PropertyValue::List(vec![PropertyValue::Int(1)])
        );
        assert_eq!(
            PropertyValue::from(vec![("k".to_string(), PropertyValue::Int(1))]),
            PropertyValue::Object(vec![("k".to_string(), PropertyValue::Int(1))])
        );
        // Option: Some → valor, None → Null (con anotación de tipo)
        assert_eq!(PropertyValue::from(Some(5i64)), PropertyValue::Int(5));
        assert_eq!(PropertyValue::from(None::<&str>), PropertyValue::Null);
    }

    #[test]
    fn test_with_property_literals() {
        let n = Node::new("T")
            .with_property("name", "Alice")
            .with_property("age", 30)
            .with_property("score", 2.5)
            .with_property("active", true)
            .with_property("pv", PropertyValue::Null); // PropertyValue directo sigue funcionando
        assert_eq!(n.properties["name"], PropertyValue::String("Alice".into()));
        assert_eq!(n.properties["age"], PropertyValue::Int(30));
        assert_eq!(n.properties["score"], PropertyValue::Float(2.5));
        assert_eq!(n.properties["active"], PropertyValue::Bool(true));
        assert_eq!(n.properties["pv"], PropertyValue::Null);
    }

    #[test]
    fn test_display_goldens() {
        assert_eq!(PropertyValue::Null.to_display_string(), "null");
        assert_eq!(PropertyValue::Bool(true).to_display_string(), "true");
        assert_eq!(PropertyValue::Int(-3).to_display_string(), "-3");
        assert_eq!(PropertyValue::Float(2.5).to_display_string(), "2.5");
        assert_eq!(PropertyValue::Float(f64::NAN).to_display_string(), "null");
        assert_eq!(PropertyValue::Float(f64::INFINITY).to_display_string(), "null");
        assert_eq!(PropertyValue::String("a".into()).to_display_string(), "a");
        assert_eq!(PropertyValue::Bytes(vec![1, 2]).to_display_string(), "<2 bytes>");
        let nested = PropertyValue::List(vec![
            PropertyValue::Int(1),
            PropertyValue::Object(vec![("k".to_string(), PropertyValue::String("v".into()))]),
        ]);
        assert_eq!(nested.to_display_string(), "[1, {k: v}]");
        // Display delega
        assert_eq!(format!("{}", nested), "[1, {k: v}]");
    }

    #[test]
    fn test_json_bridge_equals_serde() {
        // Value::from(&pv) coincide con lo que Serialize (untagged) produce
        // para toda variante EXCEPTO Object (ver abajo).
        let all = vec![
            PropertyValue::Null,
            PropertyValue::Bool(true),
            PropertyValue::Int(-7),
            PropertyValue::Float(2.5),
            PropertyValue::Float(f64::NAN),
            PropertyValue::String("a".into()),
            PropertyValue::Bytes(vec![1, 2, 3]),
            PropertyValue::List(vec![PropertyValue::Int(1), PropertyValue::Bool(false)]),
        ];
        for pv in &all {
            assert_eq!(
                serde_json::Value::from(pv),
                serde_json::to_value(pv).unwrap(),
                "bridge != serde para {pv:?}"
            );
            // La versión por valor coincide con la por referencia
            assert_eq!(serde_json::Value::from(pv), serde_json::Value::from(pv.clone()));
        }

        // Excepción deliberada: Object. Untagged serializa Vec<(String, PV)>
        // como arreglo de pares y NO puede deserializar un objeto JSON; el
        // puente produce un objeto JSON real (única forma que roundtripea).
        let obj = PropertyValue::Object(vec![("k".to_string(), PropertyValue::Int(1))]);
        assert_eq!(serde_json::Value::from(&obj), serde_json::json!({"k": 1}));
        assert_eq!(serde_json::to_value(&obj).unwrap(), serde_json::json!([["k", 1]]));
        assert!(
            serde_json::from_str::<PropertyValue>("{\"k\":1}").is_err(),
            "untagged no acepta objetos JSON — si esto cambia, revisar el puente"
        );
        assert_eq!(serde_json::Value::from(&obj), serde_json::Value::from(obj.clone()));
    }

    #[test]
    fn test_json_roundtrip_with_documented_lossiness() {
        // Escalares y estructuras roundtripean
        for pv in [
            PropertyValue::Null,
            PropertyValue::Bool(false),
            PropertyValue::Int(42),
            PropertyValue::Float(2.5),
            PropertyValue::String("x".into()),
            PropertyValue::Object(vec![("k".to_string(), PropertyValue::String("v".into()))]),
        ] {
            let back = PropertyValue::from(serde_json::Value::from(pv.clone()));
            assert_eq!(back, pv);
        }
        // Lossiness pinneada: Bytes → JSON array → regresa como List(Int)
        let bytes = PropertyValue::Bytes(vec![1, 2]);
        let back = PropertyValue::from(serde_json::Value::from(&bytes));
        assert_eq!(
            back,
            PropertyValue::List(vec![PropertyValue::Int(1), PropertyValue::Int(2)])
        );
        // NaN → JSON null → regresa como Null
        let nan = PropertyValue::Float(f64::NAN);
        assert_eq!(PropertyValue::from(serde_json::Value::from(&nan)), PropertyValue::Null);
        // Número JSON entero → Int, no Float
        let v: serde_json::Value = serde_json::json!(7);
        assert_eq!(PropertyValue::from(&v), PropertyValue::Int(7));
    }

    #[test]
    fn test_untagged_declaration_order_pins() {
        // El orden de variantes es load-bearing para serde(untagged); estos
        // pins hacen que un reorden accidental falle ruidosamente.
        let pv: PropertyValue = serde_json::from_str("7").unwrap();
        assert_eq!(pv, PropertyValue::Int(7), "los enteros deben parsear como Int, no Float");
        // Quirk existente documentado en el doc del enum: un arreglo JSON de
        // enteros pequeños deserializa como Bytes (Bytes va antes que List).
        let pv: PropertyValue = serde_json::from_str("[1,2,3]").unwrap();
        assert_eq!(pv, PropertyValue::Bytes(vec![1, 2, 3]));
    }

    #[test]
    fn test_property_value_as_number() {
        assert_eq!(PropertyValue::Int(18).as_number(), Some(18.0));
        assert_eq!(PropertyValue::Float(2.5).as_number(), Some(2.5));
        assert_eq!(PropertyValue::String("18".into()).as_number(), None);
        assert_eq!(PropertyValue::Null.as_number(), None);
    }

    #[test]
    fn test_property_value_hash() {
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert(PropertyValue::String("key".to_string()), vec![uuid::Uuid::new_v4()]);

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
