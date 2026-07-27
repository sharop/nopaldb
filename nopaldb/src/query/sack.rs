// src/query/sack.rs
//
// Acumulador por traverser ("sack") para traversals fluidos.
//
// Cada traverser carga un valor que se pliega con la arista en cada paso,
// preservando multiplicidad por camino: el mismo nodo alcanzado por dos
// ramas distintas aparece dos veces, con acumuladores distintos.

//! Acumulador por traverser para traversals.
//!
//! Permite acarrear un valor a lo largo del recorrido, transformándolo en
//! cada arista — por ejemplo, la explosión de materiales de un BOM donde la
//! cantidad se multiplica al bajar por cada componente:
//!
//! ```no_run
//! # async fn demo() -> nopaldb::Result<()> {
//! # use nopaldb::{Graph, Node};
//! # let graph = Graph::in_memory().await?;
//! # let raiz = graph.add_node(Node::new("Material")).await?;
//! let r = graph.traverse(raiz)
//!     .sack(10.0)
//!     .repeat(|b| b.out_e("ContieneComponente").sack_mul_by("cantidad"))
//!     .emit()
//!     .await?;
//! for item in &r.items {
//!     println!("{} necesita {}", item.node, item.sack);
//! }
//! assert!(r.is_complete());
//! # Ok(()) }
//! ```
//!
//! A diferencia de [`Graph::bfs`](crate::Graph::bfs)/[`Graph::dfs`](crate::Graph::dfs),
//! aquí no hay deduplicación por nodo: la multiplicidad por camino es parte
//! del resultado. Los ciclos se detectan sobre la línea de ancestros de cada
//! traverser (un diamante no es un ciclo) y el truncamiento por
//! `max_depth`/`max_nodes` se reporta en [`SackResult::truncated`] en vez de
//! cortar en silencio.

use std::fmt;
use std::sync::Arc;

use crate::error::{NopalError, Result};
use crate::graph::{Direction, Graph};
use crate::query::filter::NodePredicate;
use crate::query::step::TraversalStep;
use crate::types::{Edge, Node, NodeId};

/// Pliegue del acumulador: recibe la arista recién seguida y el valor actual.
pub type SackFold<T> = Arc<dyn Fn(&Edge, &T) -> Result<T> + Send + Sync>;

/// Predicado sobre la última arista seguida.
pub type EdgePredicate = Arc<dyn Fn(&Edge) -> bool + Send + Sync>;

/// Qué hacer al detectar un ciclo (un traverser que vuelve a un nodo de su
/// propia línea de ancestros) dentro de `repeat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CycleMode {
    /// Abortar con un error que nombra el camino del ciclo (default).
    Error,
    /// Descartar el traverser y contarlo en [`SackResult::cycles_skipped`].
    Skip,
}

/// Por qué se detuvo el recorrido antes de agotar el frontier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Truncation {
    /// Se descartaron traversers que excedían `max_depth`.
    MaxDepth,
    /// Se alcanzó el tope de traversers creados (`max_nodes`).
    MaxNodes,
}

/// Un nodo emitido junto con su acumulador y profundidad.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SackItem<T> {
    pub node: NodeId,
    pub sack: T,
    pub depth: usize,
}

/// Resultado de un traversal con acumulador.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct SackResult<T> {
    /// Pares (nodo, acumulador) en orden de visita.
    pub items: Vec<SackItem<T>>,
    /// `None` = el recorrido terminó; `Some` = se detuvo por un tope.
    pub truncated: Option<Truncation>,
    /// Traversers descartados por ciclo bajo [`CycleMode::Skip`].
    pub cycles_skipped: usize,
}

impl<T> SackResult<T> {
    /// `true` si el recorrido terminó sin toparse con `max_depth`/`max_nodes`.
    pub fn is_complete(&self) -> bool {
        self.truncated.is_none()
    }
}

/// Un paso del pipeline sack. Guarda closures directamente.
enum SackStep<T> {
    FollowEdge {
        edge_type: Option<String>,
        direction: Direction,
    },
    Fold(SackFold<T>),
    FilterNode(NodePredicate),
    FilterEdge(EdgePredicate),
    Limit(usize),
    Skip(usize),
}

