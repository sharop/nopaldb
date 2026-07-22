// src/query/nql/executor/operators.rs

use std::sync::Arc;
use std::collections::{HashMap, VecDeque};
use crate::error::{NopalError, Result};
use crate::types::{Node, Edge, PropertyValue};
use crate::graph::Graph;
use crate::query::nql::parser::ast::{Expression, BinaryOperator};
use crate::query::nql::executor::result::Row;
use futures::future::BoxFuture;

// ================================================================================================
// STREAMING TRAITS (Phase 2.5 — Volcano Model)
// ================================================================================================

/// A stream of nodes, lazily produced.
pub trait NodeStream: Send + Sync {
    fn next<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<Node>>>;
}

/// A stream of query result rows, lazily produced.
pub trait RowStream: Send + Sync {
    fn next<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<Row>>>;
}

/// A stream of graph pattern matches (source, target, optional edge).
pub trait PatternMatchStream: Send + Sync {
    fn next<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<PatternMatch>>>;
}

// ================================================================================================
// NODE OPERATORS
// ================================================================================================

/// Scanner that produces nodes lazily from storage in bounded batches.
pub struct ScanNodesStream {
    graph: Arc<Graph>,
    label: Option<String>,
    batch_size: usize,
    buffer: VecDeque<Node>,
    cursor: Option<String>,
    exhausted: bool,
}

impl ScanNodesStream {
    pub fn new(graph: Arc<Graph>, label: Option<String>, batch_size: usize) -> Self {
        Self {
            graph,
            label,
            batch_size: batch_size.max(1),
            buffer: VecDeque::new(),
            cursor: None,
            exhausted: false,
        }
    }
}

impl NodeStream for ScanNodesStream {
    fn next<'a>(&'a mut self) -> BoxFuture<'a, Result<Option<Node>>> {
        Box::pin(async move {
            if let Some(node) = self.buffer.pop_front() {
                return Ok(Some(node));
            }

            if self.exhausted {
                return Ok(None);
            }

            let (nodes, next_cursor) = self.graph
                .scan_nodes_batch(
                    self.label.as_deref(),
                    self.cursor.as_deref(),
                    self.batch_size,
                )
                .await?;

            self.cursor = next_cursor;
            if self.cursor.is_none() {
                self.exhausted = true;
            }

            if nodes.is_empty() {
                self.exhausted = true;
                return Ok(None);
            }

            self.buffer.extend(nodes);
            Ok(self.buffer.pop_front())
        })
    }
}

/// Filter operator that passes nodes through a predicate.
pub struct FilterNodesStream<'a, F>
where
    F: Fn(&Node) -> Result<bool> + Send + Sync,
{
    input: Box<dyn NodeStream + 'a>,
    predicate: F,
}

impl<'a, F> FilterNodesStream<'a, F>
where
    F: Fn(&Node) -> Result<bool> + Send + Sync,
{
    pub fn new(input: Box<dyn NodeStream + 'a>, predicate: F) -> Self {
        Self { input, predicate }
    }
}

impl<'a, F> NodeStream for FilterNodesStream<'a, F>
where
    F: Fn(&Node) -> Result<bool> + Send + Sync,
{
    fn next<'b>(&'b mut self) -> BoxFuture<'b, Result<Option<Node>>> {
        Box::pin(async move {
            while let Some(node) = self.input.next().await? {
                if (self.predicate)(&node)? {
                    return Ok(Some(node));
                }
            }
            Ok(None)
        })
    }
}

// ================================================================================================
// ROW OPERATORS
// ================================================================================================

/// Projection operator for specific properties.
pub struct ProjectNodesStream<'a> {
    input: Box<dyn NodeStream + 'a>,
    variable: String,
    projections: Vec<String>,
}

impl<'a> ProjectNodesStream<'a> {
    pub fn new(input: Box<dyn NodeStream + 'a>, variable: String, projections: Vec<String>) -> Self {
        Self { input, variable, projections }
    }
}

