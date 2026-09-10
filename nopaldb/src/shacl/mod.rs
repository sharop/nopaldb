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

pub use shape::{Shape, Target, PropertyShape, PathSpec, PathValue, ConstraintType, DatatypeKind, ShaclNodeKind, PatternConstraint};
pub use constraint::{evaluate_constraints, evaluate_shape, component, EvalContext};
pub use report::{ValidationReport, ConstraintViolation, Severity, ShapesReport};
pub use turtle::parse_shapes;


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
    target_mode: TargetMode,
}

/// Cómo `sh:targetClass` elige sus focus nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TargetMode {
    /// Por la taxonomía del grafo (default): la clase se nombra por label,
    /// `prefix:Local` o IRI, y son focus nodes sus instancias directas y las
    /// de sus subclases, por cualquiera de sus tipos declarados. Sin
    /// taxonomía (grafo construido a mano) cae a `ExactLabel`.
    #[default]
    Taxonomy,
    /// El comportamiento anterior a 0.5.15: los individuos cuyo `label` es
    /// exactamente el texto del target. Ni subclases ni IRIs.
    ExactLabel,
}

impl ShaclValidator {
    /// Construye el validador con shapes programaticas (API fluida).
    ///
    /// Util para definir shapes en codigo sin cargar desde el grafo.
    pub fn from_shapes(shapes: Vec<Shape>) -> Self {
        Self { shapes, target_mode: TargetMode::default() }
    }

    /// Cambia cómo se resuelve `sh:targetClass` (ver [`TargetMode`]).
    pub fn with_target_mode(mut self, mode: TargetMode) -> Self {
        self.target_mode = mode;
        self
    }

    /// Construye el validador desde un documento Turtle de shapes SHACL.
    ///
    /// Devuelve también el [`ShapesReport`]: cuánto se cargó y qué términos
    /// `sh:*` este validador no comprueba (con razón). Un documento
    /// malformado es `Err` con línea y columna; no se carga nada.
    pub fn from_turtle(source: &str) -> Result<(Self, ShapesReport)> {
        let (shapes, report) = parse_shapes(source)?;
        Ok((Self { shapes, target_mode: TargetMode::default() }, report))
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

        Ok(Self { shapes, target_mode: TargetMode::default() })
    }

    /// Valida todos los nodos del grafo contra todos los shapes registrados.
    ///
    /// Retorna un `ValidationReport` que indica si el grafo conforma
    /// y lista todas las violaciones encontradas.
    pub async fn validate(&self, graph: &Graph) -> Result<ValidationReport> {
        let mut all_violations = Vec::new();
        let mut notes = Vec::new();
        let mut ctx = self.context(graph);

        for shape in &self.shapes {
            if shape.deactivated {
                continue;
            }
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
        let mut ctx = self.context(graph);

        for shape in &self.shapes {
            if shape.deactivated {
                continue;
            }
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
            // SHACL: a shape without targets has no focus nodes of its own;
            // it exists to be referenced (`sh:node`, `sh:or ( … )`). That is
            // what a shape loaded from Turtle (it has an `iri`) means. The
            // programmatic API keeps its older convention: no targets = every
            // node, which is what its callers rely on.
            if shape.iri.is_some() {
                return Ok(focus);
            }
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
                Target::Class(class) => {
                    focus.extend(self.instances_of(graph, class).await?);
                }
            }
        }

        // Deduplicar
        focus.sort_unstable();
        focus.dedup();
        Ok(focus)
    }

    /// Los focus nodes de `sh:targetClass C`.
    ///
    /// Con taxonomía: `C` se resuelve como en NQL (`TaxonomyIndex::resolve_class`:
    /// label, `prefix:Local` o IRI) y cuentan las instancias directas y las de
    /// sus subclases, por cualquiera de sus tipos declarados — el mismo
    /// criterio que `instanceOf(n, C)`. Sin taxonomía, o en
    /// [`TargetMode::ExactLabel`], los individuos cuyo `label` es el local
    /// name de `C`, que es lo que hacía siempre.
    ///
    /// Un nodo clase lleva el label de la clase (`:Receta a owl:Class` tiene
    /// label "Receta") pero una clase no es instancia de sí misma: nunca es
    /// focus node de su propio shape.
    async fn instances_of(&self, graph: &Graph, class: &str) -> Result<Vec<NodeId>> {
        use crate::rdf_owl::importer::local_name;
        use crate::types::NodeKind;

        if self.target_mode == TargetMode::Taxonomy
            && let Some(mut tax) = graph.get_taxonomy_sync()
            && let Some(class_id) = tax.resolve_class(class)
        {
            let all = graph.get_all_nodes().await?;
            return Ok(all
                .iter()
                .filter(|n| n.kind != NodeKind::Class && tax.is_instance_of(n.id, &n.label, class_id))
                .map(|n| n.id)
                .collect());
        }
        let nodes = graph.get_nodes_by_label(&local_name(class)).await?;
        Ok(nodes.iter().filter(|n| n.kind != NodeKind::Class).map(|n| n.id).collect())
    }

    /// El contexto de evaluación: taxonomía y las shapes del documento por
    /// IRI y por nombre (para `sh:node`).
    fn context(&self, graph: &Graph) -> EvalContext {
        let mut ctx = EvalContext { taxonomy: graph.get_taxonomy_sync(), ..Default::default() };
        for shape in &self.shapes {
            if let Some(iri) = &shape.iri {
                ctx.shapes.insert(iri.clone(), shape.clone());
            }
            ctx.shapes.entry(shape.name.clone()).or_insert_with(|| shape.clone());
        }
        ctx
    }