impl<T> fmt::Debug for SackStep<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SackStep::FollowEdge { edge_type, direction } => f
                .debug_struct("FollowEdge")
                .field("edge_type", edge_type)
                .field("direction", direction)
                .finish(),
            SackStep::Fold(_) => write!(f, "Fold(<closure>)"),
            SackStep::FilterNode(_) => write!(f, "FilterNode(<closure>)"),
            SackStep::FilterEdge(_) => write!(f, "FilterEdge(<closure>)"),
            SackStep::Limit(n) => write!(f, "Limit({n})"),
            SackStep::Skip(n) => write!(f, "Skip({n})"),
        }
    }
}

/// Eslabón inmutable de la cadena de ancestros de un traverser.
struct PathLink {
    node: NodeId,
    parent: Option<Arc<PathLink>>,
}

/// Un traverser: posición actual + acumulador + contexto del camino.
struct Traverser<T> {
    node: NodeId,
    sack: T,
    depth: usize,
    last_edge: Option<Arc<Edge>>,
    path: Arc<PathLink>,
}

impl<T: Clone> Clone for Traverser<T> {
    fn clone(&self) -> Self {
        Self {
            node: self.node,
            sack: self.sack.clone(),
            depth: self.depth,
            last_edge: self.last_edge.clone(),
            path: self.path.clone(),
        }
    }
}

impl<T> Traverser<T> {
    fn ancestry_contains(&self, node: NodeId) -> bool {
        let mut cur = Some(&self.path);
        while let Some(link) = cur {
            if link.node == node {
                return true;
            }
            cur = link.parent.as_ref();
        }
        false
    }
}

/// Grabador de pasos para el bloque de `repeat`.
pub struct SackBlock<T> {
    steps: Vec<SackStep<T>>,
    has_follow: bool,
    error: Option<String>,
}

/// Builder de un traversal con acumulador. Se obtiene con
/// [`TraverseBuilder::sack`](crate::TraverseBuilder::sack).
pub struct SackBuilder<T> {
    graph: Arc<Graph>,
    start: Vec<NodeId>,
    init: T,
    prefix: Vec<SackStep<T>>,
    repeat_block: Option<Vec<SackStep<T>>>,
    has_follow: bool,
    max_depth: usize,
    max_nodes: usize,
    on_cycle: CycleMode,
    config_error: Option<String>,
}

macro_rules! sack_step_methods {
    () => {
        /// Sigue aristas salientes de cualquier tipo.
        pub fn out(self) -> Self {
            self.push_step(SackStep::FollowEdge {
                edge_type: None,
                direction: Direction::Outgoing,
            })
        }

        /// Sigue aristas salientes de un tipo específico.
        pub fn out_e(self, edge_type: impl Into<String>) -> Self {
            self.push_step(SackStep::FollowEdge {
                edge_type: Some(edge_type.into()),
                direction: Direction::Outgoing,
            })
        }

        /// Sigue aristas entrantes de cualquier tipo.
        pub fn in_(self) -> Self {
            self.push_step(SackStep::FollowEdge {
                edge_type: None,
                direction: Direction::Incoming,
            })
        }

        /// Sigue aristas entrantes de un tipo específico.
        pub fn in_e(self, edge_type: impl Into<String>) -> Self {
            self.push_step(SackStep::FollowEdge {
                edge_type: Some(edge_type.into()),
                direction: Direction::Incoming,
            })
        }

        /// Sigue aristas en ambas direcciones.
        pub fn both(self) -> Self {
            self.push_step(SackStep::FollowEdge {
                edge_type: None,
                direction: Direction::Both,
            })
        }

        /// Filtra traversers por predicado sobre el nodo actual.
        pub fn filter<F>(self, predicate: F) -> Self
        where
            F: Fn(&Node) -> bool + Send + Sync + 'static,
        {
            self.push_step(SackStep::FilterNode(Arc::new(predicate)))
        }

        /// Filtra traversers por predicado sobre la última arista seguida.
        pub fn filter_edge<F>(self, predicate: F) -> Self
        where
            F: Fn(&Edge) -> bool + Send + Sync + 'static,
        {
            self.push_edge_dependent(SackStep::FilterEdge(Arc::new(predicate)), "filter_edge")
        }

        /// Pliega la última arista seguida dentro del acumulador.
        pub fn sack_by<F>(self, f: F) -> Self
        where
            F: Fn(&Edge, &T) -> T + Send + Sync + 'static,
        {
            self.push_edge_dependent(SackStep::Fold(Arc::new(move |e, s| Ok(f(e, s)))), "sack_by")
        }

        /// Como [`Self::sack_by`], pero el pliegue puede fallar.
        pub fn try_sack_by<F>(self, f: F) -> Self
        where
            F: Fn(&Edge, &T) -> Result<T> + Send + Sync + 'static,
        {
            self.push_edge_dependent(SackStep::Fold(Arc::new(f)), "try_sack_by")
        }

        fn push_edge_dependent(mut self, step: SackStep<T>, name: &str) -> Self {
            if !self.has_follow {
                self.note_error(format!(
                    "{name} requiere un paso de arista previo (out/out_e/in_/in_e/both)"
                ));
                return self;
            }
            self.push_step(step)
        }
    };
}