impl<'a> RowStream for ProjectNodesStream<'a> {
    fn next<'b>(&'b mut self) -> BoxFuture<'b, Result<Option<Row>>> {
        Box::pin(async move {
            if let Some(node) = self.input.next().await? {
                let mut row = Row::new();
                for proj in &self.projections {
                    let parts: Vec<&str> = proj.split('.').collect();
                    if parts.len() == 2 && parts[0] == self.variable {
                        let prop = parts[1];
                        if prop.is_empty() {
                            // `find n from (n)` — bare variable, devolver id del nodo
                            row.set(proj, PropertyValue::String(node.id.to_string()));
                        } else if prop == "label" {
                             row.set(proj, PropertyValue::String(node.label.clone()));
                        } else if prop == "id" {
                             row.set(proj, PropertyValue::String(node.id.to_string()));
                        } else if let Some(val) = node.properties.get(prop) {
                             row.set(proj, val.clone());
                        }
                    } else if parts.len() == 1 && parts[0] == self.variable {
                         row.set(proj, PropertyValue::String(node.id.to_string()));
                    }
                }
                Ok(Some(row))
            } else {
                Ok(None)
            }
        })
    }
}

/// Projection operator for wildcard (*).
pub struct ProjectWildcardStream<'a> {
    input: Box<dyn NodeStream + 'a>,
    variable: String,
}

impl<'a> ProjectWildcardStream<'a> {
    pub fn new(input: Box<dyn NodeStream + 'a>, variable: String) -> Self {
        Self { input, variable }
    }
}

impl<'a> RowStream for ProjectWildcardStream<'a> {
    fn next<'b>(&'b mut self) -> BoxFuture<'b, Result<Option<Row>>> {
        Box::pin(async move {
            if let Some(node) = self.input.next().await? {
                let mut row = Row::new();
                for (key, val) in &node.properties {
                    row.set(format!("{}.{}", self.variable, key), val.clone());
                }
                row.set(format!("{}.label", self.variable), PropertyValue::String(node.label.clone()));
                row.set(format!("{}.id", self.variable), PropertyValue::String(node.id.to_string()));
                Ok(Some(row))
            } else {
                Ok(None)
            }
        })
    }
}

// ================================================================================================
// PATTERN OPERATORS
// ================================================================================================

// ================================================================================================
// COMPATIBILITY LAYER / HELPERS
// ================================================================================================

/// Scan nodes and return a stream. Used by Executor::execute_from_stream.
pub async fn scan_nodes_stream<'a>(
    graph: &'a Graph,
    label: Option<&str>,
) -> Result<Box<dyn NodeStream + 'a>> {
    let stream = ScanNodesStream::new(
        Arc::new(graph.clone()),
        label.map(|s| s.to_string()),
        512,
    );
    Ok(Box::new(stream))
}

/// Legacy materialized pattern matcher used by WriteExecutor.
pub async fn execute_pattern(
    graph: &Graph,
    source_label: Option<&str>,
    rel_type: Option<&str>,
    target_label: Option<&str>,
) -> Result<Vec<PatternMatch>> {
    // Implement via streaming for consistency
    let source_stream = Box::new(ScanNodesStream::new(Arc::new(graph.clone()), source_label.map(|s| s.to_string()), 512));
    let mut traversal = TraverseStream::new(
        graph,
        source_stream,
        rel_type.map(|s| s.to_string()),
        target_label.map(|s| s.to_string()),
        HashMap::new(),
    );
    
    let mut matches = Vec::new();
    while let Some(m) = traversal.next().await? {
        matches.push(m);
    }
    Ok(matches)
}

#[derive(Clone, Debug)]
pub struct PatternMatch {
    pub source: Node,
    pub target: Node,
    pub edge: Option<Edge>,
}

pub struct TraverseStream<'a> {
    graph: &'a Graph,
    source_stream: Box<dyn NodeStream + 'a>,
    rel_type: Option<String>,
    target_label: Option<String>,
    /// Filtros inline sobre propiedades del nodo destino, e.g. `(b {active: true})`.
    target_properties: HashMap<String, PropertyValue>,
    /// Filtros inline sobre propiedades de la arista, e.g. `-[r:TRANS {amount: 1000}]->`.
    /// Empty map significa sin filtro adicional.
    edge_properties: HashMap<String, PropertyValue>,
    current_source: Option<Node>,
    edge_iter: Option<std::vec::IntoIter<Edge>>,
}

