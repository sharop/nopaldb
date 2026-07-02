// src/shacl/mod.rs
//! SHACL Core — validacion de shapes sobre el grafo NopalDB.
//!
//! Implementa un subconjunto de SHACL Core (W3C) sin SPARQL, rutas complejas
//! (only single-hop) ni SHACL Advanced Features.
//!
//! Feature gate: compilado solo con `--features shacl`.
//!
//! # Uso rapido
//!
//! ```no_run
//! # async fn example() -> nopaldb::Result<()> {
//! use nopaldb::{Graph, shacl::{ShaclValidator, Shape, Target, ConstraintType, PropertyShape, PathSpec}};
//! use nopaldb::types::PropertyValue;
//!
//! let graph = Graph::in_memory().await?;
//!
//! // Definir un shape programatico
//! let shape = Shape::new("PersonShape")
//!     .with_target(Target::Class("Person".into()))
//!     .with_property_shape(
//!         PropertyShape::new(
//!             PathSpec::Property("age".into()),
//!             vec![ConstraintType::MinCount(1)],
//!         )
//!     );
//!
//! let validator = ShaclValidator::from_shapes(vec![shape]);
//! let report = validator.validate(&graph).await?;
//!
//! if !report.conforms {
//!     for v in &report.violations {
//!         println!("Violacion: {}", v.message);
//!     }
//! }
//! # Ok(())
//! # }
//! ```

pub mod shape;
pub mod constraint;
pub mod report;

pub use shape::{Shape, Target, PropertyShape, PathSpec, ConstraintType, DatatypeKind};
pub use constraint::{evaluate_constraints, evaluate_node_kind_constraint};
pub use report::{ValidationReport, ConstraintViolation, Severity};