/// Pliegue numérico sobre una propiedad de arista. Propiedad faltante o no
/// numérica es un error duro: tratarla como identidad produciría un número
/// silenciosamente equivocado.
fn numeric_fold(prop: String, op: &'static str, apply: fn(f64, f64) -> f64) -> SackFold<f64> {
    Arc::new(move |edge, acc| {
        let v = edge
            .properties
            .get(&prop)
            .and_then(|v| v.as_number())
            .ok_or_else(|| {
                NopalError::query_error(format!(
                    "{op}: la arista {} ({}) no tiene propiedad numérica '{}'",
                    edge.id, edge.edge_type, prop
                ))
            })?;
        Ok(apply(*acc, v))
    })
}

macro_rules! sack_numeric_methods {
    () => {
        /// Multiplica el acumulador por la propiedad numérica de la arista.
        /// Propiedad faltante o no numérica → error en `emit()`.
        pub fn sack_mul_by(self, prop: impl Into<String>) -> Self {
            let fold = numeric_fold(prop.into(), "sack_mul_by", |acc, v| acc * v);
            self.push_edge_dependent(SackStep::Fold(fold), "sack_mul_by")
        }

        /// Suma la propiedad numérica de la arista al acumulador.
        /// Propiedad faltante o no numérica → error en `emit()`.
        pub fn sack_sum_by(self, prop: impl Into<String>) -> Self {
            let fold = numeric_fold(prop.into(), "sack_sum_by", |acc, v| acc + v);
            self.push_edge_dependent(SackStep::Fold(fold), "sack_sum_by")
        }
    };
}

impl<T: Clone + Send + Sync + 'static> SackBlock<T> {
    fn new(has_follow: bool) -> Self {
        Self {
            steps: Vec::new(),
            has_follow,
            error: None,
        }
    }

    fn push_step(mut self, step: SackStep<T>) -> Self {
        if matches!(step, SackStep::FollowEdge { .. }) {
            self.has_follow = true;
        }
        self.steps.push(step);
        self
    }

    fn note_error(&mut self, msg: String) {
        self.error.get_or_insert(msg);
    }

    sack_step_methods!();
}

impl SackBlock<f64> {
    sack_numeric_methods!();
}