impl<'a> TraverseStream<'a> {
    pub fn new(
        graph: &'a Graph,
        source_stream: Box<dyn NodeStream + 'a>,
        rel_type: Option<String>,
        target_label: Option<String>,
        target_properties: HashMap<String, PropertyValue>,
    ) -> Self {
        Self {
            graph,
            source_stream,
            rel_type,
            target_label,
            target_properties,
            edge_properties: HashMap::new(),
            current_source: None,
            edge_iter: None,
        }
    }

    /// Configura filtros inline sobre propiedades de arista.
    pub fn with_edge_properties(mut self, props: HashMap<String, PropertyValue>) -> Self {
        self.edge_properties = props;
        self
    }
}

impl<'a> PatternMatchStream for TraverseStream<'a> {
    fn next<'b>(&'b mut self) -> BoxFuture<'b, Result<Option<PatternMatch>>> {
        Box::pin(async move {
            loop {
                if let Some(iter) = &mut self.edge_iter
                    && let Some(edge) = iter.next()
                {
                    if let Some(rt) = &self.rel_type && edge.edge_type != *rt { continue; }
                    // Filtrar por propiedades inline de la arista, e.g. -[r:TRANS {amount: 1000}]->
                    if !self.edge_properties.is_empty()
                        && self.edge_properties.iter().any(|(k, v)| edge.properties.get(k) != Some(v))
                    {
                        continue;
                    }
                    if let Ok(target) = self.graph.get_node(edge.target).await {
                        if let Some(tl) = &self.target_label && target.label != *tl { continue; }
                        // Filtrar por propiedades inline del nodo destino, e.g. (b {active: true})
                        if !self.target_properties.is_empty()
                            && self.target_properties.iter().any(|(k, v)| target.properties.get(k) != Some(v))
                        {
                            continue;
                        }
                        let source = self.current_source.as_ref().ok_or_else(|| {
                            NopalError::query_error(
                                "TraverseStream invariant violated: missing current_source",
                            )
                        })?;
                        return Ok(Some(PatternMatch {
                            source: source.clone(),
                            target,
                            edge: Some(edge),
                        }));
                    }
                    continue;
                }
                if let Some(source) = self.source_stream.next().await? {
                    let edges = self.graph.get_outgoing_edges(source.id).await?;
                    self.current_source = Some(source);
                    self.edge_iter = Some(edges.into_iter());
                } else {
                    return Ok(None);
                }
            }
        })
    }
}

pub struct FilterPatternStream<'a, F>
where
    F: Fn(&PatternMatch) -> Result<bool> + Send + Sync,
{
    input: Box<dyn PatternMatchStream + 'a>,
    predicate: F,
}

impl<'a, F> FilterPatternStream<'a, F>
where
    F: Fn(&PatternMatch) -> Result<bool> + Send + Sync,
{
    pub fn new(input: Box<dyn PatternMatchStream + 'a>, predicate: F) -> Self {
        Self { input, predicate }
    }
}

impl<'a, F> PatternMatchStream for FilterPatternStream<'a, F>
where
    F: Fn(&PatternMatch) -> Result<bool> + Send + Sync,
{
    fn next<'b>(&'b mut self) -> BoxFuture<'b, Result<Option<PatternMatch>>> {
        Box::pin(async move {
            while let Some(m) = self.input.next().await? {
                if (self.predicate)(&m)? {
                    return Ok(Some(m));
                }
            }
            Ok(None)
        })
    }
}

pub struct ProjectPatternStream<'a> {
    input: Box<dyn PatternMatchStream + 'a>,
    source_var: String,
    target_var: String,
    edge_var: Option<String>,
    projections: Vec<String>,
}

impl<'a> ProjectPatternStream<'a> {
    pub fn new(input: Box<dyn PatternMatchStream + 'a>, source_var: String, target_var: String, edge_var: Option<String>, projections: Vec<String>) -> Self {
        Self { input, source_var, target_var, edge_var, projections }
    }
}

