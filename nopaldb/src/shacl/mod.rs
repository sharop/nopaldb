// src/shacl/mod.rs
//! SHACL Core — validacion de shapes sobre el grafo NopalDB.
//!
//! Implementa un subconjunto de SHACL Core (W3C) sin SPARQL: 14 constraints
//! sobre paths de un salto (`sh:minCount`, `sh:maxCount`, `sh:datatype`,
//! los cuatro rangos numéricos, `sh:minLength`/`sh:maxLength`, `sh:pattern`,
//! `sh:in`, `sh:hasValue`, `sh:nodeKind`, `sh:class`), con `sh:targetClass`
//! y `sh:targetNode`. Lo que no implementa lo dice: al cargar shapes desde
//! Turtle cada término no soportado sale en [`ShapesReport::ignored`] con la
//! razón, y una violación trae el componente, el path y el valor culpable.
//!
//! Feature gate: compilado solo con `--features shacl` (que trae el parser
//! Turtle del puente RDF y la taxonomía, porque las shapes son Turtle y
//! `sh:class` se resuelve por la jerarquía de clases).
//!
//! # Uso rapido
//!
//! ```no_run
//! # async fn example() -> nopaldb::Result<()> {
//! use nopaldb::Graph;
//!
//! let graph = Graph::in_memory().await?;
//! graph.import_turtle(r#"
//!   @prefix : <http://cocina.example/> .
//!   :Receta a <http://www.w3.org/2002/07/owl#Class> .
//!   :sopa a :Receta ; :tiempoMin "-5"^^<http://www.w3.org/2001/XMLSchema#integer> .
//! "#).await?;
//!
//! let (report, shapes) = graph.validate_shapes(r#"
//!   @prefix sh: <http://www.w3.org/ns/shacl#> .
//!   @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
//!   @prefix : <http://cocina.example/> .
//!   :RecetaShape a sh:NodeShape ; sh:targetClass :Receta ;
//!     sh:property [ sh:path :nombre ; sh:minCount 1 ] ,
//!                 [ sh:path :tiempoMin ; sh:datatype xsd:integer ; sh:minInclusive 1 ] .
//! "#).await?;
//!
//! assert!(shapes.ignored.is_empty());          // every sh:* term was understood
//! assert!(!report.conforms);
//! for v in &report.violations {
//!     println!("{} {} {:?}: {}", v.shape_name, v.constraint, v.path, v.message);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Las shapes también se construyen desde Rust ([`Shape::new`] y la API
//! fluida) o se leen de nodos `sh:NodeShape` del propio grafo
//! ([`ShaclValidator::from_graph`], limitado a `sh:targetClass` y
//! `sh:nodeKind`).

pub mod shape;
pub mod constraint;
pub mod report;
pub mod turtle;

pub use shape::{Shape, Target, PropertyShape, PathSpec, PathValue, ConstraintType, DatatypeKind, ShaclNodeKind};
pub use constraint::{evaluate_constraints, component, EvalContext};
pub use report::{ValidationReport, ConstraintViolation, Severity, ShapesReport};
pub use turtle::parse_shapes;

use std::collections::HashMap;

use crate::error::Result;
use crate::graph::Graph;
use crate::types::{Node, NodeId, PropertyValue};

/// Validador SHACL Core para grafos NopalDB.
///
/// Construido con una lista de shapes y ejecutado contra el grafo
/// para generar un `ValidationReport`.
///
/// No modifica el grafo bajo ningun concepto.
pub struct ShaclValidator {
    shapes: Vec<Shape>,
}

impl ShaclValidator {
    /// Construye el validador con shapes programaticas (API fluida).
    ///
    /// Util para definir shapes en codigo sin cargar desde el grafo.
    pub fn from_shapes(shapes: Vec<Shape>) -> Self {
        Self { shapes }
    }