    /// Valida un focus node contra un shape especifico: carga en `ctx` los
    /// valores de todos los paths que la evaluación va a necesitar (los de la
    /// shape, y recursivamente los de las shapes de sus combinadores y
    /// `sh:node` sobre los valores-nodo) y evalúa en un solo paso síncrono.
    async fn validate_focus_node(
        &self,
        graph: &Graph,
        shape: &Shape,
        node_id: NodeId,
        ctx: &mut EvalContext,
    ) -> Result<Vec<ConstraintViolation>> {
        if graph.get_node(node_id).await.is_err() {
            return Ok(vec![]); // nodo no existe: ignorar
        }
        let mut visited = std::collections::HashSet::new();
        self.prefetch(graph, node_id, shape, ctx, &mut visited).await?;
        Ok(evaluate_shape(shape, &PathValue::Node(node_id), ctx))
    }

    /// Resuelve los paths de `shape` sobre `node_id` y, para cada shape que un
    /// combinador o `sh:node` aplicará a un valor-nodo, los de esa shape sobre
    /// ese valor. Acotado por `visited` (un par nodo/shape se carga una vez),
    /// así que un `sh:node` recursivo termina.
    fn prefetch<'a>(
        &'a self,
        graph: &'a Graph,
        node_id: NodeId,
        shape: &'a Shape,
        ctx: &'a mut EvalContext,
        visited: &'a mut std::collections::HashSet<(NodeId, NodeId)>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
        Box::pin(async move {
            if !visited.insert((node_id, shape.id)) {
                return Ok(());
            }
            let node = match ctx.nodes.get(&node_id) {
                Some(n) => n.clone(),
                None => {
                    let Ok(n) = graph.get_node(node_id).await else { return Ok(()) };
                    ctx.nodes.insert(node_id, n.clone());
                    n
                }
            };
            // Node-level combinators apply their shapes to the focus node itself.
            for c in &shape.constraints {
                for sub in self.referenced_shapes(c, ctx) {
                    self.prefetch(graph, node_id, &sub, ctx, visited).await?;
                }
            }
            for ps in &shape.property_shapes {
                let values = self.resolve_path_values(graph, &node, &ps.path, ctx).await?;
                for c in &ps.constraints {
                    for sub in self.referenced_shapes(c, ctx) {
                        for v in &values {
                            if let PathValue::Node(id) = v {
                                self.prefetch(graph, *id, &sub, ctx, visited).await?;
                            }
                        }
                    }
                }
                ctx.paths.insert((node_id, ps.path.clone()), values);
            }
            Ok(())
        })
    }

    /// Las shapes que `constraint` aplicará a sus valores: los miembros de un
    /// combinador, o la shape que `sh:node` refiere (si existe).
    fn referenced_shapes(&self, constraint: &ConstraintType, ctx: &EvalContext) -> Vec<Shape> {
        let mut out: Vec<Shape> = constraint.member_shapes().into_iter().cloned().collect();
        if let ConstraintType::Node(reference) = constraint
            && let Some(s) = ctx.shapes.get(reference)
        {
            out.push(s.clone());
        }
        out
    }

    /// Resuelve los valores de un path sobre un nodo, y deja en `ctx` los
    /// nodos destino para que las constraints de nodo puedan juzgarlos.
    ///
    /// - `PathSpec::Property(key)` → literales de `node.properties[key]`
    ///   (una `List`, un valor por elemento)
    /// - `PathSpec::Edge(edge_type)` → nodos destino de aristas salientes
    /// - `PathSpec::Predicate(name)` → ambos
    /// - `PathSpec::Sequence(steps)` → paso a paso sobre los valores-nodo;
    ///   un literal intermedio corta su rama; unión sin duplicados
    async fn resolve_path_values(
        &self,
        graph: &Graph,
        node: &Node,
        path: &PathSpec,
        ctx: &mut EvalContext,
    ) -> Result<Vec<PathValue>> {
        if let PathSpec::Sequence(steps) = path {
            let mut current: Vec<PathValue> = vec![PathValue::Node(node.id)];
            for step in steps {
                let mut next: Vec<PathValue> = Vec::new();
                for v in &current {
                    let PathValue::Node(id) = v else { continue }; // a literal has no further hops
                    let n = match ctx.nodes.get(id) {
                        Some(n) => n.clone(),
                        None => match graph.get_node(*id).await {
                            Ok(n) => {
                                ctx.nodes.insert(*id, n.clone());
                                n
                            }
                            Err(_) => continue,
                        },
                    };
                    for value in Box::pin(self.resolve_path_values(graph, &n, step, ctx)).await? {
                        if !next.contains(&value) {
                            next.push(value);
                        }
                    }
                }
                current = next;
                if current.is_empty() {
                    break;
                }
            }
            return Ok(current);
        }

        let mut values = Vec::new();
        let (property, edge) = match path {
            PathSpec::Property(k) => (Some(k), None),
            PathSpec::Edge(e) => (None, Some(e)),
            PathSpec::Predicate(p) => (Some(p), Some(p)),
            PathSpec::Sequence(_) => unreachable!("handled above"),
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