impl<'a> RowStream for ProjectPatternStream<'a> {
    fn next<'b>(&'b mut self) -> BoxFuture<'b, Result<Option<Row>>> {
        Box::pin(async move {
            if let Some(m) = self.input.next().await? {
                let mut row = Row::new();
                for proj in &self.projections {
                    if proj == "*" {
                        for (k, v) in &m.source.properties { row.set(format!("{}.{}", self.source_var, k), v.clone()); }
                        for (k, v) in &m.target.properties { row.set(format!("{}.{}", self.target_var, k), v.clone()); }
                        row.set(format!("{}.label", self.source_var), PropertyValue::String(m.source.label.clone()));
                        row.set(format!("{}.id", self.source_var), PropertyValue::String(m.source.id.to_string()));
                        row.set(format!("{}.label", self.target_var), PropertyValue::String(m.target.label.clone()));
                        row.set(format!("{}.id", self.target_var), PropertyValue::String(m.target.id.to_string()));
                        if let (Some(ev), Some(e)) = (&self.edge_var, &m.edge) {
                            for (k, v) in &e.properties { row.set(format!("{}.{}", ev, k), v.clone()); }
                            row.set(format!("{}.type", ev), PropertyValue::String(e.edge_type.clone()));
                            row.set(format!("{}.id", ev), PropertyValue::String(e.id.to_string()));
                        }
                        continue;
                    }
                    let parts: Vec<&str> = proj.split('.').collect();
                    if parts.len() == 2 {
                        let var = parts[0];
                        let prop = parts[1];
                        if prop == "*" {
                            if var == self.source_var {
                                for (k, v) in &m.source.properties { row.set(format!("{}.{}", self.source_var, k), v.clone()); }
                                row.set(format!("{}.label", self.source_var), PropertyValue::String(m.source.label.clone()));
                                row.set(format!("{}.id", self.source_var), PropertyValue::String(m.source.id.to_string()));
                            } else if var == self.target_var {
                                for (k, v) in &m.target.properties { row.set(format!("{}.{}", self.target_var, k), v.clone()); }
                                row.set(format!("{}.label", self.target_var), PropertyValue::String(m.target.label.clone()));
                                row.set(format!("{}.id", self.target_var), PropertyValue::String(m.target.id.to_string()));
                            } else if let Some(ev) = &self.edge_var
                                && ev == var
                                && let Some(e) = &m.edge
                            {
                                for (k, v) in &e.properties { row.set(format!("{}.{}", ev, k), v.clone()); }
                                row.set(format!("{}.type", ev), PropertyValue::String(e.edge_type.clone()));
                                row.set(format!("{}.id", ev), PropertyValue::String(e.id.to_string()));
                            }
                        } else if var == self.source_var {
                            if prop.is_empty() { row.set(proj, PropertyValue::String(m.source.id.to_string())); }
                            else if prop == "label" { row.set(proj, PropertyValue::String(m.source.label.clone())); }
                            else if prop == "id" { row.set(proj, PropertyValue::String(m.source.id.to_string())); }
                            else if let Some(v) = m.source.properties.get(prop) { row.set(proj, v.clone()); }
                        } else if var == self.target_var {
                            if prop.is_empty() { row.set(proj, PropertyValue::String(m.target.id.to_string())); }
                            else if prop == "label" { row.set(proj, PropertyValue::String(m.target.label.clone())); }
                            else if prop == "id" { row.set(proj, PropertyValue::String(m.target.id.to_string())); }
                            else if let Some(v) = m.target.properties.get(prop) { row.set(proj, v.clone()); }
                        } else if let Some(ev) = &self.edge_var
                            && ev == var
                            && let Some(e) = &m.edge
                        {
                            if prop.is_empty() { row.set(proj, PropertyValue::String(e.id.to_string())); }
                            else if prop == "type" { row.set(proj, PropertyValue::String(e.edge_type.clone())); }
                            else if prop == "id" { row.set(proj, PropertyValue::String(e.id.to_string())); }
                            else if let Some(v) = e.properties.get(prop) { row.set(proj, v.clone()); }
                        }
                    } else if parts.len() == 1 {
                        let var = parts[0];
                        if var == self.source_var { row.set(proj, PropertyValue::String(m.source.id.to_string())); }
                        else if var == self.target_var { row.set(proj, PropertyValue::String(m.target.id.to_string())); }
                        else if let Some(ev) = &self.edge_var
                            && ev == var
                            && let Some(e) = &m.edge
                        {
                            row.set(proj, PropertyValue::String(e.id.to_string()));
                        }
                    }
                }
                Ok(Some(row))
            } else {
                Ok(None)
            }
        })
    }
}

// ================================================================================================
// EVALUATORS
// ================================================================================================