    /// Construye el validador desde un documento Turtle de shapes SHACL.
    ///
    /// Devuelve también el [`ShapesReport`]: cuánto se cargó y qué términos
    /// `sh:*` este validador no comprueba (con razón). Un documento
    /// malformado es `Err` con línea y columna; no se carga nada.
    pub fn from_turtle(source: &str) -> Result<(Self, ShapesReport)> {
        let (shapes, report) = parse_shapes(source)?;
        Ok((Self { shapes }, report))
    }

    /// Las shapes cargadas.
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// Construye el validador leyendo shapes del grafo.
    ///
    /// Busca nodos con `label == "sh:NodeShape"` y construye shapes
    /// a partir de sus propiedades. Esto permite declarar shapes como
    /// nodos en el propio grafo.
    ///
    /// # Propiedades reconocidas en un nodo sh:NodeShape
    ///
    /// - `sh:targetClass` (String) — label de clase objetivo
    /// - `sh:nodeKind` (String) — nombre del NodeKind esperado
    ///
    /// Para property shapes, cargar el documento con [`Self::from_turtle`].
    pub async fn from_graph(graph: &Graph) -> Result<Self> {
        let mut shapes = Vec::new();

        let all_nodes = graph.get_all_nodes().await?;
        for node in &all_nodes {
            if node.label == "sh:NodeShape" {
                let shape_name = node
                    .properties
                    .get("sh:name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unnamed")
                    .to_string();

                let mut shape = Shape::new(shape_name);
                shape.id = node.id;

                // sh:targetClass
                if let Some(PropertyValue::String(label)) =
                    node.properties.get("sh:targetClass")
                {
                    shape.targets.push(Target::Class(label.clone()));
                }

                // sh:nodeKind
                if let Some(PropertyValue::String(kind_str)) =
                    node.properties.get("sh:nodeKind")
                {
                    use crate::types::NodeKind;
                    let kind = match kind_str.as_str() {
                        "Individual" => Some(NodeKind::Individual),
                        "Class" => Some(NodeKind::Class),
                        "ObjectProperty" => Some(NodeKind::ObjectProperty),
                        "DataProperty" => Some(NodeKind::DataProperty),
                        "Restriction" => Some(NodeKind::Restriction),
                        "AnnotationProperty" => Some(NodeKind::AnnotationProperty),
                        _ => None,
                    };
                    if let Some(k) = kind {
                        shape.constraints.push(ConstraintType::NodeKindConstraint(k));
                    }
                }

                shapes.push(shape);
            }
        }

        Ok(Self { shapes })
    }

    /// Valida todos los nodos del grafo contra todos los shapes registrados.
    ///
    /// Retorna un `ValidationReport` que indica si el grafo conforma
    /// y lista todas las violaciones encontradas.
    pub async fn validate(&self, graph: &Graph) -> Result<ValidationReport> {
        let mut all_violations = Vec::new();
        let mut notes = Vec::new();
        let mut ctx = EvalContext { nodes: HashMap::new(), taxonomy: graph.get_taxonomy_sync() };

        for shape in &self.shapes {
            let focus_nodes = self.resolve_focus_nodes(graph, shape, &mut notes).await?;
            for node_id in focus_nodes {
                let violations = self.validate_focus_node(graph, shape, node_id, &mut ctx).await?;
                all_violations.extend(violations);
            }
        }

        let mut report = ValidationReport::from_violations(all_violations);
        report.notes = notes;
        Ok(report)
    }

    /// Valida un nodo especifico contra todos los shapes registrados.
    pub async fn validate_node(
        &self,
        graph: &Graph,
        node_id: NodeId,
    ) -> Result<Vec<ConstraintViolation>> {
        let mut all_violations = Vec::new();
        let mut ctx = EvalContext { nodes: HashMap::new(), taxonomy: graph.get_taxonomy_sync() };

        for shape in &self.shapes {
            let violations = self.validate_focus_node(graph, shape, node_id, &mut ctx).await?;
            all_violations.extend(violations);
        }

        Ok(all_violations)
    }