impl<T: Clone + Send + Sync + 'static> SackBuilder<T> {
    /// Construye desde las partes de un `TraverseBuilder`: los pasos ya
    /// grabados se convierten al pipeline sack y corren como prefijo.
    pub(crate) fn from_traverse(
        graph: Arc<Graph>,
        start: Vec<NodeId>,
        init: T,
        steps: Vec<TraversalStep>,
        predicates: Vec<NodePredicate>,
    ) -> Self {
        let mut prefix = Vec::with_capacity(steps.len());
        let mut has_follow = false;
        let mut pred_idx = 0;

        for step in steps {
            match step {
                TraversalStep::FollowEdge { edge_type, direction } => {
                    has_follow = true;
                    prefix.push(SackStep::FollowEdge { edge_type, direction });
                }
                TraversalStep::Filter { .. } => {
                    if pred_idx < predicates.len() {
                        prefix.push(SackStep::FilterNode(predicates[pred_idx].clone()));
                        pred_idx += 1;
                    }
                }
                TraversalStep::Limit { count } => prefix.push(SackStep::Limit(count)),
                TraversalStep::Skip { count } => prefix.push(SackStep::Skip(count)),
            }
        }

        Self {
            graph,
            start,
            init,
            prefix,
            repeat_block: None,
            has_follow,
            max_depth: 32,
            max_nodes: 10_000,
            on_cycle: CycleMode::Error,
            config_error: None,
        }
    }

    fn push_step(mut self, step: SackStep<T>) -> Self {
        if self.repeat_block.is_some() {
            self.note_error("no se admiten pasos después de repeat()".to_string());
            return self;
        }
        if matches!(step, SackStep::FollowEdge { .. }) {
            self.has_follow = true;
        }
        self.prefix.push(step);
        self
    }

    fn note_error(&mut self, msg: String) {
        self.config_error.get_or_insert(msg);
    }

    sack_step_methods!();

    /// Repite el bloque de pasos hasta que el frontier quede vacío. El
    /// recorrido siempre está acotado por [`Self::max_depth`] y
    /// [`Self::max_nodes`]; los ciclos se manejan según [`Self::on_cycle`].
    pub fn repeat(mut self, f: impl FnOnce(SackBlock<T>) -> SackBlock<T>) -> Self {
        if self.repeat_block.is_some() {
            self.note_error("repeat() solo puede llamarse una vez".to_string());
            return self;
        }
        let block = f(SackBlock::new(false));
        if let Some(err) = block.error {
            self.note_error(err);
        }
        self.repeat_block = Some(block.steps);
        self
    }

    /// Profundidad máxima (default 32). La guarda es obligatoria: no puede
    /// desactivarse, solo ajustarse.
    pub fn max_depth(mut self, depth: usize) -> Self {
        self.max_depth = depth;
        self
    }

    /// Tope de traversers creados (default 10 000, paridad con
    /// [`TraversalConfig`](crate::TraversalConfig)).
    pub fn max_nodes(mut self, max: usize) -> Self {
        self.max_nodes = max;
        self
    }

    /// Manejo de ciclos dentro de `repeat` (default [`CycleMode::Error`]).
    pub fn on_cycle(mut self, mode: CycleMode) -> Self {
        self.on_cycle = mode;
        self
    }

    /// Ejecuta y emite cada nodo alcanzado al cierre de cada bloque: el
    /// frontier tras el prefijo y el de cada iteración de `repeat`. Los
    /// nodos de inicio no se emiten (nada se ha plegado aún).
    pub async fn emit(self) -> Result<SackResult<T>> {
        self.run(false).await
    }

    /// Ejecuta y emite solo las hojas: traversers que no producen sucesores
    /// en una iteración de `repeat` (o el frontier final si no hay `repeat`).
    pub async fn emit_leaves(self) -> Result<SackResult<T>> {
        self.run(true).await
    }

    async fn run(self, leaves_only: bool) -> Result<SackResult<T>> {
        if let Some(msg) = self.config_error {
            return Err(NopalError::query_error(msg));
        }

        let mut ctx = RunCtx {
            max_depth: self.max_depth,
            max_nodes: self.max_nodes,
            on_cycle: self.on_cycle,
            created: 0,
            cycles_skipped: 0,
            truncated: None,
            stop: false,
        };
        let mut items: Vec<SackItem<T>> = Vec::new();

        let mut frontier: Vec<Traverser<T>> = self
            .start
            .iter()
            .map(|&node| Traverser {
                node,
                sack: self.init.clone(),
                depth: 0,
                last_edge: None,
                path: Arc::new(PathLink { node, parent: None }),
            })
            .collect();

        // Prefijo: los ciclos no se vigilan aquí (una secuencia fija de
        // pasos siempre termina, y p.ej. both().both() revisita el origen
        // legítimamente).
        frontier = apply_block(&self.graph, &self.prefix, frontier, &mut ctx, false).await?;

        if !leaves_only {
            emit_frontier(&frontier, &mut items);
        }

        match self.repeat_block {
            None => {
                if leaves_only {
                    for t in &frontier {
                        items.push(SackItem {
                            node: t.node,
                            sack: t.sack.clone(),
                            depth: t.depth,
                        });
                    }
                }
            }
            Some(block) => {
                while !frontier.is_empty() && !ctx.stop {
                    let mut next = Vec::new();
                    for t in frontier {
                        if ctx.stop {
                            break;
                        }
                        let children =
                            apply_block(&self.graph, &block, vec![t.clone()], &mut ctx, true)
                                .await?;
                        if leaves_only && children.is_empty() {
                            items.push(SackItem {
                                node: t.node,
                                sack: t.sack,
                                depth: t.depth,
                            });
                        }
                        next.extend(children);
                    }
                    if !leaves_only {
                        emit_frontier(&next, &mut items);
                    }
                    frontier = next;
                }
            }
        }

        Ok(SackResult {
            items,
            truncated: ctx.truncated,
            cycles_skipped: ctx.cycles_skipped,
        })
    }
}