pub fn eval_condition(node: &Node, expr: &Expression, variable: &str) -> bool {
    match expr {
        Expression::BinaryOp { left, op, right } => match op {
            BinaryOperator::And => eval_condition(node, left, variable) && eval_condition(node, right, variable),
            BinaryOperator::Or => eval_condition(node, left, variable) || eval_condition(node, right, variable),
            _ => {
                let lv = eval_expression(node, left, variable);
                let rv = eval_expression(node, right, variable);
                match (lv, rv) {
                    (Some(l), Some(r)) => compare_values(&l, op, &r),
                    _ => false,
                }
            }
        },
        // Las ontology predicates (instanceOf/subClassOf) requieren acceso al grafo.
        // Sin él, retornar false para evitar que todos los nodos pasen el filtro.
        Expression::FunctionCall { name, .. }
            if matches!(name.to_lowercase().as_str(), "instanceof" | "subclassof") =>
        {
            false
        }
        _ => true,
    }
}

pub fn eval_expression(node: &Node, expr: &Expression, expected_var: &str) -> Option<PropertyValue> {
    match expr {
        Expression::Literal(val) => Some(val.clone()),
        Expression::Property { variable, property } => {
            if variable != expected_var { return None; }
            if property.is_empty() { return Some(PropertyValue::String(node.id.to_string())); }
            if property == "label" { return Some(PropertyValue::String(node.label.clone())); }
            if property == "id" { return Some(PropertyValue::String(node.id.to_string())); }
            node.properties.get(property).cloned()
        },
        _ => None,
    }
}

pub fn eval_pattern_condition(m: &PatternMatch, expr: &Expression, source_var: &str, target_var: &str) -> bool {
    match expr {
        Expression::BinaryOp { left, op, right } => match op {
            BinaryOperator::And => eval_pattern_condition(m, left, source_var, target_var) && eval_pattern_condition(m, right, source_var, target_var),
            BinaryOperator::Or => eval_pattern_condition(m, left, source_var, target_var) || eval_pattern_condition(m, right, source_var, target_var),
            _ => {
                let lv = eval_pattern_expression(m, left, source_var, target_var);
                let rv = eval_pattern_expression(m, right, source_var, target_var);
                match (lv, rv) {
                    (Some(l), Some(r)) => compare_values(&l, op, &r),
                    _ => false,
                }
            }
        },
        _ => true,
    }
}

pub fn eval_pattern_expression(m: &PatternMatch, expr: &Expression, source_var: &str, target_var: &str) -> Option<PropertyValue> {
    match expr {
        Expression::Literal(val) => Some(val.clone()),
        Expression::Property { variable, property } => {
            if property.is_empty() {
                if variable == source_var { return Some(PropertyValue::String(m.source.id.to_string())); }
                if variable == target_var { return Some(PropertyValue::String(m.target.id.to_string())); }
                if let Some(e) = &m.edge { return Some(PropertyValue::String(e.id.to_string())); }
                return None;
            }
            let node = if variable == source_var { &m.source } else if variable == target_var { &m.target } else {
                if let Some(e) = &m.edge {
                    if property == "type" || property == "edge_type" { return Some(PropertyValue::String(e.edge_type.clone())); }
                    if property == "id" { return Some(PropertyValue::String(e.id.to_string())); }
                    return e.properties.get(property).cloned();
                }
                return None;
            };
            if property == "label" { return Some(PropertyValue::String(node.label.clone())); }
            if property == "id" { return Some(PropertyValue::String(node.id.to_string())); }
            node.properties.get(property).cloned()
        },
        _ => None,
    }
}

pub fn compare_values(left: &PropertyValue, op: &BinaryOperator, right: &PropertyValue) -> bool {
    match op {
        BinaryOperator::Eq => left == right,
        BinaryOperator::NotEq => left != right,
        BinaryOperator::Gt => left > right,
        BinaryOperator::Lt => left < right,
        BinaryOperator::GtEq => left >= right,
        BinaryOperator::LtEq => left <= right,
        _ => false,
    }
}

pub fn filter_stream_from_expr<'a>(input: Box<dyn NodeStream + 'a>, expr: Arc<Expression>, variable: String) -> Box<dyn NodeStream + 'a> {
    Box::new(FilterNodesStream::new(input, move |node| Ok(eval_condition(node, &expr, &variable))))
}