    /// Determina los focus nodes de un shape segun sus targets.
    async fn resolve_focus_nodes(
        &self,
        graph: &Graph,
        shape: &Shape,
        notes: &mut Vec<String>,
    ) -> Result<Vec<NodeId>> {
        let mut focus = Vec::new();

        if shape.targets.is_empty() {
            // Sin targets: aplica a todos los nodos
            let all = graph.get_all_nodes().await?;
            focus.extend(all.iter().map(|n| n.id));
            return Ok(focus);
        }

        for target in &shape.targets {
            match target {
                Target::Node(id) => {
                    if graph.get_node(*id).await.is_ok() {
                        focus.push(*id);
                    } else {
                        notes.push(format!("{}: sh:targetNode {id} no existe en el grafo; nada que validar", shape.name));
                    }
                }
                Target::NodeIri(iri) => {
                    let nodes = graph
                        .get_all_nodes_by_property("iri", &PropertyValue::String(iri.clone()))
                        .await?;
                    if nodes.is_empty() {
                        notes.push(format!("{}: sh:targetNode <{iri}> no existe en el grafo; nada que validar", shape.name));
                    }
                    focus.extend(nodes.iter().copied());
                }
                Target::Class(label) => {
                    // A class node carries the class label too (`:Receta a
                    // owl:Class` has label "Receta"), but a class is not an
                    // instance of itself: validating it against its own shape
                    // reported every minCount as a violation of the class.
                    let nodes = graph.get_nodes_by_label(label).await?;
                    focus.extend(nodes.iter().filter(|n| n.kind != crate::types::NodeKind::Class).map(|n| n.id));
                }
            }
        }

        // Deduplicar
        focus.sort_unstable();
        focus.dedup();
        Ok(focus)
    }

    /// Valida un focus node contra un shape especifico.
    async fn validate_focus_node(
        &self,
        graph: &Graph,
        shape: &Shape,
        node_id: NodeId,
        ctx: &mut EvalContext,
    ) -> Result<Vec<ConstraintViolation>> {
        let mut violations = Vec::new();

        let node = match graph.get_node(node_id).await {
            Ok(n) => n,
            Err(_) => return Ok(violations), // nodo no existe: ignorar
        };
        ctx.nodes.insert(node_id, node.clone());

        // Constraints directas sobre el nodo: el propio focus node es el valor.
        if !shape.constraints.is_empty() {
            let focus = [PathValue::Node(node_id)];
            violations.extend(evaluate_constraints(&shape.constraints, &focus, node_id, shape.id, None, ctx));
        }

        // PropertyShapes
        for ps in &shape.property_shapes {
            let values = self.resolve_path_values(graph, &node, &ps.path, ctx).await?;
            let vs = evaluate_constraints(&ps.constraints, &values, node_id, shape.id, Some(ps.path.as_str()), ctx);
            violations.extend(vs);
        }

        for v in &mut violations {
            v.shape_name = shape.name.clone();
        }
        Ok(violations)
    }

    /// Resuelve los valores de un path sobre un nodo, y deja en `ctx` los
    /// nodos destino para que las constraints de nodo puedan juzgarlos.
    ///
    /// - `PathSpec::Property(key)` → literales de `node.properties[key]`
    ///   (una `List`, un valor por elemento)
    /// - `PathSpec::Edge(edge_type)` → nodos destino de aristas salientes
    /// - `PathSpec::Predicate(name)` → ambos
    async fn resolve_path_values(
        &self,
        graph: &Graph,
        node: &Node,
        path: &PathSpec,
        ctx: &mut EvalContext,
    ) -> Result<Vec<PathValue>> {
        let mut values = Vec::new();
        let (property, edge) = match path {
            PathSpec::Property(k) => (Some(k), None),
            PathSpec::Edge(e) => (None, Some(e)),
            PathSpec::Predicate(p) => (Some(p), Some(p)),
        };
        if let Some(key) = property
            && let Some(v) = node.properties.get(key)
        {
            match v {
                PropertyValue::List(items) => values.extend(items.iter().cloned().map(PathValue::Literal)),
                other => values.push(PathValue::Literal(other.clone())),
            }
        }
        if let Some(edge_type) = edge {
            let edges = graph.get_outgoing_edges(node.id).await?;
            for e in edges.iter().filter(|e| e.edge_type == *edge_type) {
                if !ctx.nodes.contains_key(&e.target)
                    && let Ok(target) = graph.get_node(e.target).await
                {
                    ctx.nodes.insert(e.target, target);
                }
                values.push(PathValue::Node(e.target));
            }
        }
        Ok(values)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Node, NodeKind};