impl SackBuilder<f64> {
    sack_numeric_methods!();
}

struct RunCtx {
    max_depth: usize,
    max_nodes: usize,
    on_cycle: CycleMode,
    created: usize,
    cycles_skipped: usize,
    truncated: Option<Truncation>,
    stop: bool,
}

fn emit_frontier<T: Clone>(frontier: &[Traverser<T>], items: &mut Vec<SackItem<T>>) {
    for t in frontier {
        if t.depth > 0 {
            items.push(SackItem {
                node: t.node,
                sack: t.sack.clone(),
                depth: t.depth,
            });
        }
    }
}

async fn apply_block<T: Clone + Send + Sync + 'static>(
    graph: &Graph,
    steps: &[SackStep<T>],
    mut frontier: Vec<Traverser<T>>,
    ctx: &mut RunCtx,
    check_cycles: bool,
) -> Result<Vec<Traverser<T>>> {
    for step in steps {
        match step {
            SackStep::FollowEdge { edge_type, direction } => {
                let mut next = Vec::new();
                for t in &frontier {
                    if ctx.stop {
                        break;
                    }
                    let edges = graph.edges_of(t.node, *direction).await?;
                    for edge in edges {
                        if let Some(et) = edge_type
                            && edge.edge_type != *et
                        {
                            continue;
                        }

                        let target = match direction {
                            Direction::Outgoing => edge.target,
                            Direction::Incoming => edge.source,
                            Direction::Both => {
                                if edge.source == t.node {
                                    edge.target
                                } else {
                                    edge.source
                                }
                            }
                        };

                        let child_depth = t.depth + 1;
                        if child_depth > ctx.max_depth {
                            ctx.truncated.get_or_insert(Truncation::MaxDepth);
                            continue;
                        }

                        if check_cycles && t.ancestry_contains(target) {
                            match ctx.on_cycle {
                                CycleMode::Error => {
                                    return Err(cycle_error(graph, t, target).await);
                                }
                                CycleMode::Skip => {
                                    ctx.cycles_skipped += 1;
                                    continue;
                                }
                            }
                        }

                        if ctx.created >= ctx.max_nodes {
                            ctx.truncated = Some(Truncation::MaxNodes);
                            ctx.stop = true;
                            break;
                        }
                        ctx.created += 1;

                        next.push(Traverser {
                            node: target,
                            sack: t.sack.clone(),
                            depth: child_depth,
                            last_edge: Some(Arc::new(edge)),
                            path: Arc::new(PathLink {
                                node: target,
                                parent: Some(t.path.clone()),
                            }),
                        });
                    }
                }
                frontier = next;
            }

            SackStep::Fold(fold) => {
                for t in &mut frontier {
                    let edge = t.last_edge.as_ref().ok_or_else(|| {
                        NopalError::query_error(
                            "el pliegue del sack requiere un paso de arista previo",
                        )
                    })?;
                    t.sack = fold(edge, &t.sack)?;
                }
            }

            SackStep::FilterNode(predicate) => {
                let mut kept = Vec::with_capacity(frontier.len());
                for t in frontier {
                    let node = graph.get_node(t.node).await?;
                    if predicate(&node) {
                        kept.push(t);
                    }
                }
                frontier = kept;
            }

            SackStep::FilterEdge(predicate) => {
                let mut kept = Vec::with_capacity(frontier.len());
                for t in frontier {
                    let edge = t.last_edge.as_ref().ok_or_else(|| {
                        NopalError::query_error(
                            "filter_edge requiere un paso de arista previo",
                        )
                    })?;
                    if predicate(edge) {
                        kept.push(t);
                    }
                }
                frontier = kept;
            }

            SackStep::Limit(count) => frontier.truncate(*count),

            SackStep::Skip(count) => {
                if *count < frontier.len() {
                    frontier.drain(..*count);
                } else {
                    frontier.clear();
                }
            }
        }

        if frontier.is_empty() {
            break;
        }
    }

    Ok(frontier)
}