use crate::error::Result;
use crate::graph::Graph;
use crate::types::{NodeId, PropertyValue};

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
    /// Para PropertyShapes anidadas, usar la API programatica en v1.
    pub async fn from_graph(graph: &Graph) -> Result<Self> {
        let mut shapes = Vec::new();

        let all_nodes = graph.get_all_nodes().await?;
        for node in &all_nodes {
            if node.label == "sh:NodeShape" {
                let shape_id = node.id;
                let shape_name = node
                    .properties
                    .get("sh:name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unnamed")
                    .to_string();

                let mut shape = Shape {
                    id: shape_id,
                    name: shape_name,
                    targets: vec![],
                    constraints: vec![],
                    property_shapes: vec![],
                };

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

        for shape in &self.shapes {
            let focus_nodes = self.resolve_focus_nodes(graph, shape).await?;
            for node_id in focus_nodes {
                let violations = self.validate_focus_node(graph, shape, node_id).await?;
                all_violations.extend(violations);
            }
        }

        Ok(ValidationReport::from_violations(all_violations))
    }

    /// Valida un nodo especifico contra todos los shapes registrados.
    pub async fn validate_node(
        &self,
        graph: &Graph,
        node_id: NodeId,
    ) -> Result<Vec<ConstraintViolation>> {
        let mut all_violations = Vec::new();

        for shape in &self.shapes {
            let violations = self.validate_focus_node(graph, shape, node_id).await?;
            all_violations.extend(violations);
        }

        Ok(all_violations)
    }

    /// Determina los focus nodes de un shape segun sus targets.
    async fn resolve_focus_nodes(
        &self,
        graph: &Graph,
        shape: &Shape,
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
                    focus.push(*id);
                }
                Target::Class(label) => {
                    let nodes = graph.get_nodes_by_label(label).await?;
                    focus.extend(nodes.iter().map(|n| n.id));
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
    ) -> Result<Vec<ConstraintViolation>> {
        let mut violations = Vec::new();

        let node = match graph.get_node(node_id).await {
            Ok(n) => n,
            Err(_) => return Ok(violations), // nodo no existe: ignorar
        };

        // Constraints directas sobre el nodo (NodeKind, Class)
        for constraint in &shape.constraints {
            if let Some(v) =
                evaluate_node_kind_constraint(&node, constraint, shape.id)
            {
                violations.push(v);
            }
        }

        // PropertyShapes
        for ps in &shape.property_shapes {
            let values = self.resolve_path_values(graph, &node, &ps.path).await?;
            let path_str = ps.path.as_str();
            let vs = evaluate_constraints(
                &ps.constraints,
                &values,
                node_id,
                shape.id,
                Some(path_str),
            );
            violations.extend(vs);
        }

        Ok(violations)
    }

    /// Resuelve los valores de un path sobre un nodo.
    ///
    /// - `PathSpec::Property(key)` → valores de `node.properties[key]`
    /// - `PathSpec::Edge(edge_type)` → nodos destino de aristas salientes
    ///   (retorna sus IDs como `PropertyValue::String(uuid)`)
    async fn resolve_path_values(
        &self,
        graph: &Graph,
        node: &crate::types::Node,
        path: &PathSpec,
    ) -> Result<Vec<PropertyValue>> {
        match path {
            PathSpec::Property(key) => {
                if let Some(v) = node.properties.get(key) {
                    Ok(vec![v.clone()])
                } else {
                    Ok(vec![])
                }
            }
            PathSpec::Edge(edge_type) => {
                let edges = graph.get_outgoing_edges(node.id).await?;
                let targets: Vec<PropertyValue> = edges
                    .iter()
                    .filter(|e| e.edge_type == *edge_type)
                    .map(|e| PropertyValue::String(e.target.to_string()))
                    .collect();
                Ok(targets)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Node, PropertyValue, NodeKind};
    use crate::graph::Graph;

    async fn temp_graph() -> Graph {
        Graph::in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn test_from_shapes_empty_report() {
        let graph = temp_graph().await;
        let validator = ShaclValidator::from_shapes(vec![]);
        let report = validator.validate(&graph).await.unwrap();
        assert!(report.conforms);
        assert!(report.violations.is_empty());
    }

    #[tokio::test]
    async fn test_no_violations_when_conforms() {
        let graph = temp_graph().await;
        let mut tx = graph.begin_transaction().await.unwrap();
        tx.add_node(
            Node::new("Person")
                .with_property("age", PropertyValue::Int(30))
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let shape = Shape::new("PersonShape")
            .with_target(Target::Class("Person".into()))
            .with_property_shape(PropertyShape::new(
                PathSpec::Property("age".into()),
                vec![ConstraintType::MinCount(1)],
            ));

        let validator = ShaclValidator::from_shapes(vec![shape]);
        let report = validator.validate(&graph).await.unwrap();
        assert!(report.conforms);
    }

    #[tokio::test]
    async fn test_min_count_violation_detected() {
        let graph = temp_graph().await;
        let mut tx = graph.begin_transaction().await.unwrap();
        // Bob no tiene "age"
        tx.add_node(Node::new("Person").with_property("name", PropertyValue::String("Bob".into())))
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let shape = Shape::new("PersonShape")
            .with_target(Target::Class("Person".into()))
            .with_property_shape(PropertyShape::new(
                PathSpec::Property("age".into()),
                vec![ConstraintType::MinCount(1)],
            ));

        let validator = ShaclValidator::from_shapes(vec![shape]);
        let report = validator.validate(&graph).await.unwrap();
        assert!(!report.conforms);
        assert_eq!(report.violations.len(), 1);
        assert!(report.violations[0].message.contains("minCount"));
    }

    #[tokio::test]
    async fn test_node_kind_constraint() {
        let graph = temp_graph().await;
        let mut tx = graph.begin_transaction().await.unwrap();
        // Un nodo con kind != Individual
        let mut node = Node::new("MyClass");
        node.kind = NodeKind::Class;
        tx.add_node(node).await.unwrap();
        tx.commit().await.unwrap();

        let shape = Shape::new("ClassShape")
            .with_target(Target::Class("MyClass".into()))
            .with_constraint(ConstraintType::NodeKindConstraint(NodeKind::Individual));

        let validator = ShaclValidator::from_shapes(vec![shape]);
        let report = validator.validate(&graph).await.unwrap();
        assert!(!report.conforms);
        assert_eq!(report.violations.len(), 1);
    }
}