    #[tokio::test]
    async fn test_from_shapes_empty_report() {
        let graph = Graph::in_memory().await.unwrap();
        let validator = ShaclValidator::from_shapes(vec![]);
        let report = validator.validate(&graph).await.unwrap();
        assert!(report.conforms);
        assert!(report.violations.is_empty());
    }

    #[tokio::test]
    async fn test_no_violations_when_conforms() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(Node::new("Person").with_property("age", PropertyValue::Int(30))).await.unwrap();
        tx.commit().await.unwrap();

        let shape = Shape::new("PersonShape")
            .with_target(Target::Class("Person".into()))
            .with_property_shape(PropertyShape::new(
                PathSpec::Property("age".into()),
                vec![ConstraintType::MinCount(1)],
            ));
        let report = ShaclValidator::from_shapes(vec![shape]).validate(&graph).await.unwrap();
        assert!(report.conforms);
    }

    #[tokio::test]
    async fn test_min_count_violation_detected() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(Node::new("Person")).await.unwrap();
        tx.commit().await.unwrap();

        let shape = Shape::new("PersonShape")
            .with_target(Target::Class("Person".into()))
            .with_property_shape(PropertyShape::new(
                PathSpec::Property("age".into()),
                vec![ConstraintType::MinCount(1)],
            ));
        let report = ShaclValidator::from_shapes(vec![shape]).validate(&graph).await.unwrap();
        assert!(!report.conforms);
        assert_eq!(report.violations.len(), 1);
        assert_eq!(report.violations[0].shape_name, "PersonShape");
        assert_eq!(report.violations[0].constraint, "sh:MinCountConstraintComponent");
        assert_eq!(report.violations[0].path.as_deref(), Some("age"));
    }

    #[tokio::test]
    async fn test_node_kind_constraint() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(Node::new("Thing")).await.unwrap(); // Individual
        tx.commit().await.unwrap();

        let shape = Shape::new("ClassShape")
            .with_target(Target::Class("Thing".into()))
            .with_constraint(ConstraintType::NodeKindConstraint(NodeKind::Class));
        let report = ShaclValidator::from_shapes(vec![shape]).validate(&graph).await.unwrap();
        assert!(!report.conforms, "an Individual is not a Class");
        assert_eq!(report.violations[0].constraint, "sh:NodeKindConstraintComponent");
    }

    #[tokio::test]
    async fn test_list_property_counts_one_value_per_element() {
        let graph = Graph::in_memory().await.unwrap();
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(Node::new("Receta").with_property(
            "ingrediente",
            PropertyValue::List(vec![PropertyValue::String("canela".into()), PropertyValue::String("piloncillo".into())]),
        ))
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let shape = Shape::new("R")
            .with_target(Target::Class("Receta".into()))
            .with_property_shape(PropertyShape::new(PathSpec::Predicate("ingrediente".into()), vec![ConstraintType::MinCount(2)]));
        let report = ShaclValidator::from_shapes(vec![shape]).validate(&graph).await.unwrap();
        assert!(report.conforms, "{:?}", report.violations);
    }

    #[tokio::test]
    async fn test_missing_target_node_is_noted_not_silent() {
        let graph = Graph::in_memory().await.unwrap();
        let shape = Shape::new("S").with_target(Target::NodeIri("http://cocina.example/nada".into()));
        let report = ShaclValidator::from_shapes(vec![shape]).validate(&graph).await.unwrap();
        assert!(report.conforms);
        assert_eq!(report.notes.len(), 1, "{:?}", report.notes);
        assert!(report.notes[0].contains("cocina.example/nada"));
    }
}