/// Graph-aware variant of `eval_condition` that resolves ontology predicates
/// (`instanceOf`, `subClassOf`) via TaxonomyIndex and embedding predicates
/// (`has_embedding`, `similar_to`) via the storage layer.
///
/// Available when `embeddings` OR `reasoner` is enabled. Each inner arm is
/// further gated on the specific feature it requires.
#[cfg(any(feature = "embeddings", feature = "reasoner"))]
pub fn eval_condition_with_graph(
    node: &Node,
    expr: &Expression,
    variable: &str,
    graph: &Graph,
) -> crate::error::Result<bool> {
    match expr {
        Expression::BinaryOp { left, op, right } => match op {
            BinaryOperator::And => {
                Ok(
                    eval_condition_with_graph(node, left, variable, graph)?
                        && eval_condition_with_graph(node, right, variable, graph)?,
                )
            }
            BinaryOperator::Or => {
                Ok(
                    eval_condition_with_graph(node, left, variable, graph)?
                        || eval_condition_with_graph(node, right, variable, graph)?,
                )
            }
            _ => Ok(eval_condition(node, expr, variable)),
        },
        #[cfg(feature = "embeddings")]
        Expression::FunctionCall { name, args } if name.to_lowercase() == "has_embedding" => {
            if args.len() != 2 {
                return Ok(false);
            }
            let model = match &args[1] {
                Expression::Literal(PropertyValue::String(s)) => s.as_str(),
                _ => return Ok(false),
            };
            graph.try_node_embedding_exists_sync(node.id, model)
        }
        // similar_to() is pre-computed via HNSW in the executor (precompute_similar_to).
        // The node already passed the set-membership filter, so return true here.
        #[cfg(feature = "embeddings")]
        Expression::FunctionCall { name, .. } if name.to_lowercase() == "similar_to" => Ok(true),
        // hybrid() is pre-computed via RRF in the executor (precompute_hybrid).
        // The node already passed the set-membership filter, so return true here.
        #[cfg(feature = "hybrid")]
        Expression::FunctionCall { name, .. } if name.to_lowercase() == "hybrid" => Ok(true),
        // instanceOf(var, "ClassName") / subClassOf(var, "ClassName") — requieren taxonomía
        #[cfg(feature = "reasoner")]
        Expression::FunctionCall { name, args }
            if matches!(name.to_lowercase().as_str(), "instanceof" | "subclassof") =>
        {
            if args.len() != 2 {
                return Ok(false);
            }
            let class_name = match &args[1] {
                Expression::Literal(PropertyValue::String(s)) => s.clone(),
                _ => return Ok(false),
            };
            let Some(mut tax) = graph.get_taxonomy_sync() else { return Ok(false); };
            let Some(parent_id) = tax.find_by_label(&class_name) else { return Ok(false); };
            let result = match name.to_lowercase().as_str() {
                "instanceof" => {
                    node.kind == crate::types::NodeKind::Individual
                        && tax.is_subclass_of_label(&node.label, parent_id)
                }
                "subclassof" => {
                    node.kind == crate::types::NodeKind::Class
                        && tax.is_subclass_of_label(&node.label, parent_id)
                }
                _ => false,
            };
            Ok(result)
        }
        _ => Ok(eval_condition(node, expr, variable)),
    }
}

/// Streaming filter that resolves graph-state predicates (`has_embedding`, `instanceOf`).
/// Available when `embeddings` OR `reasoner` is enabled.
#[cfg(any(feature = "embeddings", feature = "reasoner"))]
pub fn filter_stream_from_expr_with_graph<'a>(
    input: Box<dyn NodeStream + 'a>,
    expr: Arc<Expression>,
    variable: String,
    graph: Arc<Graph>,
) -> Box<dyn NodeStream + 'a> {
    Box::new(FilterNodesStream::new(input, move |node| {
        eval_condition_with_graph(node, &expr, &variable, &graph)
    }))
}

pub fn filter_pattern_stream_from_expr<'a>(input: Box<dyn PatternMatchStream + 'a>, expr: Arc<Expression>, source_var: String, target_var: String) -> Box<dyn PatternMatchStream + 'a> {
    Box::new(FilterPatternStream::new(input, move |m| Ok(eval_pattern_condition(m, &expr, &source_var, &target_var))))
}

// ================================================================================================
// LEGACY (REMOVED: Use streaming versions instead)
// ================================================================================================