/// Construye el error de ciclo nombrando el camino (`A → B → A`).
async fn cycle_error<T>(graph: &Graph, traverser: &Traverser<T>, repeated: NodeId) -> NopalError {
    let mut chain = Vec::new();
    let mut cur = Some(&traverser.path);
    while let Some(link) = cur {
        chain.push(link.node);
        if link.node == repeated {
            break;
        }
        cur = link.parent.as_ref();
    }
    chain.reverse();
    chain.push(repeated);

    let mut names = Vec::with_capacity(chain.len());
    for id in chain {
        let name = match graph.get_node(id).await {
            Ok(node) => node
                .properties
                .get("name")
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or(node.label),
            Err(_) => id.to_string(),
        };
        names.push(name);
    }

    NopalError::query_error(format!("cycle detected: {}", names.join(" → ")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::PropertyValue;

    async fn graph_with_edge(cantidad: PropertyValue) -> (Graph, NodeId, NodeId) {
        let graph = Graph::in_memory().await.unwrap();
        let a = graph
            .add_node(Node::new("Material").with_property("name", PropertyValue::String("A".into())))
            .await
            .unwrap();
        let b = graph
            .add_node(Node::new("Material").with_property("name", PropertyValue::String("B".into())))
            .await
            .unwrap();
        graph
            .add_edge(Edge::new(a, b, "ContieneComponente").with_property("cantidad", cantidad))
            .await
            .unwrap();
        (graph, a, b)
    }

    #[tokio::test]
    async fn test_one_hop_multiply() {
        let (graph, a, b) = graph_with_edge(PropertyValue::Float(18.0)).await;

        let r = graph
            .traverse(a)
            .sack(10.0)
            .out_e("ContieneComponente")
            .sack_mul_by("cantidad")
            .emit()
            .await
            .unwrap();

        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].node, b);
        assert_eq!(r.items[0].sack, 180.0);
        assert_eq!(r.items[0].depth, 1);
        assert!(r.is_complete());
        assert_eq!(r.cycles_skipped, 0);
    }

    #[tokio::test]
    async fn test_int_property_coercion() {
        let (graph, a, _) = graph_with_edge(PropertyValue::Int(18)).await;

        let r = graph
            .traverse(a)
            .sack(1.0)
            .out_e("ContieneComponente")
            .sack_mul_by("cantidad")
            .emit()
            .await
            .unwrap();

        assert_eq!(r.items[0].sack, 18.0);
    }

    #[tokio::test]
    async fn test_missing_property_is_hard_error() {
        let (graph, a, _) = graph_with_edge(PropertyValue::Float(18.0)).await;

        let err = graph
            .traverse(a)
            .sack(1.0)
            .out_e("ContieneComponente")
            .sack_mul_by("merma")
            .emit()
            .await
            .unwrap_err();

        let msg = err.to_string();
        assert!(msg.contains("sack_mul_by"), "mensaje: {msg}");
        assert!(msg.contains("'merma'"), "mensaje: {msg}");
        assert!(msg.contains("ContieneComponente"), "mensaje: {msg}");
    }

    #[tokio::test]
    async fn test_fold_without_edge_step_is_config_error() {
        let (graph, a, _) = graph_with_edge(PropertyValue::Float(18.0)).await;

        let err = graph
            .traverse(a)
            .sack(1.0)
            .sack_mul_by("cantidad")
            .emit()
            .await
            .unwrap_err();

        assert!(err.to_string().contains("sack_mul_by"));
    }

    #[tokio::test]
    async fn test_prefix_steps_convert_before_sack() {
        // Los pasos grabados antes de .sack() corren como prefijo, y el
        // pliegue posterior ve la arista que ellos siguieron.
        let (graph, a, b) = graph_with_edge(PropertyValue::Float(18.0)).await;

        let r = graph
            .traverse(a)
            .out_e("ContieneComponente")
            .sack(10.0)
            .sack_mul_by("cantidad")
            .emit()
            .await
            .unwrap();

        assert_eq!(r.items.len(), 1);
        assert_eq!(r.items[0].node, b);
        assert_eq!(r.items[0].sack, 180.0);
    }
}
