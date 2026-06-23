// src/query/nql/executor/mod.rs
//
// NQL Query Executor

pub mod aggregations;
pub mod export;
pub mod operators;
pub mod result;
pub mod write;

use operators::RowStream;

use crate::Transaction;
use crate::error::{NopalError, Result};
use crate::graph::Graph;
use crate::index::IndexType as GraphIndexType;
use crate::planner::{PlanNode, QueryPlanner};
use crate::query::nql::parser::ast::{
    AddStmt, BinaryOperator, CreateIndexStmt, DeleteStmt, Direction, DropIndexStmt, Expression,
    GroupByClause, IndexType, NodePattern, OrderByClause, Pattern, PatternElement, Projection,
    Quantifier, Query, RelationshipPattern, SortOrder, Statement, UnaryOperator, UpdateStmt,
    WhereClause,
};
use crate::query::nql::parser::{parse_vm_assignment, parse_vm_expression};
use crate::types::Node;
use crate::types::{Edge, NodeId, PropertyValue};
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

// Re-export result types
pub use result::{AddResult, DeleteResult, ProfileResult, QueryResult, Row, UpdateResult};
pub use write::{MatchedElement, WriteExecutor};

use aggregations::{
    execute_aggregations, has_aggregations, has_real_aggregations, lookup_algo_value,
    precompute_for_query,
};

/// Evaluate a WHERE-style boolean expression against an already-projected row,
/// using the algorithm cache to resolve `degree(e)`-style function calls.
///
/// Used for Bug 1 post-projection algorithm filtering. Supports the common
/// shapes: AND/OR/NOT, comparisons between properties/literals/algorithm
/// calls. Properties are looked up from the row by `<var>.<prop>` key.
fn eval_row_condition_with_algo(
    row: &result::Row,
    expr: &Expression,
    source_var: &str,
    target_var: &str,
    algo_cache: &aggregations::AlgoResults,
) -> bool {
    match expr {
        Expression::BinaryOp { left, op, right } => match op {
            BinaryOperator::And => {
                eval_row_condition_with_algo(row, left, source_var, target_var, algo_cache)
                    && eval_row_condition_with_algo(row, right, source_var, target_var, algo_cache)
            }
            BinaryOperator::Or => {
                eval_row_condition_with_algo(row, left, source_var, target_var, algo_cache)
                    || eval_row_condition_with_algo(row, right, source_var, target_var, algo_cache)
            }
            _ => {
                let l = eval_row_scalar_with_algo(row, left, source_var, target_var, algo_cache);
                let r = eval_row_scalar_with_algo(row, right, source_var, target_var, algo_cache);
                match (l, r) {
                    (Some(l), Some(r)) => operators::compare_values(&l, op, &r),
                    _ => false,
                }
            }
        },
        Expression::UnaryOp {
            op: UnaryOperator::Not,
            expr: inner,
        } => !eval_row_condition_with_algo(row, inner, source_var, target_var, algo_cache),
        _ => true,
    }
}

fn eval_row_scalar_with_algo(
    row: &result::Row,
    expr: &Expression,
    source_var: &str,
    target_var: &str,
    algo_cache: &aggregations::AlgoResults,
) -> Option<PropertyValue> {
    match expr {
        Expression::Literal(v) => Some(v.clone()),
        Expression::Property { variable, property } => {
            if property.is_empty() {
                let id_key = format!("{}.id", variable);
                row.values.get(&id_key).cloned()
            } else {
                let key = format!("{}.{}", variable, property);
                row.values.get(&key).cloned()
            }
        }
        Expression::FunctionCall { name, args } if expr.is_algorithm() => {
            let var = match args.first() {
                Some(Expression::Property { variable, property }) if property.is_empty() => {
                    variable.clone()
                }
                _ => return None,
            };
            let _ = (source_var, target_var); // not needed: we use row-stored ids
            let id_key = format!("{}.id", var);
            if let Some(PropertyValue::String(id_str)) = row.values.get(&id_key)
                && let Ok(node_id) = uuid::Uuid::parse_str(id_str)
            {
                Some(lookup_algo_value(name, &node_id, algo_cache))
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Returns true if the expression references any algorithm function
/// (degree, pagerank, betweenness, clustering, community, leiden, etc.).
fn expr_contains_algorithm_function(expr: &Expression) -> bool {
    match expr {
        Expression::FunctionCall { args, .. } => {
            if expr.is_algorithm() {
                return true;
            }
            args.iter().any(expr_contains_algorithm_function)
        }
        Expression::BinaryOp { left, right, .. } => {
            expr_contains_algorithm_function(left) || expr_contains_algorithm_function(right)
        }
        Expression::UnaryOp { expr, .. } => expr_contains_algorithm_function(expr),
        _ => false,
    }
}

/// Strip algorithm predicates from a WHERE-style boolean expression.
///
/// Returns the residual expression with algo-touching subtrees removed.
/// Conservative: only handles AND chains. For complex expressions (OR, NOT
/// containing algos), returns None so the caller can defer the entire
/// filter to post-projection.
fn strip_algorithm_predicates(expr: &Expression) -> Option<Expression> {
    if !expr_contains_algorithm_function(expr) {
        return Some(expr.clone());
    }
    match expr {
        Expression::BinaryOp { left, op, right } if matches!(op, BinaryOperator::And) => {
            let l = strip_algorithm_predicates(left);
            let r = strip_algorithm_predicates(right);
            match (l, r) {
                (Some(l), Some(r)) => Some(Expression::BinaryOp {
                    left: Box::new(l),
                    op: op.clone(),
                    right: Box::new(r),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            }
        }
        // The expression itself is an algo predicate at top level.
        _ => None,
    }
}

/// Extract ONLY the algorithm-bearing subexpressions, AND-combined.
///
/// Inverse of `strip_algorithm_predicates`. Used as the post-projection
/// filter: only the algo predicates are re-evaluated after rows are
/// projected (the non-algo predicates already filtered the stream).
fn extract_algorithm_predicates(expr: &Expression) -> Option<Expression> {
    if !expr_contains_algorithm_function(expr) {
        return None;
    }
    match expr {
        Expression::BinaryOp { left, op, right } if matches!(op, BinaryOperator::And) => {
            let l = extract_algorithm_predicates(left);
            let r = extract_algorithm_predicates(right);
            match (l, r) {
                (Some(l), Some(r)) => Some(Expression::BinaryOp {
                    left: Box::new(l),
                    op: op.clone(),
                    right: Box::new(r),
                }),
                (Some(only), None) | (None, Some(only)) => Some(only),
                (None, None) => None,
            }
        }
        // Top-level algo predicate (e.g. `degree(e) > 3` directly).
        _ => Some(expr.clone()),
    }
}

fn is_path_reducer(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "path_sum" | "path_min" | "path_max" | "path_avg"
    )
}

fn expr_contains_path_reducer(expr: &Expression) -> bool {
    match expr {
        Expression::FunctionCall { name, .. } => is_path_reducer(name),
        Expression::BinaryOp { left, right, .. } => {
            expr_contains_path_reducer(left) || expr_contains_path_reducer(right)
        }
        Expression::UnaryOp { expr, .. } => expr_contains_path_reducer(expr),
        _ => false,
    }
}

fn projections_contain_path_reducer(projections: &[Projection]) -> bool {
    projections.iter().any(|p| match p {
        Projection::Expression {
            expr: Expression::FunctionCall { name, .. },
            ..
        } => is_path_reducer(name),
        _ => false,
    })
}

fn is_path_eval(name: &str) -> bool {
    name.eq_ignore_ascii_case("path_eval")
}

fn is_path_semantic_filter(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "path_start_instanceof"
            | "path_end_instanceof"
            | "path_any_instanceof"
            | "path_all_instanceof"
            | "path_start_subclassof"
            | "path_end_subclassof"
            | "path_any_subclassof"
            | "path_all_subclassof"
    )
}

fn expr_contains_path_semantic_filter(expr: &Expression) -> bool {
    match expr {
        Expression::FunctionCall { name, args } => {
            is_path_semantic_filter(name) || args.iter().any(expr_contains_path_semantic_filter)
        }
        Expression::BinaryOp { left, right, .. } => {
            expr_contains_path_semantic_filter(left) || expr_contains_path_semantic_filter(right)
        }
        Expression::UnaryOp { expr, .. } => expr_contains_path_semantic_filter(expr),
        _ => false,
    }
}

fn is_path_embedding_fn(name: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "path_has_embeddings"
            | "path_embedding"
            | "pattern_has_embeddings"
            | "path_embedding_similarity"
            | "path_knn_references"
            | "path_anomaly_score"
            | "pattern_embedding"
            | "pattern_embedding_similarity"
    )
}

fn expr_contains_path_embedding_fn(expr: &Expression) -> bool {
    match expr {
        Expression::FunctionCall { name, args } => {
            is_path_embedding_fn(name) || args.iter().any(expr_contains_path_embedding_fn)
        }
        Expression::BinaryOp { left, right, .. } => {
            expr_contains_path_embedding_fn(left) || expr_contains_path_embedding_fn(right)
        }
        Expression::UnaryOp { expr, .. } => expr_contains_path_embedding_fn(expr),
        _ => false,
    }
}

/// Verifica recursivamente si una expresión usa `path.<prop>` (F4-C).
fn expr_uses_path_property_exec(expr: &Expression, prop: &str) -> bool {
    match expr {
        Expression::Property { variable, property } => variable == "path" && property == prop,
        Expression::BinaryOp { left, right, .. } => {
            expr_uses_path_property_exec(left, prop) || expr_uses_path_property_exec(right, prop)
        }
        Expression::UnaryOp { expr, .. } => expr_uses_path_property_exec(expr, prop),
        Expression::FunctionCall { args, .. } => {
            args.iter().any(|a| expr_uses_path_property_exec(a, prop))
        }
        _ => false,
    }
}

/// Construye un PropertyValue::Object con `id` y `label` del nodo (F4-C).
fn build_path_node_object(node: &Node) -> PropertyValue {
    PropertyValue::Object(vec![
        ("id".to_string(), PropertyValue::String(node.id.to_string())),
        (
            "label".to_string(),
            PropertyValue::String(node.label.clone()),
        ),
    ])
}

/// Cosine similarity entre dos vectores de f32 (E-8).
/// Falla explicitamente si los vectores estan vacios, tienen dimensiones
/// distintas o alguna norma cero.
#[cfg(feature = "embeddings")]
fn cosine_similarity_f32(a: &[f32], b: &[f32]) -> Result<f32> {
    if a.len() != b.len() {
        return Err(NopalError::QueryExecutionError(format!(
            "PathSimilarity E-8 requires vectors with the same dimension, got {} and {}",
            a.len(),
            b.len()
        )));
    }
    if a.is_empty() {
        return Err(NopalError::QueryExecutionError(
            "PathSimilarity E-8 cannot compare empty vectors".into(),
        ));
    }
    let dot: f32 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let norm_a: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let norm_b: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm_a == 0.0 || norm_b == 0.0 {
        return Err(NopalError::QueryExecutionError(
            "PathSimilarity E-8 cannot compare zero-norm vectors".into(),
        ));
    }
    Ok(dot / (norm_a * norm_b))
}

fn expr_contains_path_eval(expr: &Expression) -> bool {
    match expr {
        Expression::FunctionCall { name, args } => {
            is_path_eval(name) || args.iter().any(expr_contains_path_eval)
        }
        Expression::BinaryOp { left, right, .. } => {
            expr_contains_path_eval(left) || expr_contains_path_eval(right)
        }
        Expression::UnaryOp { expr, .. } => expr_contains_path_eval(expr),
        _ => false,
    }
}

fn projections_contain_path_eval(projections: &[Projection]) -> bool {
    projections.iter().any(|p| match p {
        Projection::Expression { expr, .. } => expr_contains_path_eval(expr),
        _ => false,
    })
}

fn property_projection_key(variable: &str, property: &str) -> String {
    if property.is_empty() {
        variable.to_string()
    } else {
        format!("{}.{}", variable, property)
    }
}

fn projection_literal_key(value: &PropertyValue) -> Option<String> {
    match value {
        PropertyValue::String(s) => Some(format!(
            "\"{}\"",
            s.replace('\\', "\\\\").replace('"', "\\\"")
        )),
        PropertyValue::Int(i) => Some(i.to_string()),
        PropertyValue::Float(f) => Some(f.to_string()),
        PropertyValue::Bool(b) => Some(b.to_string()),
        PropertyValue::Null => Some("null".to_string()),
        _ => None,
    }
}

fn path_embedding_projection_key(name: &str, args: &[Expression]) -> Result<String> {
    let rendered_args: Result<Vec<String>> = args
        .iter()
        .map(|arg| match arg {
            Expression::Literal(value) => projection_literal_key(value).ok_or_else(|| {
                NopalError::QueryExecutionError(format!(
                    "{} uses unsupported projection literal {:?} in path/pattern embedding projections",
                    name, value
                ))
            }),
            other => Err(NopalError::QueryExecutionError(format!(
                "{} requires literal arguments for canonical projection keys; got {:?}",
                name, other
            ))),
        })
        .collect();

    Ok(format!("{}({})", name, rendered_args?.join(", ")))
}

//Uso de Lifetimes para garantizar memory safe
/// Query Executor
pub struct Executor<'a> {
    graph: &'a Graph,
    path_profile: Mutex<Option<PathProfileCounters>>,
}

#[derive(Clone, Debug)]
struct LinearPatternBinding {
    nodes: Vec<Node>,
    edges: Vec<Edge>,
    node_vars: HashMap<String, Node>,
    edge_vars: HashMap<String, Edge>,
}

/// Estado de frontera para BFS acotado en rutas cuantificadas (F1).
struct FrontierState {
    binding: LinearPatternBinding,
    current_node: Node,
    depth: usize,
    visited: HashSet<NodeId>,
}

#[derive(Clone, Debug, Default)]
struct PathProfileCounters {
    bindings_examined: usize,
    bindings_emitted: usize,
    frontier_states_visited: usize,
    cycle_prunes: usize,
    max_depth_observed: usize,
}

#[derive(Clone, Debug)]
struct PathMetadata {
    depth: usize,
    nodes: PropertyValue,
    edges: PropertyValue,
}

#[derive(Clone, Debug, Default)]
struct PathVmState {
    vars: HashMap<String, PropertyValue>,
    return_result: Option<PropertyValue>, // F4-C: result of RETURN clause
}

struct PathVmHopContext<'a> {
    source: Option<&'a Node>,
    target: Option<&'a Node>,
    edge: Option<&'a Edge>,
    path_depth: usize,
    binding: Option<&'a LinearPatternBinding>, // F4-C: for final context (return/FIND/WHERE)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PathPropertyKind {
    Depth,
    Nodes,
    Edges,
    Start,  // F4-C
    End,    // F4-C
    State,  // F4-C
    Result, // F4-C
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExecutionMode {
    FastTraverse,
    LinearBindings,
}

impl<'a> Executor<'a> {
    pub fn new(graph: &'a Graph) -> Self {
        Executor {
            graph,
            path_profile: Mutex::new(None),
        }
    }

    /// Execute NQL query
    pub async fn execute(&self, query: Query) -> Result<QueryResult> {
        // F4-C: Run semantic validation (path.result, return rules, etc.)
        {
            use crate::query::nql::parser::ast::Statement;
            use crate::query::nql::validator::SemanticValidator;
            SemanticValidator::new().validate(&Statement::Query(query.clone()))?;
        }
        self.reset_path_profile();
        self.validate_path_metadata_usage(&query)?;
        log::info!(
            "Executing NQL query with {} patterns",
            query.from.patterns.len()
        );
        let has_relationships = query.from.patterns.iter().any(|p| {
            p.elements
                .iter()
                .any(|e| matches!(e, PatternElement::Relationship(_)))
        });

        // ========================================
        // OPTIMIZATION: Try to use indexes
        // ========================================

        log::debug!("Checking if query can use index...");

        // Check if we can use an index for this query
        // (for now only single-node queries; relationship queries use dedicated pipeline).
        if !has_relationships && let Some(filter) = &query.filter {
            log::debug!("Query has WHERE clause, analyzing...");

            if let Some((variable, property, value)) = self.extract_indexed_condition(filter)? {
                log::info!(
                    "🔍 Found indexed condition candidate: {}.{}",
                    variable,
                    property
                );

                // Get label from FROM clause
                if !query.from.patterns.is_empty() {
                    let pattern = &query.from.patterns[0];
                    if !pattern.elements.is_empty()
                        && let PatternElement::Node(node_pattern) = &pattern.elements[0]
                    {
                        if let Some(label) = &node_pattern.label {
                            // Check if variable matches
                            if Some(&variable) == node_pattern.variable.as_ref() {
                                log::info!(
                                    "🚀 Attempting index lookup: {} on {}.{} = {:?}",
                                    label,
                                    label,
                                    property,
                                    value
                                );

                                // Try to use index
                                match self.graph.find_nodes_indexed(label, &property, value).await {
                                    Ok(nodes) if !nodes.is_empty() => {
                                        log::info!("✅ Index returned {} nodes", nodes.len());

                                        // Apply any remaining filters
                                        let filtered =
                                            self.apply_remaining_filters(nodes, &query)?;

                                        // P2: Inject ORDER BY extras
                                        let order_by_extras = self.extract_order_by_extras(&query);
                                        let mut result = self
                                            .project_result_with_extras(
                                                filtered,
                                                &query,
                                                &order_by_extras,
                                            )
                                            .await?;

                                        self.apply_distinct_if_needed(&mut result, &query.find);

                                        // Apply ORDER BY
                                        if let Some(order_by) = &query.order_by {
                                            self.apply_order_by(&mut result, order_by);
                                        }

                                        // Apply LIMIT/OFFSET (after ORDER BY)
                                        if let Some(limit) = &query.limit {
                                            let offset = limit.offset.unwrap_or(0);
                                            result.rows = result
                                                .rows
                                                .into_iter()
                                                .skip(offset)
                                                .take(limit.limit)
                                                .collect();
                                        }

                                        // Strip ORDER BY extras
                                        if !order_by_extras.is_empty() {
                                            self.strip_extra_columns(&mut result, &order_by_extras);
                                        }

                                        log::info!(
                                            "🎯 Returning {} results from index",
                                            result.len()
                                        );
                                        return Ok(result);
                                    }
                                    Ok(_) => {
                                        log::info!(
                                            "⚠️  Index returned 0 results, falling back to scan"
                                        );
                                    }
                                    Err(e) => {
                                        log::warn!(
                                            "❌ Index lookup failed: {}, falling back to scan",
                                            e
                                        );
                                    }
                                }
                            } else {
                                log::debug!(
                                    "Variable mismatch: {:?} != {:?}",
                                    Some(&variable),
                                    node_pattern.variable
                                );
                            }
                        } else {
                            log::debug!("Node pattern has no label");
                        }
                    }
                }
            } else {
                log::debug!("Could not extract indexed condition");
            }
        } else if !has_relationships {
            log::debug!("Query has no WHERE clause");
        } else {
            log::debug!("Skipping index fast-path for relationship query");
        }

        // ========================================
        // FALLBACK: Standard execution path
        // ========================================

        log::info!("📊 Using full scan (no index used)");

        if has_relationships {
            // Use pattern matching executor
            return self.execute_pattern_query(query).await;
        }

        // Step 1: Execute FROM clause (get nodes stream)
        let nodes_stream = self.execute_from_stream(&query).await?;

        // Resolve root variable for single-node queries.
        let root_variable = query
            .from
            .patterns
            .first()
            .and_then(|p| p.elements.first())
            .and_then(|e| match e {
                PatternElement::Node(n) => n.variable.as_deref().or(Some("n")),
                _ => None,
            })
            .unwrap_or("n");

        // Step 1.5: Pre-compute similar_to HNSW search if present in WHERE.
        // similar_to(n, "reference_name", "model") resolves via HNSW ANN search
        // BEFORE the streaming pipeline, producing a set of allowed NodeIds.
        #[cfg(feature = "embeddings-index")]
        let similar_to_set: Option<HashSet<crate::types::NodeId>> =
            if let Some(filter) = &query.filter {
                self.precompute_similar_to(&filter.condition, &query)
                    .await?
            } else {
                None
            };

        // Step 2: Apply WHERE filter (streaming - full).
        // When the `embeddings` feature is active we use a graph-aware variant that can
        // resolve `has_embedding(n, model)` predicates via synchronous storage access.
        let mut final_node_stream = nodes_stream;

        // If similar_to pre-computed a candidate set, inject a set-membership filter first.
        #[cfg(feature = "embeddings-index")]
        if let Some(ref allowed_set) = similar_to_set {
            let set = allowed_set.clone();
            final_node_stream = Box::new(operators::FilterNodesStream::new(
                final_node_stream,
                move |node| Ok(set.contains(&node.id)),
            ));
        }

        // Bug 1 fix: en single-node queries, separar WHERE algo-bearing
        // predicates (que necesitan algo cache) de los stream-time predicates.
        // Mismo enfoque que en el pattern path (execute_pattern_match).
        let single_where_post_filter: Option<Expression> = query
            .filter
            .as_ref()
            .and_then(|f| extract_algorithm_predicates(&f.condition));

        if let Some(filter) = &query.filter {
            // Si WHERE tiene algos, sólo pushdown el residuo non-algo.
            let stream_pred = if single_where_post_filter.is_some() {
                strip_algorithm_predicates(&filter.condition)
            } else {
                Some(filter.condition.clone())
            };
            if let Some(expr) = stream_pred {
                let condition = std::sync::Arc::new(expr);
                let rv = root_variable.to_string();
                #[cfg(any(feature = "embeddings", feature = "reasoner"))]
                {
                    let graph_arc = std::sync::Arc::new(self.graph.clone());
                    final_node_stream = operators::filter_stream_from_expr_with_graph(
                        final_node_stream,
                        condition,
                        rv,
                        graph_arc,
                    );
                }
                #[cfg(not(any(feature = "embeddings", feature = "reasoner")))]
                {
                    final_node_stream =
                        operators::filter_stream_from_expr(final_node_stream, condition, rv);
                }
            }
        }

        // Aggregations/group-by require materialized nodes. The streaming projection
        // path only supports property projections and will yield null/empty values for
        // function expressions if we don't branch here.
        if has_aggregations(&query.find.projections) || query.group_by.is_some() {
            let mut nodes = Vec::new();
            while let Some(node) = final_node_stream.next().await? {
                nodes.push(node);
            }

            let mut result = execute_aggregations(self.graph, nodes, &query, root_variable).await?;

            self.apply_distinct_if_needed(&mut result, &query.find);

            if let Some(order_by) = &query.order_by {
                self.apply_order_by(&mut result, order_by);
            }

            if let Some(limit) = &query.limit {
                let offset = limit.offset.unwrap_or(0);
                result.rows = result
                    .rows
                    .into_iter()
                    .skip(offset)
                    .take(limit.limit)
                    .collect();
            }

            return Ok(result);
        }

        // Step 3: Project to Row stream
        // P2: Also handle wildcard vs specific projections
        let is_wildcard = query.find.projections.len() == 1
            && matches!(&query.find.projections[0], Projection::Wildcard);

        let order_by_extras = self.extract_order_by_extras(&query);

        // Bug 1 fix: si hay post-filter con algos, vamos a necesitar
        // `<root_variable>.id` como columna auxiliar para hacer lookup en el
        // algo cache. La declaramos antes del stream para usarla luego al
        // strip.
        let single_post_id_aux: Option<String> = if single_where_post_filter.is_some() {
            Some(format!("{}.id", root_variable))
        } else {
            None
        };

        let mut row_stream: Box<dyn operators::RowStream + 'a> = if is_wildcard {
            Box::new(operators::ProjectWildcardStream::new(
                final_node_stream,
                root_variable.to_string(),
            ))
        } else {
            // Convert projections to strings for the operator
            let mut projection_strings: Vec<String> = query
                .find
                .projections
                .iter()
                .filter_map(|p| match p {
                    Projection::Wildcard => None,
                    Projection::All(var) => Some(format!("{}.*", var)),
                    Projection::Expression { expr, .. } => match expr {
                        Expression::Property { variable, property } => {
                            Some(property_projection_key(variable, property))
                        }
                        Expression::FunctionCall { name, args }
                            if name.eq_ignore_ascii_case("all") && args.len() == 1 =>
                        {
                            match &args[0] {
                                Expression::Property { variable, property }
                                    if property.is_empty() =>
                                {
                                    Some(format!("{}.*", variable))
                                }
                                _ => None,
                            }
                        }
                        _ => None, // Handle other expressions if needed
                    },
                })
                .collect();

            // Add extra columns needed for ORDER BY
            for extra in &order_by_extras {
                if !projection_strings.contains(extra) {
                    projection_strings.push(extra.clone());
                }
            }

            // Add aux id key for Bug 1 post-filter if not already present.
            if let Some(aux) = &single_post_id_aux
                && !projection_strings.contains(aux)
            {
                projection_strings.push(aux.clone());
            }
            Box::new(operators::ProjectNodesStream::new(
                final_node_stream,
                root_variable.to_string(),
                projection_strings,
            ))
        };

        // Step 4: Collect to result (public API compatibility).
        // Optimization: if there is no ORDER BY, apply LIMIT/OFFSET while streaming
        // to avoid materializing all rows in memory.
        let mut rows = Vec::new();
        if query.order_by.is_none() {
            let (offset, take_limit) = if let Some(limit) = &query.limit {
                (limit.offset.unwrap_or(0), limit.limit)
            } else {
                (0, usize::MAX)
            };

            let mut skipped = 0usize;
            while let Some(row) = row_stream.next().await? {
                if skipped < offset {
                    skipped += 1;
                    continue;
                }
                if rows.len() >= take_limit {
                    break;
                }
                rows.push(row);
            }
        } else {
            while let Some(row) = row_stream.next().await? {
                rows.push(row);
            }
        }

        // Initialize QueryResult with the expected column names (original + extras for now)
        let mut column_names: Vec<String> = query
            .find
            .projections
            .iter()
            .filter_map(|p| match p {
                Projection::Wildcard => Some("*".into()),
                Projection::All(var) => Some(format!("{}.*", var)),
                Projection::Expression { expr, alias } => {
                    if let Some(a) = alias {
                        Some(a.clone())
                    } else if let Expression::Property { variable, property } = expr {
                        Some(property_projection_key(variable, property))
                    } else if let Expression::FunctionCall { name, args } = expr {
                        if name.eq_ignore_ascii_case("all") && args.len() == 1 {
                            if let Expression::Property { variable, property } = &args[0]
                                && property.is_empty()
                            {
                                Some(format!("{}.*", variable))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            })
            .collect();

        // Add extras to columns so row values are kept
        for extra in &order_by_extras {
            if !column_names.contains(extra) {
                column_names.push(extra.clone());
            }
        }

        // Alias renaming for non-pattern queries (mirrors execute_pattern_query)
        let alias_pairs: Vec<(String, String)> = query
            .find
            .projections
            .iter()
            .filter_map(|projection| match projection {
                Projection::Expression {
                    expr: Expression::Property { variable, property },
                    alias: Some(alias),
                } => Some((property_projection_key(variable, property), alias.clone())),
                _ => None,
            })
            .collect();

        if !alias_pairs.is_empty() {
            for row in &mut rows {
                for (from, to) in &alias_pairs {
                    if let Some(value) = row.values.get(from).cloned() {
                        row.values.insert(to.clone(), value);
                    }
                }
            }
        }

        // Bug 1 fix (single-node path): apply algorithm-bearing WHERE
        // predicates as a post-filter using the pre-computed algo cache.
        if let Some(post_expr) = &single_where_post_filter {
            let nodes_for_algo = self.graph.get_all_nodes().await?;
            let cache = precompute_for_query(self.graph, &nodes_for_algo, &query).await?;
            let target = String::new(); // single-node: no target var, but eval handles by var name
            rows.retain(|row| {
                eval_row_condition_with_algo(row, post_expr, root_variable, &target, &cache)
            });
            // Strip auxiliary <var>.id if we added it.
            if let Some(aux) = &single_post_id_aux {
                let was_in_query_proj = query.find.projections.iter().any(|p| match p {
                    Projection::Expression {
                        expr: Expression::Property { variable, property },
                        ..
                    } => format!("{}.{}", variable, property) == *aux,
                    _ => false,
                });
                if !was_in_query_proj {
                    for row in &mut rows {
                        row.values.remove(aux);
                    }
                }
            }
        }

        let mut result = QueryResult::new(column_names);
        result.rows = rows;

        self.apply_distinct_if_needed(&mut result, &query.find);

        log::debug!(
            "After filter and project: {} rows (including {} extras)",
            result.len(),
            order_by_extras.len()
        );

        // Step 5: Apply ORDER BY (BEFORE LIMIT — P0 fix)
        if let Some(order_by) = &query.order_by {
            self.apply_order_by(&mut result, order_by);
        }

        // Step 6: Apply LIMIT/OFFSET on projected result
        // (only needed here when ORDER BY is present)
        if query.order_by.is_some()
            && let Some(limit) = &query.limit
        {
            let offset = limit.offset.unwrap_or(0);
            result.rows = result
                .rows
                .into_iter()
                .skip(offset)
                .take(limit.limit)
                .collect();
        }

        // Step 7: Strip extra ORDER BY columns that weren't in the original projection (P2)
        if !order_by_extras.is_empty() {
            self.strip_extra_columns(&mut result, &order_by_extras);
        }

        log::info!("Query execution complete: {} rows", result.len());

        Ok(result)
    }

    /// Execute FROM clause (streaming)
    async fn execute_from_stream(
        &self,
        query: &Query,
    ) -> Result<Box<dyn operators::NodeStream + 'a>> {
        if query.from.patterns.is_empty() {
            return Err(NopalError::QueryExecutionError(
                "No patterns in FROM clause".into(),
            ));
        }

        let pattern = &query.from.patterns[0];

        if pattern.elements.is_empty() {
            return Err(NopalError::QueryExecutionError("Empty pattern".into()));
        }

        if pattern.elements.len() == 1 {
            match &pattern.elements[0] {
                PatternElement::Node(node_pattern) => {
                    let mut stream =
                        operators::scan_nodes_stream(self.graph, node_pattern.label.as_deref())
                            .await?;
                    // Apply inline property map from the FROM pattern, e.g. `(n:Person {dept: "eng"})`.
                    if !node_pattern.properties.is_empty() {
                        let props = node_pattern.properties.clone();
                        stream = Box::new(operators::FilterNodesStream::new(stream, move |node| {
                            Ok(props.iter().all(|(k, v)| node.properties.get(k) == Some(v)))
                        }));
                    }
                    Ok(stream)
                }
                PatternElement::Relationship(_) => Err(NopalError::QueryExecutionError(
                    "Pattern cannot start with relationship".into(),
                )),
            }
        } else {
            Err(NopalError::QueryExecutionError(
                "Relationship patterns need execute_pattern_query()".into(),
            ))
        }
    }

    /// Extract indexed condition from WHERE clause
    /// Returns: (variable, property, value) if found
    fn extract_indexed_condition(
        &self,
        filter: &WhereClause,
    ) -> Result<Option<(String, String, PropertyValue)>> {
        // Log para debug
        log::debug!("Extracting indexed condition from WHERE clause");

        // Solo procesamos BinaryOp con operador =
        match &filter.condition {
            Expression::BinaryOp { left, op, right } => {
                log::debug!("Found BinaryOp: {:?}", op);

                // Verificar que sea comparación de igualdad
                if !matches!(op, BinaryOperator::Eq) {
                    log::debug!("Operator is not Eq, skipping index");
                    return Ok(None);
                }

                // Extraer propiedad del lado izquierdo (e.g., "c.house")
                let (variable, property) = match &**left {
                    Expression::Property { variable, property } => {
                        log::debug!("Found property: {}.{}", variable, property);
                        (variable.clone(), property.clone())
                    }
                    _ => {
                        log::debug!("Left side is not a Property, skipping index");
                        return Ok(None);
                    }
                };

                // Extraer valor del lado derecho (e.g., "TeamA")
                let value = match &**right {
                    Expression::Literal(val) => {
                        log::debug!("Found literal value: {:?}", val);
                        val.clone()
                    }
                    _ => {
                        log::debug!("Right side is not a Literal, skipping index");
                        return Ok(None);
                    }
                };

                // Obtener label del pattern
                // TODO: Mejorar para obtener del query pattern
                // Por ahora, asumimos que el variable corresponde a un label
                // Este es un workaround temporal

                log::info!(
                    "✅ Extracted indexed condition: {}.{} = {:?}",
                    variable,
                    property,
                    value
                );
                Ok(Some((variable, property, value)))
            }
            _ => {
                log::debug!("Condition is not BinaryOp, skipping index");
                Ok(None)
            }
        }
    }

    /// Apply remaining filters after index lookup
    /// For now, just returns the nodes as-is
    /// TODO: Implement proper filtering for complex conditions
    fn apply_remaining_filters(&self, nodes: Vec<Node>, query: &Query) -> Result<Vec<Node>> {
        let root_variable = query
            .from
            .patterns
            .first()
            .and_then(|p| p.elements.first())
            .and_then(|e| match e {
                PatternElement::Node(n) => n.variable.as_deref().or(Some("n")),
                _ => None,
            })
            .unwrap_or("n");

        // Apply WHERE conditions that weren't handled by the index lookup
        // The index handled the equality condition; apply remaining filter conditions
        if let Some(filter) = &query.filter {
            Ok(self.apply_filter(nodes, &filter.condition, root_variable)?)
        } else {
            Ok(nodes)
        }
    }

    /// Execute query with pattern matching (relationships)
    async fn execute_pattern_query(&self, query: Query) -> Result<QueryResult> {
        log::info!("Executing streaming pattern matching query");

        if query.from.patterns.len() > 1 {
            return self.execute_multi_pattern_query(query).await;
        }

        // Extract pattern
        let pattern = &query.from.patterns[0];

        if pattern.elements.len() < 3 {
            return Err(NopalError::QueryExecutionError(
                "Pattern must have at least: node -> rel -> node".into(),
            ));
        }

        let execution_mode = self.determine_execution_mode(&query, pattern);
        log::debug!(
            "Pattern query execution mode selected: {:?}",
            execution_mode
        );

        if execution_mode == ExecutionMode::LinearBindings {
            let pattern = pattern.clone();
            return self
                .execute_linear_multihop_pattern_query(query, &pattern)
                .await;
        }

        // 1. Establish source stream
        let source_pattern = match &pattern.elements[0] {
            PatternElement::Node(n) => n,
            _ => {
                return Err(NopalError::QueryExecutionError(
                    "Pattern must start with node".into(),
                ));
            }
        };
        let mut source_stream =
            operators::scan_nodes_stream(self.graph, source_pattern.label.as_deref()).await?;

        // Apply inline property map filter for source node pattern, e.g. `(a:Person {active: true})`.
        // The label is already handled by scan_nodes_stream; properties need an explicit post-filter.
        if !source_pattern.properties.is_empty() {
            let props = source_pattern.properties.clone();
            source_stream = Box::new(operators::FilterNodesStream::new(
                source_stream,
                move |node| Ok(props.iter().all(|(k, v)| node.properties.get(k) == Some(v))),
            ));
        }

        // 2. Setup Traversal
        let rel_pattern = match &pattern.elements[1] {
            PatternElement::Relationship(r) => r,
            _ => {
                return Err(NopalError::QueryExecutionError(
                    "Expected relationship after node".into(),
                ));
            }
        };

        let target_pattern = match &pattern.elements[2] {
            PatternElement::Node(n) => n,
            _ => {
                return Err(NopalError::QueryExecutionError(
                    "Expected node after relationship".into(),
                ));
            }
        };

        // Construir TraverseStream con pushdown sobre:
        //   - label + properties del nodo destino
        //   - properties inline de la arista, e.g. -[r:TRANS {amount: 1000}]->
        let mut pattern_stream: Box<dyn operators::PatternMatchStream + 'a> = Box::new(
            operators::TraverseStream::new(
                self.graph,
                source_stream,
                rel_pattern.rel_type.clone(),
                target_pattern.label.clone(),
                target_pattern.properties.clone(),
            )
            .with_edge_properties(rel_pattern.properties.clone()),
        );

        // 3. Resolve pattern variables (with fallback inference for parser edge cases).
        let edge_var = rel_pattern.variable.clone();
        let (source_var, target_var) = self.resolve_pattern_vars_for_query(
            source_pattern.variable.as_deref(),
            target_pattern.variable.as_deref(),
            edge_var.as_deref(),
            &query,
        );

        // Bug 1 fix: detect if WHERE contains per-node algorithm predicates
        // (degree, pagerank, ...). These cannot be evaluated by the streaming
        // pattern filter (which has no access to the global algo cache), so
        // we extract them as a post-projection row filter where the algo
        // cache is available. Non-algo predicates still run during streaming
        // for early filtering.
        let where_post_filter: Option<Expression> = query
            .filter
            .as_ref()
            .and_then(|f| extract_algorithm_predicates(&f.condition));

        if let Some(filter) = &query.filter {
            // If the entire WHERE is algorithm-only, skip stream-time filter.
            // Otherwise, push down everything that is not algo-related.
            let stream_pred = if where_post_filter.is_some() {
                strip_algorithm_predicates(&filter.condition)
            } else {
                Some(filter.condition.clone())
            };
            if let Some(expr) = stream_pred {
                pattern_stream = operators::filter_pattern_stream_from_expr(
                    pattern_stream,
                    std::sync::Arc::new(expr),
                    source_var.clone(),
                    target_var.clone(),
                );
            }
        }

        log::debug!("Streaming pattern matches ready");

        // 4. Handle Aggregations (Transition: collect matches for aggregation logic)
        //
        // Solo enrutamos al engine de agregaciones si hay agregaciones reales
        // (count/sum/avg/min/max) o GROUP BY explícito. Las funciones algorítmicas
        // por sí solas (degree, pagerank, etc.) NO disparan el aggregation path —
        // se manejan en el regular pattern path con post-projection algo lookup
        // (Bug 2: emitían NULL al colapsar matches en una sola fila).
        if has_real_aggregations(&query.find.projections) || query.group_by.is_some() {
            let mut matches = Vec::new();
            while let Some(m) = pattern_stream.next().await? {
                matches.push(m);
            }
            let edge_var = rel_pattern.variable.as_deref();
            // Bug 3 fix: pre-compute algorithm cache so HAVING/projections
            // referencing degree(), pagerank(), etc. resolve correctly.
            let agg_algo_cache = {
                let all_nodes = self.graph.get_all_nodes().await?;
                precompute_for_query(self.graph, &all_nodes, &query).await?
            };
            let mut result = self.execute_pattern_aggregations(
                &matches,
                &query,
                &source_var,
                &target_var,
                edge_var,
                &agg_algo_cache,
            )?;
            self.apply_distinct_if_needed(&mut result, &query.find);

            if let Some(order_by) = &query.order_by {
                self.apply_order_by(&mut result, order_by);
            }
            if let Some(limit) = &query.limit {
                let offset = limit.offset.unwrap_or(0);
                result.rows = result
                    .rows
                    .into_iter()
                    .skip(offset)
                    .take(limit.limit)
                    .collect();
            }
            return Ok(result);
        }

        // 5. Projection (Non-aggregation path)
        // Keep internal projection keys separate from output column names so
        // aliases can be applied after streaming rows are materialized.
        let mut projection_strings: Vec<String> = query
            .find
            .projections
            .iter()
            .filter_map(|p| match p {
                Projection::Wildcard => Some("*".into()),
                Projection::All(var) => Some(format!("{}.*", var)),
                Projection::Expression { expr, .. } => {
                    if let Expression::Property { variable, property } = expr {
                        Some(property_projection_key(variable, property))
                    } else if let Expression::FunctionCall { name, args } = expr {
                        if name.eq_ignore_ascii_case("all") && args.len() == 1 {
                            if let Expression::Property { variable, property } = &args[0]
                                && property.is_empty()
                            {
                                Some(format!("{}.*", variable))
                            } else {
                                None
                            }
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            })
            .collect();

        let order_by_extras = self.extract_order_by_extras(&query);
        for extra in &order_by_extras {
            if !projection_strings.contains(extra) {
                projection_strings.push(extra.clone());
            }
        }

        // Bug 2 fix: collect algorithm function projections so we can resolve
        // them per-row after streaming. We need each algo's source variable's
        // `.id` in projection_strings to look up the node in the algo cache.
        let algo_projs: Vec<(String, String, String)> = query
            .find
            .projections
            .iter()
            .filter_map(|p| match p {
                Projection::Expression { expr, alias } if expr.is_algorithm() => {
                    let (name, var) = match expr {
                        Expression::FunctionCall { name, args } => {
                            let v = match args.first() {
                                Some(Expression::Property { variable, property })
                                    if property.is_empty() =>
                                {
                                    variable.clone()
                                }
                                _ => return None,
                            };
                            (name.clone(), v)
                        }
                        _ => return None,
                    };
                    let lower = name.to_lowercase();
                    let default_key = format!("{}({})", lower, var);
                    let key = alias.clone().unwrap_or(default_key);
                    Some((key, name.clone(), var))
                }
                _ => None,
            })
            .collect();

        // Track which `<var>.id` keys we added solely to support algo lookup,
        // so we can strip them from the final result.
        let mut algo_extra_id_keys: Vec<String> = Vec::new();
        for (_, _, var) in &algo_projs {
            let id_key = format!("{}.id", var);
            if !projection_strings.contains(&id_key) {
                projection_strings.push(id_key.clone());
                algo_extra_id_keys.push(id_key);
            }
        }

        let mut row_stream = operators::ProjectPatternStream::new(
            pattern_stream,
            source_var.clone(),
            target_var.clone(),
            edge_var,
            projection_strings,
        );

        // 6. Collect result rows
        // Optimization: if there is no ORDER BY, apply LIMIT/OFFSET while streaming.
        let mut rows = Vec::new();
        if query.order_by.is_none() {
            let (offset, take_limit) = if let Some(limit) = &query.limit {
                (limit.offset.unwrap_or(0), limit.limit)
            } else {
                (0, usize::MAX)
            };

            let mut skipped = 0usize;
            while let Some(row) = row_stream.next().await? {
                if skipped < offset {
                    skipped += 1;
                    continue;
                }
                if rows.len() >= take_limit {
                    break;
                }
                rows.push(row);
            }
        } else {
            while let Some(row) = row_stream.next().await? {
                rows.push(row);
            }
        }

        // Determine final output columns for QueryResult
        let mut column_names: Vec<String> = query
            .find
            .projections
            .iter()
            .filter_map(|p| match p {
                Projection::Wildcard => Some("*".into()),
                Projection::All(var) => Some(format!("{}.*", var)),
                Projection::Expression { expr, alias } => {
                    if let Some(a) = alias {
                        Some(a.clone())
                    } else if let Expression::Property { variable, property } = expr {
                        Some(property_projection_key(variable, property))
                    } else if let Expression::FunctionCall { name, args } = expr {
                        if name.eq_ignore_ascii_case("all") && args.len() == 1 {
                            if let Expression::Property { variable, property } = &args[0]
                                && property.is_empty()
                            {
                                Some(format!("{}.*", variable))
                            } else {
                                None
                            }
                        } else if expr.is_algorithm() {
                            // Algorithm function without alias: use canonical
                            // form `<name>(<var>)`. Bug 2 fix.
                            let var = match args.first() {
                                Some(Expression::Property { variable, property })
                                    if property.is_empty() =>
                                {
                                    variable.clone()
                                }
                                _ => return None,
                            };
                            Some(format!("{}({})", name.to_lowercase(), var))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            })
            .collect();

        for extra in &order_by_extras {
            if !column_names.contains(extra) {
                column_names.push(extra.clone());
            }
        }

        let alias_pairs: Vec<(String, String)> = query
            .find
            .projections
            .iter()
            .filter_map(|projection| match projection {
                Projection::Expression {
                    expr: Expression::Property { variable, property },
                    alias: Some(alias),
                } => Some((property_projection_key(variable, property), alias.clone())),
                _ => None,
            })
            .collect();

        if !alias_pairs.is_empty() {
            for row in &mut rows {
                for (from, to) in &alias_pairs {
                    if let Some(value) = row.values.get(from).cloned() {
                        row.values.insert(to.clone(), value);
                    }
                }
            }
        }

        // Bug 2 fix: resolve algorithm function values per row using the
        // pre-computed algorithm cache. Cache is shared with Bug 1's WHERE
        // post-filter so we only compute once.
        let need_algo_cache = !algo_projs.is_empty() || where_post_filter.is_some();
        let row_algo_cache = if need_algo_cache {
            let all_nodes = self.graph.get_all_nodes().await?;
            Some(precompute_for_query(self.graph, &all_nodes, &query).await?)
        } else {
            None
        };
        if !algo_projs.is_empty()
            && let Some(cache) = row_algo_cache.as_ref()
        {
            for row in &mut rows {
                for (output_key, fn_name, var) in &algo_projs {
                    let id_key = format!("{}.id", var);
                    if let Some(PropertyValue::String(id_str)) = row.values.get(&id_key)
                        && let Ok(node_id) = uuid::Uuid::parse_str(id_str)
                    {
                        let val = lookup_algo_value(fn_name, &node_id, cache);
                        row.values.insert(output_key.clone(), val);
                    }
                }
            }
        }

        // Bug 1 fix: apply algorithm-bearing WHERE predicates as a post-filter
        // using the algo cache and the resolved row values.
        if let (Some(post_expr), Some(cache)) =
            (where_post_filter.as_ref(), row_algo_cache.as_ref())
        {
            rows.retain(|row| {
                eval_row_condition_with_algo(row, post_expr, &source_var, &target_var, cache)
            });
        }

        if !algo_extra_id_keys.is_empty() {
            for row in &mut rows {
                for key in &algo_extra_id_keys {
                    row.values.remove(key);
                }
            }
        }

        let mut result = QueryResult::new(column_names);
        result.rows = rows;

        self.apply_distinct_if_needed(&mut result, &query.find);

        // 7. Post-processing (ORDER BY, LIMIT, STRIP)
        if let Some(order_by) = &query.order_by {
            self.apply_order_by(&mut result, order_by);
        }

        if query.order_by.is_some()
            && let Some(limit) = &query.limit
        {
            let offset = limit.offset.unwrap_or(0);
            result.rows = result
                .rows
                .into_iter()
                .skip(offset)
                .take(limit.limit)
                .collect();
        }

        if !order_by_extras.is_empty() {
            self.strip_extra_columns(&mut result, &order_by_extras);
        }

        log::info!("Pattern query execution complete: {} rows", result.len());

        Ok(result)
    }

    fn determine_execution_mode(&self, query: &Query, pattern: &Pattern) -> ExecutionMode {
        if pattern.elements.len() > 3 {
            return ExecutionMode::LinearBindings;
        }

        if self.query_uses_path_metadata(query) {
            return ExecutionMode::LinearBindings;
        }

        let rel_has_quantifier = matches!(
            &pattern.elements[1],
            PatternElement::Relationship(r) if r.quantifier.is_some()
        );
        if rel_has_quantifier {
            return ExecutionMode::LinearBindings;
        }

        let uses_path_reducers = projections_contain_path_reducer(&query.find.projections)
            || query
                .filter
                .as_ref()
                .is_some_and(|f| expr_contains_path_reducer(&f.condition));
        if uses_path_reducers {
            return ExecutionMode::LinearBindings;
        }

        let uses_path_semantic_filters = query
            .filter
            .as_ref()
            .is_some_and(|f| expr_contains_path_semantic_filter(&f.condition));
        if uses_path_semantic_filters {
            return ExecutionMode::LinearBindings;
        }

        let uses_path_embeddings = query.find.projections.iter().any(|p| match p {
            Projection::Expression { expr, .. } => expr_contains_path_embedding_fn(expr),
            _ => false,
        }) || query
            .filter
            .as_ref()
            .is_some_and(|f| expr_contains_path_embedding_fn(&f.condition));
        if uses_path_embeddings {
            return ExecutionMode::LinearBindings;
        }

        if self.query_uses_quoted_path_vm(query) {
            return ExecutionMode::LinearBindings;
        }

        ExecutionMode::FastTraverse
    }

    async fn execute_multi_pattern_query(&self, query: Query) -> Result<QueryResult> {
        if has_aggregations(&query.find.projections) || query.group_by.is_some() {
            return Err(NopalError::QueryExecutionError(
                "Pattern aggregations for multi-pattern queries are not supported yet".into(),
            ));
        }

        if projections_contain_path_reducer(&query.find.projections)
            || query
                .filter
                .as_ref()
                .map(|f| expr_contains_path_reducer(&f.condition))
                .unwrap_or(false)
        {
            return Err(NopalError::QueryExecutionError(
                "Path reducers (path_sum, path_min, path_max, path_avg) are not supported in multi-pattern queries in Path Queries F3".into()
            ));
        }

        if query
            .filter
            .as_ref()
            .is_some_and(|f| expr_contains_path_semantic_filter(&f.condition))
        {
            return Err(NopalError::QueryExecutionError(
                "Semantic path filters are not supported in multi-pattern queries in Path Queries F4-D.1".into()
            ));
        }

        if query.find.projections.iter().any(|p| match p {
            Projection::Expression { expr, .. } => expr_contains_path_embedding_fn(expr),
            _ => false,
        }) || query
            .filter
            .as_ref()
            .is_some_and(|f| expr_contains_path_embedding_fn(&f.condition))
        {
            return Err(NopalError::QueryExecutionError(
                "Path embedding functions are not supported in multi-pattern queries in Path Queries F4-D".into()
            ));
        }

        if query.return_expr.is_some() {
            return Err(NopalError::QueryExecutionError(
                "RETURN clause is not supported in multi-pattern queries in Path Queries F4-C"
                    .into(),
            ));
        }

        if self.query_uses_quoted_path_vm(&query) {
            return Err(NopalError::QueryExecutionError(
                "INIT/GATHER/path_eval are not supported in multi-pattern queries in Path Queries F4-B".into()
            ));
        }

        let mut pattern_bindings_iter = query.from.patterns.iter();
        let first_pattern = pattern_bindings_iter
            .next()
            .ok_or_else(|| NopalError::QueryExecutionError("No patterns in FROM clause".into()))?;

        let mut joined_bindings = self.execute_pattern_bindings(first_pattern).await?;

        for pattern in pattern_bindings_iter {
            let next_bindings = self.execute_pattern_bindings(pattern).await?;
            joined_bindings = self.join_pattern_bindings(&joined_bindings, &next_bindings);
        }

        self.build_query_result_from_bindings(joined_bindings, &query)
    }

    async fn execute_linear_multihop_pattern_query(
        &self,
        query: Query,
        pattern: &Pattern,
    ) -> Result<QueryResult> {
        if has_aggregations(&query.find.projections) || query.group_by.is_some() {
            return Err(NopalError::QueryExecutionError(
                "Pattern aggregations for multi-hop queries are not supported yet".into(),
            ));
        }

        // F3: path reducers no soportados en ORDER BY
        if let Some(order_by) = &query.order_by {
            for item in &order_by.items {
                if expr_contains_path_reducer(&item.expression) {
                    return Err(NopalError::QueryExecutionError(
                        "Path reducers (path_sum, path_min, path_max, path_avg) are not supported in ORDER BY in Path Queries F3".into()
                    ));
                }
                if expr_contains_path_eval(&item.expression) {
                    return Err(NopalError::QueryExecutionError(
                        "path_eval(\"...\") is not supported in ORDER BY in Path Queries F4-B"
                            .into(),
                    ));
                }
            }
        }

        self.initialize_path_profile();
        let bindings = self.execute_linear_pattern_bindings(pattern).await?;
        self.update_path_profile(|metrics| {
            metrics.bindings_examined = bindings.len();
        });
        self.build_query_result_from_bindings(bindings, &query)
    }

    fn build_query_result_from_bindings(
        &self,
        bindings: Vec<LinearPatternBinding>,
        query: &Query,
    ) -> Result<QueryResult> {
        let has_path_reducer_in_filter = query
            .filter
            .as_ref()
            .is_some_and(|f| expr_contains_path_reducer(&f.condition));
        let has_path_eval_in_filter = query
            .filter
            .as_ref()
            .is_some_and(|f| expr_contains_path_eval(&f.condition));
        // F4-C: path.result in filter also needs VM path
        let has_path_result_in_filter = query
            .filter
            .as_ref()
            .is_some_and(|f| expr_uses_path_property_exec(&f.condition, "result"));
        let has_path_semantic_filter_in_filter = query
            .filter
            .as_ref()
            .is_some_and(|f| expr_contains_path_semantic_filter(&f.condition));
        let has_path_embedding_in_filter = query
            .filter
            .as_ref()
            .is_some_and(|f| expr_contains_path_embedding_fn(&f.condition));
        let uses_path_vm = self.query_uses_quoted_path_vm(query);

        let mut binding_states: Vec<(LinearPatternBinding, Option<PathVmState>)> = Vec::new();

        for binding in bindings {
            let vm_state = if uses_path_vm {
                let mut state =
                    self.evaluate_path_vm_state(&binding, &query.init, &query.gather)?;
                // F4-C: evaluate RETURN once per path and store result in state
                if let Some(return_expr) = &query.return_expr {
                    let result = self.evaluate_path_return(&binding, &state, return_expr)?;
                    state.return_result = Some(result);
                }
                Some(state)
            } else {
                None
            };

            let keep = if let Some(filter) = &query.filter {
                if has_path_reducer_in_filter
                    || has_path_eval_in_filter
                    || has_path_result_in_filter
                    || has_path_semantic_filter_in_filter
                    || has_path_embedding_in_filter
                {
                    self.evaluate_linear_pattern_condition_with_vm(
                        &binding,
                        &filter.condition,
                        vm_state.as_ref(),
                    )?
                } else {
                    self.evaluate_linear_pattern_condition(&binding, &filter.condition)
                }
            } else {
                true
            };

            if keep {
                binding_states.push((binding, vm_state));
            }
        }

        self.update_path_profile(|metrics| {
            metrics.bindings_emitted = binding_states.len();
        });

        // F3: recopilar proyecciones de path reducers con propagación de error de aridad/tipo.
        let path_reducer_projs: Vec<(String, String, String)> = {
            let collected: Result<Vec<_>> = query
                .find
                .projections
                .iter()
                .filter_map(|p| match p {
                    Projection::Expression {
                        expr: Expression::FunctionCall { name, args },
                        alias,
                    } if is_path_reducer(name) => {
                        let result = Self::extract_reducer_prop(args).map(|prop| {
                            let key = alias.clone().unwrap_or_else(|| {
                                format!("{}(\"{}\")", name.to_lowercase(), prop)
                            });
                            (key, name.clone(), prop.to_string())
                        });
                        Some(result)
                    }
                    _ => None,
                })
                .collect();
            collected?
        };

        let path_eval_projs: Vec<(String, String)> = {
            let collected: Result<Vec<_>> = query
                .find
                .projections
                .iter()
                .filter_map(|p| match p {
                    Projection::Expression {
                        expr: Expression::FunctionCall { name, args },
                        alias,
                    } if is_path_eval(name) => {
                        let result = Self::extract_path_eval_expr(args).map(|quoted| {
                            let key = alias
                                .clone()
                                .unwrap_or_else(|| format!("path_eval(\"{}\")", quoted));
                            (key, quoted.to_string())
                        });
                        Some(result)
                    }
                    _ => None,
                })
                .collect();
            collected?
        };

        let path_embedding_projs: Vec<(String, String, Vec<Expression>)> = {
            let collected: Result<Vec<_>> = query
                .find
                .projections
                .iter()
                .filter_map(|p| match p {
                    Projection::Expression {
                        expr: Expression::FunctionCall { name, args },
                        alias,
                    } if is_path_embedding_fn(name) => {
                        let key = match alias.clone() {
                            Some(alias) => alias,
                            None => match path_embedding_projection_key(name, args) {
                                Ok(key) => key,
                                Err(err) => return Some(Err(err)),
                            },
                        };
                        Some(Ok((key, name.clone(), args.clone())))
                    }
                    _ => None,
                })
                .collect();
            collected?
        };

        // F4-C: detect path.start/end/state/result projections
        let path_object_projs: Vec<(String, String, Option<String>)> = query
            .find
            .projections
            .iter()
            .filter_map(|p| match p {
                Projection::Expression {
                    expr: Expression::Property { variable, property },
                    alias,
                } if variable == "path"
                    && matches!(property.as_str(), "start" | "end" | "state" | "result") =>
                {
                    let key = alias
                        .clone()
                        .unwrap_or_else(|| format!("path.{}", property));
                    Some((key, property.clone(), alias.clone()))
                }
                _ => None,
            })
            .collect();

        let mut projection_strings: Vec<String> = query
            .find
            .projections
            .iter()
            .filter_map(|p| match p {
                Projection::Wildcard => Some("*".into()),
                Projection::All(var) => Some(format!("{}.*", var)),
                Projection::Expression { expr, .. } => match expr {
                    Expression::Property { variable, property } => {
                        Some(property_projection_key(variable, property))
                    }
                    Expression::FunctionCall { name, args }
                        if name.eq_ignore_ascii_case("all") && args.len() == 1 =>
                    {
                        match &args[0] {
                            Expression::Property { variable, property } if property.is_empty() => {
                                Some(format!("{}.*", variable))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                },
            })
            .collect();

        let order_by_extras = self.extract_order_by_extras(query);
        for extra in &order_by_extras {
            if !projection_strings.contains(extra) {
                projection_strings.push(extra.clone());
            }
        }

        // F3: computar rows con path reducers (propagando errores estrictamente).
        let rows_result: Result<Vec<Row>> = binding_states
            .iter()
            .map(|(binding, vm_state)| {
                let mut row = self.project_linear_pattern_binding(binding, &projection_strings);
                for (key, name, prop) in &path_reducer_projs {
                    let value = Self::evaluate_path_reducer(binding, name, prop)?;
                    row.set(key.clone(), value);
                }
                for (key, quoted) in &path_eval_projs {
                    let value = self.evaluate_path_eval(binding, vm_state.as_ref(), quoted)?;
                    row.set(key.clone(), value);
                }
                for (key, name, args) in &path_embedding_projs {
                    let value = self.evaluate_path_embedding_function(binding, name, args)?;
                    row.set(key.clone(), value);
                }
                // F4-C: compute path.start/end/state/result projections
                for (key, kind, _) in &path_object_projs {
                    let value = match kind.as_str() {
                        "start" => binding.nodes.first().map(build_path_node_object),
                        "end" => binding.nodes.last().map(build_path_node_object),
                        "state" => vm_state.as_ref().map(|s| {
                            let entries: Vec<(String, PropertyValue)> =
                                s.vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                            PropertyValue::Object(entries)
                        }),
                        "result" => vm_state.as_ref().and_then(|s| s.return_result.clone()),
                        _ => None,
                    };
                    if let Some(v) = value {
                        row.set(key.clone(), v);
                    }
                }
                Ok(row)
            })
            .collect();
        let mut rows = rows_result?;

        let alias_pairs: Vec<(String, String)> = query
            .find
            .projections
            .iter()
            .filter_map(|projection| match projection {
                Projection::Expression {
                    expr: Expression::Property { variable, property },
                    alias: Some(alias),
                } => Some((property_projection_key(variable, property), alias.clone())),
                _ => None,
            })
            .collect();

        if !alias_pairs.is_empty() {
            for row in &mut rows {
                for (from, to) in &alias_pairs {
                    if let Some(value) = row.values.get(from).cloned() {
                        row.values.insert(to.clone(), value);
                    }
                }
            }
        }

        let mut column_names: Vec<String> = query
            .find
            .projections
            .iter()
            .filter_map(|p| match p {
                Projection::Wildcard => Some("*".into()),
                Projection::All(var) => Some(format!("{}.*", var)),
                Projection::Expression { expr, alias } => {
                    if let Some(a) = alias {
                        Some(a.clone())
                    } else if let Expression::Property { variable, property } = expr {
                        Some(property_projection_key(variable, property))
                    } else if let Expression::FunctionCall { name, args } = expr {
                        if name.eq_ignore_ascii_case("all") && args.len() == 1 {
                            if let Expression::Property { variable, property } = &args[0]
                                && property.is_empty()
                            {
                                Some(format!("{}.*", variable))
                            } else {
                                None
                            }
                        } else if is_path_reducer(name) {
                            let prop = Self::extract_reducer_prop(args).ok()?;
                            Some(format!("{}(\"{}\")", name.to_lowercase(), prop))
                        } else if is_path_eval(name) {
                            let quoted = Self::extract_path_eval_expr(args).ok()?;
                            Some(format!("path_eval(\"{}\")", quoted))
                        } else if is_path_embedding_fn(name) {
                            path_embedding_projection_key(name, args).ok()
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                }
            })
            .collect();

        // Nombres de columna para path reducer con alias ya quedan en el alias_pairs;
        // aquí añadimos la clave canónica para reducers sin alias.
        for (key, _, _) in &path_reducer_projs {
            if !column_names.contains(key) {
                column_names.push(key.clone());
            }
        }

        for (key, _) in &path_eval_projs {
            if !column_names.contains(key) {
                column_names.push(key.clone());
            }
        }

        // F4-C: column names for path object projections
        for (key, _, _) in &path_object_projs {
            if !column_names.contains(key) {
                column_names.push(key.clone());
            }
        }

        for extra in &order_by_extras {
            if !column_names.contains(extra) {
                column_names.push(extra.clone());
            }
        }

        let mut result = QueryResult::new(column_names);
        result.rows = rows;

        self.apply_distinct_if_needed(&mut result, &query.find);

        if let Some(order_by) = &query.order_by {
            self.apply_order_by(&mut result, order_by);
        }

        if let Some(limit) = &query.limit {
            let offset = limit.offset.unwrap_or(0);
            result.rows = result
                .rows
                .into_iter()
                .skip(offset)
                .take(limit.limit)
                .collect();
        }

        if !order_by_extras.is_empty() {
            self.strip_extra_columns(&mut result, &order_by_extras);
        }

        Ok(result)
    }

    async fn execute_pattern_bindings(
        &self,
        pattern: &Pattern,
    ) -> Result<Vec<LinearPatternBinding>> {
        if pattern.elements.len() == 1 {
            let source_pattern = match &pattern.elements[0] {
                PatternElement::Node(node_pattern) => node_pattern,
                _ => {
                    return Err(NopalError::QueryExecutionError(
                        "Pattern must start with node".into(),
                    ));
                }
            };

            let mut bindings = Vec::new();
            let mut source_stream =
                operators::scan_nodes_stream(self.graph, source_pattern.label.as_deref()).await?;
            while let Some(node) = source_stream.next().await? {
                if !self.node_matches_pattern(&node, source_pattern) {
                    continue;
                }

                let mut node_vars = HashMap::new();
                if let Some(var) = &source_pattern.variable {
                    node_vars.insert(var.clone(), node.clone());
                }

                bindings.push(LinearPatternBinding {
                    nodes: vec![node],
                    edges: Vec::new(),
                    node_vars,
                    edge_vars: HashMap::new(),
                });
            }

            Ok(bindings)
        } else {
            self.execute_linear_pattern_bindings(pattern).await
        }
    }

    async fn execute_linear_pattern_bindings(
        &self,
        pattern: &Pattern,
    ) -> Result<Vec<LinearPatternBinding>> {
        #[allow(clippy::manual_is_multiple_of)]
        if pattern.elements.is_empty() || pattern.elements.len() % 2 == 0 {
            return Err(NopalError::QueryExecutionError(
                "Linear pattern must alternate node and relationship elements".into(),
            ));
        }

        let source_pattern = match &pattern.elements[0] {
            PatternElement::Node(node_pattern) => node_pattern,
            _ => {
                return Err(NopalError::QueryExecutionError(
                    "Pattern must start with node".into(),
                ));
            }
        };

        let mut bindings = Vec::new();
        let mut source_stream =
            operators::scan_nodes_stream(self.graph, source_pattern.label.as_deref()).await?;
        while let Some(node) = source_stream.next().await? {
            if !self.node_matches_pattern(&node, source_pattern) {
                continue;
            }

            let mut node_vars = HashMap::new();
            if let Some(var) = &source_pattern.variable {
                node_vars.insert(var.clone(), node.clone());
            }

            bindings.push(LinearPatternBinding {
                nodes: vec![node],
                edges: Vec::new(),
                node_vars,
                edge_vars: HashMap::new(),
            });
        }

        for hop_idx in (1..pattern.elements.len()).step_by(2) {
            let rel_pattern = match &pattern.elements[hop_idx] {
                PatternElement::Relationship(rel_pattern) => rel_pattern,
                _ => {
                    return Err(NopalError::QueryExecutionError(
                        "Expected relationship in linear pattern".into(),
                    ));
                }
            };
            let target_pattern = match &pattern.elements[hop_idx + 1] {
                PatternElement::Node(node_pattern) => node_pattern,
                _ => {
                    return Err(NopalError::QueryExecutionError(
                        "Expected node after relationship".into(),
                    ));
                }
            };

            // Dispatch: fixed hop vs. quantified BFS (F1)
            bindings = match &rel_pattern.quantifier {
                None => {
                    self.expand_linear_hop(bindings, rel_pattern, target_pattern)
                        .await?
                }
                Some(q) => {
                    self.expand_quantified_hop(bindings, rel_pattern, q, target_pattern)
                        .await?
                }
            };
        }

        Ok(bindings)
    }

    /// Expande exactamente un hop en el patrón lineal.
    /// Comportamiento idéntico al loop original — extraído para reusar en dispatch.
    async fn expand_linear_hop(
        &self,
        bindings: Vec<LinearPatternBinding>,
        rel: &RelationshipPattern,
        target: &NodePattern,
    ) -> Result<Vec<LinearPatternBinding>> {
        let mut next_bindings = Vec::new();

        for binding in bindings {
            let current_node = binding.nodes.last().ok_or_else(|| {
                NopalError::QueryExecutionError(
                    "Linear pattern binding missing current node".into(),
                )
            })?;

            let candidate_edges = self
                .get_edges_for_direction(current_node.id, &rel.direction)
                .await?;
            for edge in candidate_edges {
                if let Some(rel_type) = &rel.rel_type
                    && edge.edge_type != *rel_type
                {
                    continue;
                }

                let next_node_id = match rel.direction {
                    Direction::Outgoing => edge.target,
                    Direction::Incoming => edge.source,
                    Direction::Bidirectional => {
                        if edge.source == current_node.id {
                            edge.target
                        } else {
                            edge.source
                        }
                    }
                };

                let next_node = match self.graph.get_node(next_node_id).await {
                    Ok(node) => node,
                    Err(_) => continue,
                };

                if !self.node_matches_pattern(&next_node, target) {
                    continue;
                }

                let mut next_binding = binding.clone();
                next_binding.edges.push(edge.clone());
                next_binding.nodes.push(next_node.clone());

                if let Some(var) = &rel.variable {
                    next_binding.edge_vars.insert(var.clone(), edge);
                }
                if let Some(var) = &target.variable {
                    next_binding.node_vars.insert(var.clone(), next_node);
                }

                next_bindings.push(next_binding);
            }
        }

        Ok(next_bindings)
    }

    /// BFS acotado para relaciones cuantificadas (F1).
    ///
    /// Semántica:
    /// - `{n}` → profundidad exacta n
    /// - `{n,m}` → todas las profundidades en [n, m]
    /// - `{n,}` → error explícito (unbounded no soportado en F1)
    /// - Variable en la relación cuantificada → error explícito
    /// - simple-path: un nodo no puede repetirse dentro de un mismo camino
    /// - El patrón terminal solo se aplica al nodo final del segmento cuantificado
    async fn expand_quantified_hop(
        &self,
        bindings: Vec<LinearPatternBinding>,
        rel: &RelationshipPattern,
        quantifier: &Quantifier,
        target: &NodePattern,
    ) -> Result<Vec<LinearPatternBinding>> {
        // F1: rechazar unbounded
        let max_depth = quantifier.max.ok_or_else(|| {
            NopalError::QueryExecutionError(
                "Unbounded quantified traversals are not supported in Path Queries F1; provide an upper bound".into()
            )
        })?;

        // F1: rechazar min=0 — semántica de zero-hop no está definida en F1
        if quantifier.min == 0 {
            return Err(NopalError::QueryExecutionError(
                "Zero-hop quantifiers ({0} or {0,n}) are not supported in Path Queries F1; min must be >= 1".into()
            ));
        }

        // F1: rechazar variable en relación cuantificada
        if rel.variable.is_some() {
            return Err(NopalError::QueryExecutionError(
                "Relationship variables on quantified hops are not supported in Path Queries F1"
                    .into(),
            ));
        }

        let min_depth = quantifier.min;
        let mut results = Vec::new();

        for binding in bindings {
            let source_node = binding.nodes.last().ok_or_else(|| {
                NopalError::QueryExecutionError(
                    "Linear pattern binding missing current node".into(),
                )
            })?;

            // Inicializar frontera BFS
            let mut frontier: std::collections::VecDeque<FrontierState> =
                std::collections::VecDeque::new();
            let mut initial_visited = HashSet::new();
            initial_visited.insert(source_node.id);

            frontier.push_back(FrontierState {
                binding: binding.clone(),
                current_node: source_node.clone(),
                depth: 0,
                visited: initial_visited,
            });

            while let Some(state) = frontier.pop_front() {
                self.update_path_profile(|metrics| {
                    metrics.frontier_states_visited += 1;
                });

                if state.depth >= max_depth {
                    continue;
                }

                let candidate_edges = self
                    .get_edges_for_direction(state.current_node.id, &rel.direction)
                    .await?;

                for edge in candidate_edges {
                    // Filtrar por tipo de relación
                    if let Some(rel_type) = &rel.rel_type
                        && edge.edge_type != *rel_type
                    {
                        continue;
                    }

                    // Filtrar por propiedades de arista (cada hop debe satisfacerlas)
                    let mut props_ok = true;
                    for (key, expected) in &rel.properties {
                        match edge.properties.get(key) {
                            Some(actual) if actual == expected => {}
                            _ => {
                                props_ok = false;
                                break;
                            }
                        }
                    }
                    if !props_ok {
                        continue;
                    }

                    let next_node_id = match rel.direction {
                        Direction::Outgoing => edge.target,
                        Direction::Incoming => edge.source,
                        Direction::Bidirectional => {
                            if edge.source == state.current_node.id {
                                edge.target
                            } else {
                                edge.source
                            }
                        }
                    };

                    // simple-path: descartar si el nodo ya fue visitado en este camino
                    if state.visited.contains(&next_node_id) {
                        self.update_path_profile(|metrics| {
                            metrics.cycle_prunes += 1;
                        });
                        continue;
                    }

                    let next_node = match self.graph.get_node(next_node_id).await {
                        Ok(node) => node,
                        Err(_) => continue,
                    };

                    let next_depth = state.depth + 1;
                    self.update_path_profile(|metrics| {
                        metrics.max_depth_observed = metrics.max_depth_observed.max(next_depth);
                    });

                    // Emitir resultado si profundidad en rango y terminal coincide
                    if next_depth >= min_depth && self.node_matches_pattern(&next_node, target) {
                        let mut next_binding = state.binding.clone();
                        next_binding.edges.push(edge.clone());
                        next_binding.nodes.push(next_node.clone());
                        if let Some(var) = &target.variable {
                            next_binding
                                .node_vars
                                .insert(var.clone(), next_node.clone());
                        }
                        results.push(next_binding);
                    }

                    // Continuar expandiendo si no alcanzamos el máximo
                    if next_depth < max_depth {
                        let mut next_visited = state.visited.clone();
                        next_visited.insert(next_node_id);

                        let mut next_binding = state.binding.clone();
                        next_binding.edges.push(edge.clone());
                        next_binding.nodes.push(next_node.clone());

                        frontier.push_back(FrontierState {
                            binding: next_binding,
                            current_node: next_node,
                            depth: next_depth,
                            visited: next_visited,
                        });
                    }
                }
            }
        }

        Ok(results)
    }

    fn join_pattern_bindings(
        &self,
        left_bindings: &[LinearPatternBinding],
        right_bindings: &[LinearPatternBinding],
    ) -> Vec<LinearPatternBinding> {
        let mut joined = Vec::new();

        for left in left_bindings {
            for right in right_bindings {
                if !self.pattern_bindings_compatible(left, right) {
                    continue;
                }

                let mut merged = left.clone();
                merged.nodes.extend(right.nodes.clone());
                merged.edges.extend(right.edges.clone());

                for (var, node) in &right.node_vars {
                    merged
                        .node_vars
                        .entry(var.clone())
                        .or_insert_with(|| node.clone());
                }
                for (var, edge) in &right.edge_vars {
                    merged
                        .edge_vars
                        .entry(var.clone())
                        .or_insert_with(|| edge.clone());
                }

                joined.push(merged);
            }
        }

        joined
    }

    fn pattern_bindings_compatible(
        &self,
        left: &LinearPatternBinding,
        right: &LinearPatternBinding,
    ) -> bool {
        for (var, left_node) in &left.node_vars {
            if let Some(right_node) = right.node_vars.get(var)
                && left_node.id != right_node.id
            {
                return false;
            }

            if right.edge_vars.contains_key(var) {
                return false;
            }
        }

        for (var, left_edge) in &left.edge_vars {
            if let Some(right_edge) = right.edge_vars.get(var)
                && left_edge.id != right_edge.id
            {
                return false;
            }

            if right.node_vars.contains_key(var) {
                return false;
            }
        }

        true
    }

    async fn get_edges_for_direction(
        &self,
        node_id: uuid::Uuid,
        direction: &Direction,
    ) -> Result<Vec<Edge>> {
        match direction {
            Direction::Outgoing => self.graph.get_outgoing_edges(node_id).await,
            Direction::Incoming => self.graph.get_incoming_edges(node_id).await,
            Direction::Bidirectional => {
                let mut outgoing = self.graph.get_outgoing_edges(node_id).await?;
                let mut incoming = self.graph.get_incoming_edges(node_id).await?;
                outgoing.append(&mut incoming);
                Ok(outgoing)
            }
        }
    }

    fn node_matches_pattern(
        &self,
        node: &Node,
        pattern: &crate::query::nql::parser::ast::NodePattern,
    ) -> bool {
        if let Some(label) = &pattern.label
            && node.label != *label
        {
            return false;
        }

        for (key, expected) in &pattern.properties {
            match node.properties.get(key) {
                Some(actual) if actual == expected => {}
                _ => return false,
            }
        }

        true
    }

    fn evaluate_linear_pattern_condition(
        &self,
        binding: &LinearPatternBinding,
        expr: &Expression,
    ) -> bool {
        match expr {
            Expression::BinaryOp { left, op, right } => match op {
                BinaryOperator::And => {
                    self.evaluate_linear_pattern_condition(binding, left)
                        && self.evaluate_linear_pattern_condition(binding, right)
                }
                BinaryOperator::Or => {
                    self.evaluate_linear_pattern_condition(binding, left)
                        || self.evaluate_linear_pattern_condition(binding, right)
                }
                _ => {
                    let left_val = self.evaluate_linear_pattern_expression(binding, left);
                    let right_val = self.evaluate_linear_pattern_expression(binding, right);
                    match (left_val, right_val) {
                        (Some(l), Some(r)) => self.compare_values(&l, op, &r),
                        _ => false,
                    }
                }
            },
            _ => true,
        }
    }

    fn evaluate_linear_pattern_expression(
        &self,
        binding: &LinearPatternBinding,
        expr: &Expression,
    ) -> Option<PropertyValue> {
        match expr {
            Expression::Literal(val) => Some(val.clone()),
            Expression::Property { variable, property } => {
                if variable == "path" {
                    let metadata = self.path_metadata_from_binding(binding);
                    return match property.as_str() {
                        "depth" => Some(PropertyValue::Int(metadata.depth as i64)),
                        "nodes" => Some(metadata.nodes),
                        "edges" => Some(metadata.edges),
                        _ => None,
                    };
                }

                if let Some(node) = binding.node_vars.get(variable) {
                    if property.is_empty() || property == "id" {
                        return Some(PropertyValue::String(node.id.to_string()));
                    }
                    if property == "label" {
                        return Some(PropertyValue::String(node.label.clone()));
                    }
                    return node.properties.get(property).cloned();
                }

                if let Some(edge) = binding.edge_vars.get(variable) {
                    if property.is_empty() || property == "id" {
                        return Some(PropertyValue::String(edge.id.to_string()));
                    }
                    if property == "type" || property == "edge_type" {
                        return Some(PropertyValue::String(edge.edge_type.clone()));
                    }
                    return edge.properties.get(property).cloned();
                }

                None
            }
            _ => None,
        }
    }

    // ─── F3: Path Reducers ───────────────────────────────────────────────────

    /// Extrae el valor numérico de la propiedad `prop` de una arista.
    /// Falla explícitamente si la propiedad no existe o no es numérica (semántica estricta F3).
    fn edge_property_as_numeric(edge: &Edge, prop: &str) -> Result<PropertyValue> {
        match edge.properties.get(prop) {
            Some(PropertyValue::Int(n)) => Ok(PropertyValue::Int(*n)),
            Some(PropertyValue::Float(f)) => Ok(PropertyValue::Float(*f)),
            Some(_) => Err(NopalError::QueryExecutionError(format!(
                "path reducer: property '{}' on edge {} is not numeric",
                prop, edge.id
            ))),
            None => Err(NopalError::QueryExecutionError(format!(
                "path reducer: property '{}' missing on edge {}",
                prop, edge.id
            ))),
        }
    }

    /// Valida y extrae el único argumento string-literal de un path reducer.
    fn extract_reducer_prop(args: &[Expression]) -> Result<&str> {
        if args.len() != 1 {
            return Err(NopalError::QueryExecutionError(format!(
                "path reducers require exactly 1 argument (property name), got {}",
                args.len()
            )));
        }
        match &args[0] {
            Expression::Literal(PropertyValue::String(s)) => Ok(s.as_str()),
            _ => Err(NopalError::QueryExecutionError(
                "path reducer argument must be a string literal, e.g. path_sum(\"amount\")".into(),
            )),
        }
    }

    /// Evalúa un path reducer sobre todas las aristas del binding.
    /// Semántica estricta: falla si alguna arista no tiene la propiedad o es de tipo incorrecto.
    fn evaluate_path_reducer(
        binding: &LinearPatternBinding,
        name: &str,
        prop: &str,
    ) -> Result<PropertyValue> {
        if binding.edges.is_empty() {
            return Err(NopalError::QueryExecutionError(
                "path reducers require at least one traversed edge in the path".into(),
            ));
        }

        let values: Result<Vec<PropertyValue>> = binding
            .edges
            .iter()
            .map(|e| Self::edge_property_as_numeric(e, prop))
            .collect();
        let values = values?;

        match name.to_lowercase().as_str() {
            "path_sum" => {
                if values.iter().any(|v| matches!(v, PropertyValue::Float(_))) {
                    let sum: f64 = values
                        .iter()
                        .map(|v| match v {
                            PropertyValue::Float(f) => *f,
                            PropertyValue::Int(i) => *i as f64,
                            _ => unreachable!(),
                        })
                        .sum();
                    Ok(PropertyValue::Float(sum))
                } else {
                    let sum: i64 = values
                        .iter()
                        .map(|v| match v {
                            PropertyValue::Int(i) => *i,
                            _ => unreachable!(),
                        })
                        .sum();
                    Ok(PropertyValue::Int(sum))
                }
            }
            "path_min" => {
                if values.iter().any(|v| matches!(v, PropertyValue::Float(_))) {
                    let min_val = values
                        .iter()
                        .map(|v| match v {
                            PropertyValue::Float(f) => *f,
                            PropertyValue::Int(i) => *i as f64,
                            _ => unreachable!(),
                        })
                        .fold(f64::INFINITY, f64::min);
                    Ok(PropertyValue::Float(min_val))
                } else {
                    let min_val = values
                        .iter()
                        .map(|v| match v {
                            PropertyValue::Int(i) => *i,
                            _ => unreachable!(),
                        })
                        .min()
                        .expect("non-empty");
                    Ok(PropertyValue::Int(min_val))
                }
            }
            "path_max" => {
                if values.iter().any(|v| matches!(v, PropertyValue::Float(_))) {
                    let max_val = values
                        .iter()
                        .map(|v| match v {
                            PropertyValue::Float(f) => *f,
                            PropertyValue::Int(i) => *i as f64,
                            _ => unreachable!(),
                        })
                        .fold(f64::NEG_INFINITY, f64::max);
                    Ok(PropertyValue::Float(max_val))
                } else {
                    let max_val = values
                        .iter()
                        .map(|v| match v {
                            PropertyValue::Int(i) => *i,
                            _ => unreachable!(),
                        })
                        .max()
                        .expect("non-empty");
                    Ok(PropertyValue::Int(max_val))
                }
            }
            "path_avg" => {
                let sum: f64 = values
                    .iter()
                    .map(|v| match v {
                        PropertyValue::Float(f) => *f,
                        PropertyValue::Int(i) => *i as f64,
                        _ => unreachable!(),
                    })
                    .sum();
                Ok(PropertyValue::Float(sum / values.len() as f64))
            }
            _ => unreachable!("caller must check is_path_reducer before calling"),
        }
    }

    fn extract_path_eval_expr(args: &[Expression]) -> Result<&str> {
        if args.len() != 1 {
            return Err(NopalError::QueryExecutionError(format!(
                "path_eval requires exactly 1 quoted expression, got {}",
                args.len()
            )));
        }

        match &args[0] {
            Expression::Literal(PropertyValue::String(s)) => Ok(s.as_str()),
            _ => Err(NopalError::QueryExecutionError(
                "path_eval argument must be a quoted expression, e.g. path_eval(\"sum\")".into(),
            )),
        }
    }

    fn evaluate_path_vm_state(
        &self,
        binding: &LinearPatternBinding,
        init: &[String],
        gather: &[String],
    ) -> Result<PathVmState> {
        let mut state = PathVmState::default();
        let init_ctx = PathVmHopContext {
            source: None,
            target: None,
            edge: None,
            path_depth: binding.edges.len(),
            binding: None,
        };

        for stmt in init {
            let assignment = parse_vm_assignment(stmt)?;
            let value = self.evaluate_path_vm_expr(&assignment.expr, &state, &init_ctx)?;
            state.vars.insert(assignment.variable, value);
        }

        for hop_idx in 0..binding.edges.len() {
            let ctx = PathVmHopContext {
                source: binding.nodes.get(hop_idx),
                target: binding.nodes.get(hop_idx + 1),
                edge: binding.edges.get(hop_idx),
                path_depth: binding.edges.len(),
                binding: None,
            };

            for stmt in gather {
                let assignment = parse_vm_assignment(stmt)?;
                let value = self.evaluate_path_vm_expr(&assignment.expr, &state, &ctx)?;
                state.vars.insert(assignment.variable, value);
            }
        }

        Ok(state)
    }

    fn evaluate_path_eval(
        &self,
        binding: &LinearPatternBinding,
        vm_state: Option<&PathVmState>,
        quoted_expr: &str,
    ) -> Result<PropertyValue> {
        let expr = parse_vm_expression(quoted_expr)?;
        let empty_state = PathVmState::default();
        let ctx = PathVmHopContext {
            source: None,
            target: None,
            edge: None,
            path_depth: binding.edges.len(),
            binding: None,
        };

        self.evaluate_path_vm_expr(&expr, vm_state.unwrap_or(&empty_state), &ctx)
    }

    /// Evalúa la cláusula `return "..."` una vez por path completo (F4-C).
    ///
    /// Contexto disponible: variables finales del mini-VM, path.depth/start/end/state.
    /// Restricción: el resultado debe ser escalar (Int, Float, Bool, String, Null).
    fn evaluate_path_return(
        &self,
        binding: &LinearPatternBinding,
        vm_state: &PathVmState,
        quoted: &str,
    ) -> Result<PropertyValue> {
        let expr = parse_vm_expression(quoted)?;
        let ctx = PathVmHopContext {
            source: None,
            target: None,
            edge: None,
            path_depth: binding.edges.len(),
            binding: Some(binding), // habilita path.start/end/state en el contexto final
        };
        let value = self.evaluate_path_vm_expr(&expr, vm_state, &ctx)?;
        match &value {
            PropertyValue::Object(_) | PropertyValue::List(_) => {
                Err(NopalError::QueryExecutionError(
                    "RETURN must produce a scalar value (Int, Float, Bool, String, or Null) in Path Queries F4-C".into()
                ))
            }
            _ => Ok(value),
        }
    }

    fn evaluate_path_vm_expr(
        &self,
        expr: &Expression,
        state: &PathVmState,
        ctx: &PathVmHopContext<'_>,
    ) -> Result<PropertyValue> {
        match expr {
            Expression::Literal(value) => Ok(value.clone()),
            Expression::Property { variable, property } => {
                self.resolve_path_vm_property(variable, property, state, ctx)
            }
            Expression::BinaryOp { left, op, right } => {
                if *op == BinaryOperator::And {
                    let left_value = self.evaluate_path_vm_expr(left, state, ctx)?;
                    let PropertyValue::Bool(left_bool) = left_value else {
                        return Err(NopalError::QueryExecutionError(
                            "VM operator 'and' requires Bool operands".into(),
                        ));
                    };
                    if !left_bool {
                        return Ok(PropertyValue::Bool(false));
                    }

                    let right_value = self.evaluate_path_vm_expr(right, state, ctx)?;
                    let PropertyValue::Bool(right_bool) = right_value else {
                        return Err(NopalError::QueryExecutionError(
                            "VM operator 'and' requires Bool operands".into(),
                        ));
                    };
                    return Ok(PropertyValue::Bool(right_bool));
                }

                if *op == BinaryOperator::Or {
                    let left_value = self.evaluate_path_vm_expr(left, state, ctx)?;
                    let PropertyValue::Bool(left_bool) = left_value else {
                        return Err(NopalError::QueryExecutionError(
                            "VM operator 'or' requires Bool operands".into(),
                        ));
                    };
                    if left_bool {
                        return Ok(PropertyValue::Bool(true));
                    }

                    let right_value = self.evaluate_path_vm_expr(right, state, ctx)?;
                    let PropertyValue::Bool(right_bool) = right_value else {
                        return Err(NopalError::QueryExecutionError(
                            "VM operator 'or' requires Bool operands".into(),
                        ));
                    };
                    return Ok(PropertyValue::Bool(right_bool));
                }

                let left_value = self.evaluate_path_vm_expr(left, state, ctx)?;
                let right_value = self.evaluate_path_vm_expr(right, state, ctx)?;
                self.evaluate_path_vm_binary(op, left_value, right_value)
            }
            Expression::UnaryOp { op, expr } => {
                let value = self.evaluate_path_vm_expr(expr, state, ctx)?;
                self.evaluate_path_vm_unary(op, value)
            }
            Expression::FunctionCall { .. } => Err(NopalError::QueryExecutionError(
                "Function calls are not supported inside quoted VM expressions in Path Queries F4-B".into(),
            )),
            Expression::Wildcard => Err(NopalError::QueryExecutionError(
                "Wildcard is not supported inside quoted VM expressions in Path Queries F4-B".into(),
            )),
        }
    }

    fn resolve_path_vm_property(
        &self,
        variable: &str,
        property: &str,
        state: &PathVmState,
        ctx: &PathVmHopContext<'_>,
    ) -> Result<PropertyValue> {
        if property.is_empty() {
            return state
                .vars
                .get(variable)
                .cloned()
                .ok_or_else(|| NopalError::QueryExecutionError(format!(
                    "VM variable '{}' is not defined; initialize it with INIT or assign it earlier in GATHER",
                    variable
                )));
        }

        match variable {
            "path" => match property {
                "depth" => Ok(PropertyValue::Int(ctx.path_depth as i64)),
                "start" => {
                    let binding = ctx.binding.ok_or_else(|| NopalError::QueryExecutionError(
                        "path.start is not available inside INIT or GATHER expressions; use it in RETURN or FIND".into()
                    ))?;
                    let node = binding.nodes.first().ok_or_else(|| {
                        NopalError::QueryExecutionError("path.start: path has no nodes".into())
                    })?;
                    Ok(build_path_node_object(node))
                }
                "end" => {
                    let binding = ctx.binding.ok_or_else(|| NopalError::QueryExecutionError(
                        "path.end is not available inside INIT or GATHER expressions; use it in RETURN or FIND".into()
                    ))?;
                    let node = binding.nodes.last().ok_or_else(|| {
                        NopalError::QueryExecutionError("path.end: path has no nodes".into())
                    })?;
                    Ok(build_path_node_object(node))
                }
                "state" => {
                    // Expone el estado final del mini-VM como Object (disponible en RETURN)
                    let entries: Vec<(String, PropertyValue)> = state
                        .vars
                        .iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect();
                    Ok(PropertyValue::Object(entries))
                }
                "result" => Err(NopalError::QueryExecutionError(
                    "path.result cannot be used inside a RETURN expression in Path Queries F4-C"
                        .into(),
                )),
                _ => Err(NopalError::QueryExecutionError(format!(
                    "VM path property 'path.{}' is not supported in Path Queries F4-B",
                    property
                ))),
            },
            "edge" => {
                let edge = ctx.edge.ok_or_else(|| {
                    NopalError::QueryExecutionError(
                        "VM context 'edge' is only available while executing GATHER clauses".into(),
                    )
                })?;
                match property {
                    "id" => Ok(PropertyValue::String(edge.id.to_string())),
                    "type" | "edge_type" => Ok(PropertyValue::String(edge.edge_type.clone())),
                    prop => edge.properties.get(prop).cloned().ok_or_else(|| {
                        NopalError::QueryExecutionError(format!(
                            "VM property 'edge.{}' is missing on edge {}",
                            prop, edge.id
                        ))
                    }),
                }
            }
            "source" => {
                let node = ctx.source.ok_or_else(|| {
                    NopalError::QueryExecutionError(
                        "VM context 'source' is only available while executing GATHER clauses"
                            .into(),
                    )
                })?;
                match property {
                    "id" => Ok(PropertyValue::String(node.id.to_string())),
                    "label" => Ok(PropertyValue::String(node.label.clone())),
                    prop => node.properties.get(prop).cloned().ok_or_else(|| {
                        NopalError::QueryExecutionError(format!(
                            "VM property 'source.{}' is missing on node {}",
                            prop, node.id
                        ))
                    }),
                }
            }
            "target" => {
                let node = ctx.target.ok_or_else(|| {
                    NopalError::QueryExecutionError(
                        "VM context 'target' is only available while executing GATHER clauses"
                            .into(),
                    )
                })?;
                match property {
                    "id" => Ok(PropertyValue::String(node.id.to_string())),
                    "label" => Ok(PropertyValue::String(node.label.clone())),
                    prop => node.properties.get(prop).cloned().ok_or_else(|| {
                        NopalError::QueryExecutionError(format!(
                            "VM property 'target.{}' is missing on node {}",
                            prop, node.id
                        ))
                    }),
                }
            }
            other => Err(NopalError::QueryExecutionError(format!(
                "VM object '{}' is not supported in Path Queries F4-B",
                other
            ))),
        }
    }

    fn evaluate_path_vm_binary(
        &self,
        op: &BinaryOperator,
        left: PropertyValue,
        right: PropertyValue,
    ) -> Result<PropertyValue> {
        match op {
            BinaryOperator::Eq => Ok(PropertyValue::Bool(left == right)),
            BinaryOperator::NotEq => Ok(PropertyValue::Bool(left != right)),
            BinaryOperator::Gt
            | BinaryOperator::Lt
            | BinaryOperator::GtEq
            | BinaryOperator::LtEq => {
                let result = match (&left, &right) {
                    (PropertyValue::Int(a), PropertyValue::Int(b)) => match op {
                        BinaryOperator::Gt => a > b,
                        BinaryOperator::Lt => a < b,
                        BinaryOperator::GtEq => a >= b,
                        BinaryOperator::LtEq => a <= b,
                        _ => unreachable!(),
                    },
                    (PropertyValue::Float(a), PropertyValue::Float(b)) => match op {
                        BinaryOperator::Gt => a > b,
                        BinaryOperator::Lt => a < b,
                        BinaryOperator::GtEq => a >= b,
                        BinaryOperator::LtEq => a <= b,
                        _ => unreachable!(),
                    },
                    (PropertyValue::Int(a), PropertyValue::Float(b)) => match op {
                        BinaryOperator::Gt => (*a as f64) > *b,
                        BinaryOperator::Lt => (*a as f64) < *b,
                        BinaryOperator::GtEq => (*a as f64) >= *b,
                        BinaryOperator::LtEq => (*a as f64) <= *b,
                        _ => unreachable!(),
                    },
                    (PropertyValue::Float(a), PropertyValue::Int(b)) => match op {
                        BinaryOperator::Gt => *a > (*b as f64),
                        BinaryOperator::Lt => *a < (*b as f64),
                        BinaryOperator::GtEq => *a >= (*b as f64),
                        BinaryOperator::LtEq => *a <= (*b as f64),
                        _ => unreachable!(),
                    },
                    (PropertyValue::String(a), PropertyValue::String(b)) => match op {
                        BinaryOperator::Gt => a > b,
                        BinaryOperator::Lt => a < b,
                        BinaryOperator::GtEq => a >= b,
                        BinaryOperator::LtEq => a <= b,
                        _ => unreachable!(),
                    },
                    (PropertyValue::Bool(a), PropertyValue::Bool(b)) => match op {
                        BinaryOperator::Gt => a > b,
                        BinaryOperator::Lt => a < b,
                        BinaryOperator::GtEq => a >= b,
                        BinaryOperator::LtEq => a <= b,
                        _ => unreachable!(),
                    },
                    _ => {
                        return Err(NopalError::QueryExecutionError(format!(
                            "VM comparison '{:?}' is not supported for values {:?} and {:?}",
                            op, left, right
                        )));
                    }
                };
                Ok(PropertyValue::Bool(result))
            }
            BinaryOperator::Add
            | BinaryOperator::Sub
            | BinaryOperator::Mul
            | BinaryOperator::Div
            | BinaryOperator::Mod => match (left, right) {
                (PropertyValue::Int(a), PropertyValue::Int(b)) => match op {
                    BinaryOperator::Add => Ok(PropertyValue::Int(a + b)),
                    BinaryOperator::Sub => Ok(PropertyValue::Int(a - b)),
                    BinaryOperator::Mul => Ok(PropertyValue::Int(a * b)),
                    BinaryOperator::Div => {
                        if b == 0 {
                            Err(NopalError::QueryExecutionError(
                                "VM division by zero".into(),
                            ))
                        } else {
                            Ok(PropertyValue::Float(a as f64 / b as f64))
                        }
                    }
                    BinaryOperator::Mod => {
                        if b == 0 {
                            Err(NopalError::QueryExecutionError("VM modulo by zero".into()))
                        } else {
                            Ok(PropertyValue::Int(a % b))
                        }
                    }
                    _ => unreachable!(),
                },
                (PropertyValue::Int(a), PropertyValue::Float(b)) => self.evaluate_path_vm_binary(
                    op,
                    PropertyValue::Float(a as f64),
                    PropertyValue::Float(b),
                ),
                (PropertyValue::Float(a), PropertyValue::Int(b)) => self.evaluate_path_vm_binary(
                    op,
                    PropertyValue::Float(a),
                    PropertyValue::Float(b as f64),
                ),
                (PropertyValue::Float(a), PropertyValue::Float(b)) => match op {
                    BinaryOperator::Add => Ok(PropertyValue::Float(a + b)),
                    BinaryOperator::Sub => Ok(PropertyValue::Float(a - b)),
                    BinaryOperator::Mul => Ok(PropertyValue::Float(a * b)),
                    BinaryOperator::Div => {
                        if b == 0.0 {
                            Err(NopalError::QueryExecutionError(
                                "VM division by zero".into(),
                            ))
                        } else {
                            Ok(PropertyValue::Float(a / b))
                        }
                    }
                    BinaryOperator::Mod => {
                        if b == 0.0 {
                            Err(NopalError::QueryExecutionError("VM modulo by zero".into()))
                        } else {
                            Ok(PropertyValue::Float(a % b))
                        }
                    }
                    _ => unreachable!(),
                },
                (left, right) => Err(NopalError::QueryExecutionError(format!(
                    "VM arithmetic operator '{:?}' requires numeric operands, got {:?} and {:?}",
                    op, left, right
                ))),
            },
            BinaryOperator::And | BinaryOperator::Or => Err(NopalError::QueryExecutionError(
                "VM logical operators are handled before binary evaluation".into(),
            )),
        }
    }

    fn evaluate_path_vm_unary(
        &self,
        op: &UnaryOperator,
        value: PropertyValue,
    ) -> Result<PropertyValue> {
        match op {
            UnaryOperator::Not => match value {
                PropertyValue::Bool(value) => Ok(PropertyValue::Bool(!value)),
                other => Err(NopalError::QueryExecutionError(format!(
                    "VM operator 'not' requires Bool, got {:?}",
                    other
                ))),
            },
            UnaryOperator::Neg => match value {
                PropertyValue::Int(value) => Ok(PropertyValue::Int(-value)),
                PropertyValue::Float(value) => Ok(PropertyValue::Float(-value)),
                other => Err(NopalError::QueryExecutionError(format!(
                    "VM unary '-' requires numeric operand, got {:?}",
                    other
                ))),
            },
        }
    }

    fn evaluate_linear_pattern_expression_with_vm(
        &self,
        binding: &LinearPatternBinding,
        expr: &Expression,
        vm_state: Option<&PathVmState>,
    ) -> Result<Option<PropertyValue>> {
        match expr {
            Expression::Literal(_) | Expression::Property { .. } => {
                if let Expression::Property { variable, property } = expr
                    && property.is_empty()
                    && let Some(state) = vm_state
                    && let Some(value) = state.vars.get(variable)
                {
                    return Ok(Some(value.clone()));
                }
                // F4-C: new path.* properties available after RETURN evaluation
                if let Expression::Property { variable, property } = expr
                    && variable == "path"
                {
                    match property.as_str() {
                        "start" => {
                            return Ok(binding.nodes.first().map(build_path_node_object));
                        }
                        "end" => {
                            return Ok(binding.nodes.last().map(build_path_node_object));
                        }
                        "state" => {
                            return Ok(vm_state.map(|s| {
                                let entries: Vec<(String, PropertyValue)> =
                                    s.vars.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                                PropertyValue::Object(entries)
                            }));
                        }
                        "result" => {
                            return if let Some(state) = vm_state {
                                Ok(state.return_result.clone())
                            } else {
                                Err(NopalError::QueryExecutionError(
                                    "path.result requires a RETURN clause in Path Queries F4-C"
                                        .into(),
                                ))
                            };
                        }
                        _ => {} // fall through: path.depth/nodes/edges handled by evaluate_linear_pattern_expression
                    }
                }
                Ok(self.evaluate_linear_pattern_expression(binding, expr))
            }
            Expression::FunctionCall { name, args } if is_path_reducer(name) => {
                let prop = Self::extract_reducer_prop(args)?;
                Ok(Some(Self::evaluate_path_reducer(binding, name, prop)?))
            }
            Expression::FunctionCall { name, args } if is_path_eval(name) => {
                let quoted = Self::extract_path_eval_expr(args)?;
                Ok(Some(self.evaluate_path_eval(binding, vm_state, quoted)?))
            }
            Expression::FunctionCall { name, args } if is_path_semantic_filter(name) => Ok(Some(
                self.evaluate_path_semantic_filter(binding, name, args)?,
            )),
            Expression::FunctionCall { name, args } if is_path_embedding_fn(name) => Ok(Some(
                self.evaluate_path_embedding_function(binding, name, args)?,
            )),
            Expression::FunctionCall { .. } => Ok(None),
            Expression::BinaryOp { left, op, right } => {
                if *op == BinaryOperator::And || *op == BinaryOperator::Or {
                    let value =
                        self.evaluate_linear_pattern_condition_with_vm(binding, expr, vm_state)?;
                    return Ok(Some(PropertyValue::Bool(value)));
                }

                let left_value =
                    self.evaluate_linear_pattern_expression_with_vm(binding, left, vm_state)?;
                let right_value =
                    self.evaluate_linear_pattern_expression_with_vm(binding, right, vm_state)?;

                match (left_value, right_value) {
                    (Some(left), Some(right)) => {
                        Ok(Some(self.evaluate_path_vm_binary(op, left, right)?))
                    }
                    _ => Ok(None),
                }
            }
            Expression::UnaryOp { op, expr } => {
                let value =
                    self.evaluate_linear_pattern_expression_with_vm(binding, expr, vm_state)?;
                match value {
                    Some(value) => Ok(Some(self.evaluate_path_vm_unary(op, value)?)),
                    None => Ok(None),
                }
            }
            Expression::Wildcard => Ok(None),
        }
    }

    fn evaluate_linear_pattern_condition_with_vm(
        &self,
        binding: &LinearPatternBinding,
        expr: &Expression,
        vm_state: Option<&PathVmState>,
    ) -> Result<bool> {
        if let Expression::BinaryOp { left, op, right } = expr {
            match op {
                BinaryOperator::And => {
                    return Ok(self
                        .evaluate_linear_pattern_condition_with_vm(binding, left, vm_state)?
                        && self.evaluate_linear_pattern_condition_with_vm(
                            binding, right, vm_state,
                        )?);
                }
                BinaryOperator::Or => {
                    return Ok(self
                        .evaluate_linear_pattern_condition_with_vm(binding, left, vm_state)?
                        || self.evaluate_linear_pattern_condition_with_vm(
                            binding, right, vm_state,
                        )?);
                }
                _ => {}
            }
        }

        match self.evaluate_linear_pattern_expression_with_vm(binding, expr, vm_state)? {
            Some(PropertyValue::Bool(value)) => Ok(value),
            Some(other) => Err(NopalError::QueryExecutionError(format!(
                "Path-aware WHERE expression must evaluate to Bool, got {:?}",
                other
            ))),
            None => Ok(false),
        }
    }

    // ─── End F3 ──────────────────────────────────────────────────────────────

    fn project_linear_pattern_binding(
        &self,
        binding: &LinearPatternBinding,
        projections: &[String],
    ) -> Row {
        let mut row = Row::new();

        for proj in projections {
            if proj == "*" {
                for (var, node) in &binding.node_vars {
                    self.populate_row_with_node(&mut row, var, node);
                }
                for (var, edge) in &binding.edge_vars {
                    self.populate_row_with_edge(&mut row, var, edge);
                }
                continue;
            }

            let parts: Vec<&str> = proj.split('.').collect();
            if parts.len() == 2 {
                let var = parts[0];
                let prop = parts[1];

                if prop == "*" {
                    if let Some(node) = binding.node_vars.get(var) {
                        self.populate_row_with_node(&mut row, var, node);
                    } else if let Some(edge) = binding.edge_vars.get(var) {
                        self.populate_row_with_edge(&mut row, var, edge);
                    }
                    continue;
                }

                if let Some(value) = self.evaluate_linear_pattern_expression(
                    binding,
                    &Expression::Property {
                        variable: var.to_string(),
                        property: prop.to_string(),
                    },
                ) {
                    row.set(proj.clone(), value);
                }
            } else if parts.len() == 1
                && let Some(value) = self.evaluate_linear_pattern_expression(
                    binding,
                    &Expression::Property {
                        variable: parts[0].to_string(),
                        property: String::new(),
                    },
                )
            {
                row.set(proj.clone(), value);
            }
        }

        row
    }

    fn populate_row_with_node(&self, row: &mut Row, var: &str, node: &Node) {
        for (key, value) in &node.properties {
            row.set(format!("{}.{}", var, key), value.clone());
        }
        row.set(
            format!("{}.label", var),
            PropertyValue::String(node.label.clone()),
        );
        row.set(
            format!("{}.id", var),
            PropertyValue::String(node.id.to_string()),
        );
    }

    fn populate_row_with_edge(&self, row: &mut Row, var: &str, edge: &Edge) {
        for (key, value) in &edge.properties {
            row.set(format!("{}.{}", var, key), value.clone());
        }
        row.set(
            format!("{}.type", var),
            PropertyValue::String(edge.edge_type.clone()),
        );
        row.set(
            format!("{}.id", var),
            PropertyValue::String(edge.id.to_string()),
        );
    }

    fn execute_pattern_aggregations(
        &self,
        matches: &[operators::PatternMatch],
        query: &Query,
        source_var: &str,
        target_var: &str,
        edge_var: Option<&str>,
        algo_cache: &aggregations::AlgoResults,
    ) -> Result<QueryResult> {
        let columns: Vec<String> = query
            .find
            .projections
            .iter()
            .enumerate()
            .map(|(idx, p)| match p {
                Projection::Expression { expr, alias } => {
                    if let Some(a) = alias {
                        a.clone()
                    } else if let Expression::Property {
                        variable: v,
                        property: prop,
                    } = expr
                    {
                        if prop.is_empty() {
                            v.clone()
                        } else {
                            property_projection_key(v, prop)
                        }
                    } else if let Expression::FunctionCall { name, .. } = expr {
                        name.clone()
                    } else {
                        format!("col_{}", idx)
                    }
                }
                Projection::Wildcard => "*".to_string(),
                Projection::All(var) => format!("all({})", var),
            })
            .collect();

        let mut grouped: std::collections::HashMap<
            String,
            (Vec<PropertyValue>, Vec<&operators::PatternMatch>),
        > = std::collections::HashMap::new();

        if let Some(group_by) = &query.group_by {
            for m in matches {
                let key_values: Vec<PropertyValue> = group_by
                    .expressions
                    .iter()
                    .map(|expr| {
                        self.evaluate_pattern_expression(m, expr, source_var, target_var)
                            .unwrap_or(PropertyValue::Null)
                    })
                    .collect();

                let key = key_values
                    .iter()
                    .map(|v| format!("{:?}", v))
                    .collect::<Vec<_>>()
                    .join("|");
                grouped
                    .entry(key)
                    .or_insert_with(|| (key_values, Vec::new()))
                    .1
                    .push(m);
            }
        } else {
            grouped.insert(
                "__all__".to_string(),
                (Vec::new(), matches.iter().collect()),
            );
        }

        let mut result = QueryResult::new(columns.clone());

        for (_group_key, (group_key_values, group_matches)) in grouped {
            if group_matches.is_empty() {
                continue;
            }

            if let Some(having) = &query.having
                && !self.evaluate_pattern_group_condition(
                    &group_matches,
                    &having.condition,
                    source_var,
                    target_var,
                    edge_var,
                    query.group_by.as_ref(),
                    &group_key_values,
                    algo_cache,
                )?
            {
                continue;
            }

            let mut row = Row::new();

            for (idx, projection) in query.find.projections.iter().enumerate() {
                match projection {
                    Projection::Expression { expr, alias } => {
                        let key = columns[idx].clone();

                        let value = if self.is_pattern_aggregation_expr(expr) {
                            self.evaluate_pattern_aggregation(
                                expr,
                                &group_matches,
                                source_var,
                                target_var,
                                edge_var,
                                algo_cache,
                            )?
                        } else {
                            self.evaluate_pattern_expression(
                                group_matches[0],
                                expr,
                                source_var,
                                target_var,
                            )
                            .unwrap_or(PropertyValue::Null)
                        };

                        let output_key = alias.clone().unwrap_or(key);
                        row.set(output_key, value);
                    }
                    Projection::Wildcard | Projection::All(_) => {
                        return Err(NopalError::QueryExecutionError(
                            "Wildcard/all() projections are not supported with pattern aggregations".into()
                        ));
                    }
                }
            }

            result.add_row(row);
        }

        Ok(result)
    }

    fn is_pattern_aggregation_expr(&self, expr: &Expression) -> bool {
        match expr {
            Expression::FunctionCall { name, .. } => {
                let lower = name.to_lowercase();
                matches!(lower.as_str(), "count" | "sum" | "avg" | "min" | "max")
                    || expr.is_algorithm()
            }
            _ => false,
        }
    }

    fn resolve_pattern_vars_for_query(
        &self,
        source_var: Option<&str>,
        target_var: Option<&str>,
        edge_var: Option<&str>,
        query: &Query,
    ) -> (String, String) {
        let mut resolved_source = source_var.unwrap_or("n").to_string();
        let mut resolved_target = target_var.unwrap_or("m").to_string();
        let source_missing = source_var.is_none();
        let target_missing = target_var.is_none();

        if !source_missing && !target_missing {
            return (resolved_source, resolved_target);
        }

        let mut vars: Vec<String> = Vec::new();
        for proj in &query.find.projections {
            match proj {
                Projection::All(var) => {
                    if !vars.contains(var) {
                        vars.push(var.clone());
                    }
                }
                Projection::Expression { expr, .. } => {
                    self.collect_property_vars(expr, &mut vars);
                }
                Projection::Wildcard => {}
            }
        }
        if let Some(filter) = &query.filter {
            self.collect_property_vars(&filter.condition, &mut vars);
        }

        if let Some(edge_name) = edge_var {
            vars.retain(|v| v != edge_name);
        }

        if source_missing && let Some(first) = vars.first() {
            resolved_source = first.clone();
        }

        if target_missing {
            let candidate = vars.into_iter().find(|v| v != &resolved_source);
            if let Some(v) = candidate {
                resolved_target = v;
            }
        }

        (resolved_source, resolved_target)
    }

    fn collect_property_vars(&self, expr: &Expression, vars: &mut Vec<String>) {
        match expr {
            Expression::Property { variable, .. } => {
                if !vars.contains(variable) {
                    vars.push(variable.clone());
                }
            }
            Expression::BinaryOp { left, right, .. } => {
                self.collect_property_vars(left, vars);
                self.collect_property_vars(right, vars);
            }
            Expression::UnaryOp { expr, .. } => {
                self.collect_property_vars(expr, vars);
            }
            Expression::FunctionCall { args, .. } => {
                for arg in args {
                    self.collect_property_vars(arg, vars);
                }
            }
            _ => {}
        }
    }

    fn evaluate_pattern_aggregation(
        &self,
        expr: &Expression,
        group_matches: &[&operators::PatternMatch],
        source_var: &str,
        target_var: &str,
        edge_var: Option<&str>,
        algo_cache: &aggregations::AlgoResults,
    ) -> Result<PropertyValue> {
        let Expression::FunctionCall { name, args } = expr else {
            return Err(NopalError::QueryExecutionError(
                "Not an aggregation expression".into(),
            ));
        };

        // Algorithm functions (degree, pagerank, community, ...) operate per
        // node. For a group, we resolve the value of the node referenced by
        // the function's argument (typically the source variable) using the
        // first match in the group. When grouping by source-node properties,
        // the source node is identical for all matches in a group.
        if expr.is_algorithm() {
            let var = match args.first() {
                Some(Expression::Property { variable, property }) if property.is_empty() => {
                    variable.clone()
                }
                _ => return Ok(PropertyValue::Null),
            };
            let Some(first) = group_matches.first() else {
                return Ok(PropertyValue::Null);
            };
            let node_id = if var == source_var {
                first.source.id
            } else if var == target_var {
                first.target.id
            } else {
                return Ok(PropertyValue::Null);
            };
            return Ok(lookup_algo_value(name, &node_id, algo_cache));
        }

        match name.to_lowercase().as_str() {
            "count" => {
                if args.is_empty() || matches!(args.first(), Some(Expression::Wildcard)) {
                    Ok(PropertyValue::Int(group_matches.len() as i64))
                } else {
                    let count = group_matches
                        .iter()
                        .filter(|m| {
                            self.evaluate_pattern_group_expression(
                                m, &args[0], source_var, target_var, edge_var,
                            )
                            .is_some_and(|v| !matches!(v, PropertyValue::Null))
                        })
                        .count() as i64;
                    Ok(PropertyValue::Int(count))
                }
            }
            "sum" | "avg" => {
                let arg = args.first().ok_or_else(|| {
                    NopalError::QueryExecutionError(format!("{}() requires an argument", name))
                })?;

                let mut total = 0.0_f64;
                let mut count = 0_usize;

                for m in group_matches {
                    if let Some(v) = self
                        .evaluate_pattern_group_expression(m, arg, source_var, target_var, edge_var)
                        && let Some(num) = self.property_value_to_f64(&v)
                    {
                        total += num;
                        count += 1;
                    }
                }

                if name.eq_ignore_ascii_case("avg") {
                    if count == 0 {
                        return Ok(PropertyValue::Null);
                    }
                    Ok(PropertyValue::Float(total / count as f64))
                } else {
                    Ok(PropertyValue::Float(total))
                }
            }
            "min" | "max" => {
                let arg = args.first().ok_or_else(|| {
                    NopalError::QueryExecutionError(format!("{}() requires an argument", name))
                })?;

                let mut best: Option<PropertyValue> = None;
                for m in group_matches {
                    if let Some(v) = self
                        .evaluate_pattern_group_expression(m, arg, source_var, target_var, edge_var)
                    {
                        match &best {
                            None => best = Some(v),
                            Some(curr) => {
                                let is_better = if name.eq_ignore_ascii_case("min") {
                                    self.compare_values(&v, &BinaryOperator::Lt, curr)
                                } else {
                                    self.compare_values(&v, &BinaryOperator::Gt, curr)
                                };
                                if is_better {
                                    best = Some(v);
                                }
                            }
                        }
                    }
                }

                Ok(best.unwrap_or(PropertyValue::Null))
            }
            _ => Err(NopalError::QueryExecutionError(format!(
                "Unsupported aggregation for relationship patterns: {}",
                name
            ))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_pattern_group_condition(
        &self,
        group_matches: &[&operators::PatternMatch],
        expr: &Expression,
        source_var: &str,
        target_var: &str,
        edge_var: Option<&str>,
        group_by: Option<&GroupByClause>,
        group_key_values: &[PropertyValue],
        algo_cache: &aggregations::AlgoResults,
    ) -> Result<bool> {
        match expr {
            Expression::BinaryOp { left, op, right } => match op {
                BinaryOperator::And => Ok(self.evaluate_pattern_group_condition(
                    group_matches,
                    left,
                    source_var,
                    target_var,
                    edge_var,
                    group_by,
                    group_key_values,
                    algo_cache,
                )? && self.evaluate_pattern_group_condition(
                    group_matches,
                    right,
                    source_var,
                    target_var,
                    edge_var,
                    group_by,
                    group_key_values,
                    algo_cache,
                )?),
                BinaryOperator::Or => Ok(self.evaluate_pattern_group_condition(
                    group_matches,
                    left,
                    source_var,
                    target_var,
                    edge_var,
                    group_by,
                    group_key_values,
                    algo_cache,
                )? || self.evaluate_pattern_group_condition(
                    group_matches,
                    right,
                    source_var,
                    target_var,
                    edge_var,
                    group_by,
                    group_key_values,
                    algo_cache,
                )?),
                _ => {
                    let left_val = self.evaluate_pattern_group_scalar(
                        group_matches,
                        left,
                        source_var,
                        target_var,
                        edge_var,
                        group_by,
                        group_key_values,
                        algo_cache,
                    )?;
                    let right_val = self.evaluate_pattern_group_scalar(
                        group_matches,
                        right,
                        source_var,
                        target_var,
                        edge_var,
                        group_by,
                        group_key_values,
                        algo_cache,
                    )?;

                    match (left_val, right_val) {
                        (Some(l), Some(r)) => Ok(self.compare_values(&l, op, &r)),
                        _ => Ok(false),
                    }
                }
            },
            _ => Ok(false),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn evaluate_pattern_group_scalar(
        &self,
        group_matches: &[&operators::PatternMatch],
        expr: &Expression,
        source_var: &str,
        target_var: &str,
        edge_var: Option<&str>,
        group_by: Option<&GroupByClause>,
        group_key_values: &[PropertyValue],
        algo_cache: &aggregations::AlgoResults,
    ) -> Result<Option<PropertyValue>> {
        match expr {
            Expression::Literal(v) => Ok(Some(v.clone())),
            Expression::FunctionCall { .. } => Ok(Some(self.evaluate_pattern_aggregation(
                expr,
                group_matches,
                source_var,
                target_var,
                edge_var,
                algo_cache,
            )?)),
            Expression::Property { .. } => {
                if let Some(group_by_clause) = group_by
                    && let Some(idx) = group_by_clause.expressions.iter().position(|e| e == expr)
                    && let Some(v) = group_key_values.get(idx)
                {
                    return Ok(Some(v.clone()));
                }

                Ok(group_matches.first().and_then(|m| {
                    self.evaluate_pattern_group_expression(
                        m, expr, source_var, target_var, edge_var,
                    )
                }))
            }
            _ => Ok(None),
        }
    }

    fn evaluate_pattern_group_expression(
        &self,
        m: &operators::PatternMatch,
        expr: &Expression,
        source_var: &str,
        target_var: &str,
        edge_var: Option<&str>,
    ) -> Option<PropertyValue> {
        match expr {
            Expression::Property { variable, property } if property.is_empty() => {
                if variable == source_var {
                    Some(PropertyValue::String(m.source.id.to_string()))
                } else if variable == target_var {
                    Some(PropertyValue::String(m.target.id.to_string()))
                } else if Some(variable.as_str()) == edge_var {
                    m.edge
                        .as_ref()
                        .map(|e| PropertyValue::String(e.id.to_string()))
                } else {
                    None
                }
            }
            _ => self.evaluate_pattern_expression(m, expr, source_var, target_var),
        }
    }

    fn property_value_to_f64(&self, value: &PropertyValue) -> Option<f64> {
        match value {
            PropertyValue::Int(i) => Some(*i as f64),
            PropertyValue::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// Apply WHERE filter
    fn apply_filter(
        &self,
        nodes: Vec<crate::types::Node>,
        condition: &Expression,
        variable: &str,
    ) -> Result<Vec<crate::types::Node>> {
        let filtered: Result<Vec<_>> = nodes
            .into_iter()
            .filter_map(
                |node| match self.evaluate_condition(&node, condition, variable) {
                    Ok(true) => Some(Ok(node)),
                    Ok(false) => None,
                    Err(err) => Some(Err(err)),
                },
            )
            .collect();

        filtered
    }

    /// Evaluate condition on a node (with AND/OR support)
    fn evaluate_condition(
        &self,
        node: &crate::types::Node,
        expr: &Expression,
        variable: &str,
    ) -> Result<bool> {
        match expr {
            Expression::BinaryOp { left, op, right } => match op {
                BinaryOperator::And => Ok(self.evaluate_condition(node, left, variable)?
                    && self.evaluate_condition(node, right, variable)?),
                BinaryOperator::Or => Ok(self.evaluate_condition(node, left, variable)?
                    || self.evaluate_condition(node, right, variable)?),
                _ => {
                    let left_val = self.evaluate_expression(node, left, variable);
                    let right_val = self.evaluate_expression(node, right, variable);
                    match (left_val, right_val) {
                        (Some(l), Some(r)) => Ok(self.compare_values(&l, op, &r)),
                        _ => Ok(false),
                    }
                }
            },
            // Embedding predicates: has_embedding(var, "model")
            #[cfg(feature = "embeddings")]
            Expression::FunctionCall { name, args } if name.to_lowercase() == "has_embedding" => {
                self.evaluate_embedding_predicate(node, args)
            }
            // Ontology predicates: instanceOf(var, "ClassName") and subClassOf(var, "ClassName")
            Expression::FunctionCall { name, args } => {
                Ok(self.evaluate_ontology_predicate(node, name, args, variable))
            }
            _ => {
                // For now, other expressions return true
                Ok(true)
            }
        }
    }

    /// Evaluate `instanceOf(var, "ClassName")` and `subClassOf(var, "ClassName")` predicates.
    ///
    /// Returns `false` if no TaxonomyIndex is registered or the class is unknown.
    fn evaluate_ontology_predicate(
        &self,
        node: &crate::types::Node,
        name: &str,
        args: &[Expression],
        variable: &str,
    ) -> bool {
        if args.len() != 2 {
            return false;
        }
        // Second argument must be a string literal naming the target class.
        let class_name = match &args[1] {
            Expression::Literal(crate::types::PropertyValue::String(s)) => s.clone(),
            _ => return false,
        };
        // First argument must reference the correct variable (or be ignored if not applicable).
        if let Expression::Property { variable: var, .. } = &args[0]
            && var != variable
        {
            return false;
        }

        // Get taxonomy snapshot (non-blocking clone).
        let Some(mut tax) = self.graph.get_taxonomy_sync() else {
            return false;
        };
        let Some(parent_id) = tax.find_by_label(&class_name) else {
            return false;
        };

        self.evaluate_node_ontology_predicate(node, name, &mut tax, parent_id)
    }

    fn evaluate_node_ontology_predicate(
        &self,
        node: &crate::types::Node,
        name: &str,
        tax: &mut crate::index::TaxonomyIndex,
        parent_id: NodeId,
    ) -> bool {
        match name.to_lowercase().as_str() {
            "instanceof" => {
                if node.kind != crate::types::NodeKind::Individual {
                    return false;
                }
                tax.is_subclass_of_label(&node.label, parent_id)
            }
            "subclassof" => {
                if node.kind != crate::types::NodeKind::Class {
                    return false;
                }
                tax.is_subclass_of_label(&node.label, parent_id)
            }
            _ => false,
        }
    }

    fn evaluate_path_semantic_filter(
        &self,
        binding: &LinearPatternBinding,
        name: &str,
        args: &[Expression],
    ) -> Result<PropertyValue> {
        if args.len() != 1 {
            return Err(NopalError::QueryExecutionError(format!(
                "{} requires exactly 1 string literal class name in Path Queries F4-D.1",
                name
            )));
        }

        let class_name = match &args[0] {
            Expression::Literal(PropertyValue::String(s)) => s.clone(),
            _ => {
                return Err(NopalError::QueryExecutionError(format!(
                    "{} requires a string literal class name in Path Queries F4-D.1",
                    name
                )));
            }
        };

        let Some(mut tax) = self.graph.get_taxonomy_sync() else {
            return Ok(PropertyValue::Bool(false));
        };
        let Some(parent_id) = tax.find_by_label(&class_name) else {
            return Ok(PropertyValue::Bool(false));
        };

        let lower = name.to_lowercase();
        let predicate = if lower.ends_with("instanceof") {
            "instanceof"
        } else if lower.ends_with("subclassof") {
            "subclassof"
        } else {
            return Err(NopalError::QueryExecutionError(format!(
                "Unsupported semantic path filter '{}' in Path Queries F4-D.1",
                name
            )));
        };

        let matches_node =
            |node: &crate::types::Node, this: &Self, tax: &mut crate::index::TaxonomyIndex| {
                this.evaluate_node_ontology_predicate(node, predicate, tax, parent_id)
            };

        let result = match lower.as_str() {
            "path_start_instanceof" | "path_start_subclassof" => binding
                .nodes
                .first()
                .is_some_and(|node| matches_node(node, self, &mut tax)),
            "path_end_instanceof" | "path_end_subclassof" => binding
                .nodes
                .last()
                .is_some_and(|node| matches_node(node, self, &mut tax)),
            "path_any_instanceof" | "path_any_subclassof" => binding
                .nodes
                .iter()
                .any(|node| matches_node(node, self, &mut tax)),
            "path_all_instanceof" | "path_all_subclassof" => binding
                .nodes
                .iter()
                .all(|node| matches_node(node, self, &mut tax)),
            _ => {
                return Err(NopalError::QueryExecutionError(format!(
                    "Unsupported semantic path filter '{}' in Path Queries F4-D.1",
                    name
                )));
            }
        };

        Ok(PropertyValue::Bool(result))
    }

    #[cfg(feature = "embeddings")]
    fn path_has_embeddings(
        &self,
        binding: &LinearPatternBinding,
        node_model: &str,
        edge_model: &str,
    ) -> Result<bool> {
        if binding.nodes.is_empty() || binding.edges.is_empty() {
            return Ok(false);
        }
        for node in &binding.nodes {
            if !self
                .graph
                .try_node_embedding_exists_sync(node.id, node_model)?
            {
                return Ok(false);
            }
        }
        for edge in &binding.edges {
            if !self
                .graph
                .try_edge_embedding_exists_sync(edge.id, edge_model)?
            {
                return Ok(false);
            }
        }
        Ok(true)
    }

    #[cfg(feature = "embeddings")]
    fn pattern_has_embeddings(
        &self,
        binding: &LinearPatternBinding,
        node_model: &str,
        edge_model: &str,
    ) -> Result<bool> {
        if binding.nodes.is_empty() || binding.edges.is_empty() {
            return Ok(false);
        }

        for node in &binding.nodes {
            if !self
                .graph
                .try_node_embedding_exists_sync(node.id, node_model)?
            {
                return Ok(false);
            }
        }
        for edge in &binding.edges {
            if !self
                .graph
                .try_edge_embedding_exists_sync(edge.id, edge_model)?
            {
                return Ok(false);
            }
        }

        Ok(true)
    }

    #[cfg(feature = "embeddings")]
    fn build_path_embedding_vector(
        &self,
        binding: &LinearPatternBinding,
        node_model: &str,
        edge_model: &str,
    ) -> Result<Vec<f32>> {
        if binding.nodes.is_empty() || binding.edges.is_empty() {
            return Err(NopalError::QueryExecutionError(
                "PathEmbedding requires a materialized linear path with at least one relationship in E-7".into(),
            ));
        }

        if binding.nodes.len() != binding.edges.len() + 1 {
            return Err(NopalError::QueryExecutionError(format!(
                "PathEmbedding E-7 expects nodes = edges + 1, got {} nodes and {} edges",
                binding.nodes.len(),
                binding.edges.len()
            )));
        }

        let mut node_vectors: Vec<Vec<f32>> = Vec::with_capacity(binding.nodes.len());
        for node in &binding.nodes {
            let emb = self.graph.get_node_embedding_sync(node.id, node_model)?;
            node_vectors.push(emb.vector);
        }

        let node_dim = node_vectors.first().map(|v| v.len()).ok_or_else(|| {
            NopalError::QueryExecutionError("PathEmbedding E-7: empty node vector set".into())
        })?;

        if node_vectors.iter().any(|v| v.len() != node_dim) {
            return Err(NopalError::QueryExecutionError(format!(
                "PathEmbedding E-7: inconsistent node embedding dimensions for model '{}'",
                node_model
            )));
        }

        let mut mean_nodes = vec![0.0f32; node_dim];
        for vector in &node_vectors {
            for (i, value) in vector.iter().enumerate() {
                mean_nodes[i] += *value;
            }
        }
        let node_count = node_vectors.len() as f32;
        for value in &mut mean_nodes {
            *value /= node_count;
        }

        let mut edge_vectors: Vec<Vec<f32>> = Vec::with_capacity(binding.edges.len());
        for edge in &binding.edges {
            let emb = self.graph.get_edge_embedding_sync(edge.id, edge_model)?;
            edge_vectors.push(emb.vector);
        }

        let edge_dim = edge_vectors.first().map(|v| v.len()).ok_or_else(|| {
            NopalError::QueryExecutionError("PathEmbedding E-7: empty edge vector set".into())
        })?;

        if edge_vectors.iter().any(|v| v.len() != edge_dim) {
            return Err(NopalError::QueryExecutionError(format!(
                "PathEmbedding E-7: inconsistent edge embedding dimensions for model '{}'",
                edge_model
            )));
        }

        let mut mean_edges = vec![0.0f32; edge_dim];
        for vector in &edge_vectors {
            for (i, value) in vector.iter().enumerate() {
                mean_edges[i] += *value;
            }
        }
        let edge_count = edge_vectors.len() as f32;
        for value in &mut mean_edges {
            *value /= edge_count;
        }

        let mut combined = mean_nodes;
        combined.extend(mean_edges);
        Ok(combined)
    }

    #[cfg(feature = "embeddings")]
    fn build_pattern_embedding_vector(
        &self,
        binding: &LinearPatternBinding,
        node_model: &str,
        edge_model: &str,
    ) -> Result<Vec<f32>> {
        if binding.nodes.is_empty() || binding.edges.is_empty() {
            return Err(NopalError::QueryExecutionError(
                "PatternEmbedding requires a materialized linear path with at least one relationship in E-3".into(),
            ));
        }

        if binding.nodes.len() != binding.edges.len() + 1 {
            return Err(NopalError::QueryExecutionError(format!(
                "PatternEmbedding E-3 expects nodes = edges + 1, got {} nodes and {} edges",
                binding.nodes.len(),
                binding.edges.len()
            )));
        }

        let mut combined = Vec::new();
        let mut node_dim: Option<usize> = None;
        let mut edge_dim: Option<usize> = None;

        for (index, node) in binding.nodes.iter().enumerate() {
            let embedding = self.graph.get_node_embedding_sync(node.id, node_model)?;
            let current_node_dim = embedding.vector.len();
            if let Some(expected) = node_dim {
                if expected != current_node_dim {
                    return Err(NopalError::QueryExecutionError(format!(
                        "PatternEmbedding E-3: inconsistent node embedding dimensions for model '{}': expected {}, got {}",
                        node_model, expected, current_node_dim
                    )));
                }
            } else {
                node_dim = Some(current_node_dim);
            }
            combined.extend(embedding.vector.into_iter());

            if index < binding.edges.len() {
                let edge = &binding.edges[index];
                let embedding = self.graph.get_edge_embedding_sync(edge.id, edge_model)?;
                let current_edge_dim = embedding.vector.len();
                if let Some(expected) = edge_dim {
                    if expected != current_edge_dim {
                        return Err(NopalError::QueryExecutionError(format!(
                            "PatternEmbedding E-3: inconsistent edge embedding dimensions for model '{}': expected {}, got {}",
                            edge_model, expected, current_edge_dim
                        )));
                    }
                } else {
                    edge_dim = Some(current_edge_dim);
                }
                combined.extend(embedding.vector.into_iter());
            }
        }

        Ok(combined)
    }

    fn evaluate_path_embedding_function(
        &self,
        binding: &LinearPatternBinding,
        name: &str,
        args: &[Expression],
    ) -> Result<PropertyValue> {
        #[cfg(feature = "embeddings")]
        {
            let lower = name.to_lowercase();
            match lower.as_str() {
                "path_has_embeddings" => {
                    if args.len() == 1 {
                        Err(NopalError::QueryExecutionError(
                            "path_has_embeddings(\"model\") was replaced in PathEmbedding E-7; use path_has_embeddings(node_model, edge_model)".into(),
                        ))
                    } else if args.len() != 2 {
                        Err(NopalError::QueryExecutionError(
                            "path_has_embeddings(node_model, edge_model) requires exactly 2 string literal model names in PathEmbedding E-7".into(),
                        ))
                    } else {
                        let node_model = match &args[0] {
                            Expression::Literal(PropertyValue::String(s)) => s.clone(),
                            _ => {
                                return Err(NopalError::QueryExecutionError(
                                    "path_has_embeddings(node_model, edge_model) requires a string literal node model as first argument in PathEmbedding E-7".into(),
                                ))
                            }
                        };
                        let edge_model = match &args[1] {
                            Expression::Literal(PropertyValue::String(s)) => s.clone(),
                            _ => {
                                return Err(NopalError::QueryExecutionError(
                                    "path_has_embeddings(node_model, edge_model) requires a string literal edge model as second argument in PathEmbedding E-7".into(),
                                ))
                            }
                        };
                        Ok(PropertyValue::Bool(
                            self.path_has_embeddings(binding, &node_model, &edge_model)?,
                        ))
                    }
                }
                "path_embedding" => {
                    if args.len() != 2 {
                        Err(NopalError::QueryExecutionError(
                            "path_embedding(node_model, edge_model) requires exactly 2 string literal model names in PathEmbedding E-7".into(),
                        ))
                    } else {
                        let node_model = match &args[0] {
                            Expression::Literal(PropertyValue::String(s)) => s.clone(),
                            _ => {
                                return Err(NopalError::QueryExecutionError(
                                    "path_embedding(node_model, edge_model) requires a string literal node model as first argument in PathEmbedding E-7".into(),
                                ))
                            }
                        };
                        let edge_model = match &args[1] {
                            Expression::Literal(PropertyValue::String(s)) => s.clone(),
                            _ => {
                                return Err(NopalError::QueryExecutionError(
                                    "path_embedding(node_model, edge_model) requires a string literal edge model as second argument in PathEmbedding E-7".into(),
                                ))
                            }
                        };
                        let vector =
                            self.build_path_embedding_vector(binding, &node_model, &edge_model)?;
                        Ok(PropertyValue::List(
                            vector
                                .into_iter()
                                .map(|value| PropertyValue::Float(value as f64))
                                .collect(),
                        ))
                    }
                }
                "pattern_has_embeddings" => {
                    if args.len() == 1 {
                        Err(NopalError::QueryExecutionError(
                            "pattern_has_embeddings(\"model\") was replaced in PatternEmbedding E-3; use pattern_has_embeddings(node_model, edge_model)".into(),
                        ))
                    } else if args.len() != 2 {
                        Err(NopalError::QueryExecutionError(
                            "pattern_has_embeddings(node_model, edge_model) requires exactly 2 string literal model names in PatternEmbedding E-3".into(),
                        ))
                    } else {
                        let node_model = match &args[0] {
                            Expression::Literal(PropertyValue::String(s)) => s.clone(),
                            _ => {
                                return Err(NopalError::QueryExecutionError(
                                    "pattern_has_embeddings(node_model, edge_model) requires a string literal node model as first argument in PatternEmbedding E-3".into(),
                                ))
                            }
                        };
                        let edge_model = match &args[1] {
                            Expression::Literal(PropertyValue::String(s)) => s.clone(),
                            _ => {
                                return Err(NopalError::QueryExecutionError(
                                    "pattern_has_embeddings(node_model, edge_model) requires a string literal edge model as second argument in PatternEmbedding E-3".into(),
                                ))
                            }
                        };
                        Ok(PropertyValue::Bool(
                            self.pattern_has_embeddings(binding, &node_model, &edge_model)?,
                        ))
                    }
                }
                "pattern_embedding" => {
                    if args.len() != 2 {
                        Err(NopalError::QueryExecutionError(
                            "pattern_embedding(node_model, edge_model) requires exactly 2 string literal model names in PatternEmbedding E-3".into(),
                        ))
                    } else {
                        let node_model = match &args[0] {
                            Expression::Literal(PropertyValue::String(s)) => s.clone(),
                            _ => {
                                return Err(NopalError::QueryExecutionError(
                                    "pattern_embedding(node_model, edge_model) requires a string literal node model as first argument in PatternEmbedding E-3".into(),
                                ))
                            }
                        };
                        let edge_model = match &args[1] {
                            Expression::Literal(PropertyValue::String(s)) => s.clone(),
                            _ => {
                                return Err(NopalError::QueryExecutionError(
                                    "pattern_embedding(node_model, edge_model) requires a string literal edge model as second argument in PatternEmbedding E-3".into(),
                                ))
                            }
                        };
                        let vector =
                            self.build_pattern_embedding_vector(binding, &node_model, &edge_model)?;
                        Ok(PropertyValue::List(
                            vector
                                .into_iter()
                                .map(|value| PropertyValue::Float(value as f64))
                                .collect(),
                        ))
                    }
                }
                "path_embedding_similarity" => {
                    // E-8: path_embedding_similarity(ref_name, node_model, edge_model)
                    if args.len() != 3 {
                        return Err(NopalError::QueryExecutionError(format!(
                            "path_embedding_similarity requires exactly 3 arguments: \
                             path_embedding_similarity(ref_name, node_model, edge_model), got {}",
                            args.len()
                        )));
                    }
                    let ref_name = match &args[0] {
                        Expression::Literal(PropertyValue::String(s)) => s.clone(),
                        _ => return Err(NopalError::QueryExecutionError(
                            "path_embedding_similarity: ref_name (arg 1) must be a string literal".into()
                        )),
                    };
                    let node_model = match &args[1] {
                        Expression::Literal(PropertyValue::String(s)) => s.clone(),
                        _ => return Err(NopalError::QueryExecutionError(
                            "path_embedding_similarity: node_model (arg 2) must be a string literal".into()
                        )),
                    };
                    let edge_model = match &args[2] {
                        Expression::Literal(PropertyValue::String(s)) => s.clone(),
                        _ => return Err(NopalError::QueryExecutionError(
                            "path_embedding_similarity: edge_model (arg 3) must be a string literal".into()
                        )),
                    };
                    let path_vec = self.build_path_embedding_vector(binding, &node_model, &edge_model)?;
                    let reference = self.graph.get_path_reference_embedding_sync(&ref_name, &node_model, &edge_model)?;
                    if path_vec.len() != reference.vector.len() {
                        return Err(NopalError::QueryExecutionError(format!(
                            "path_embedding_similarity: dimension mismatch — path vector has {} dimensions, \
                             reference '{}' has {} dimensions",
                            path_vec.len(), ref_name, reference.vector.len()
                        )));
                    }
                    let score = cosine_similarity_f32(&path_vec, &reference.vector)?;
                    Ok(PropertyValue::Float(score as f64))
                }
                "path_knn_references" => {
                    // E-9: path_knn_references(node_model, edge_model, k, min_score)
                    if args.len() != 4 {
                        return Err(NopalError::QueryExecutionError(format!(
                            "path_knn_references requires exactly 4 arguments, got {}",
                            args.len()
                        )));
                    }
                    let node_model = match &args[0] {
                        Expression::Literal(PropertyValue::String(s)) => s.clone(),
                        _ => return Err(NopalError::QueryExecutionError(
                            "path_knn_references: node_model (arg 1) must be a string literal".into()
                        )),
                    };
                    let edge_model = match &args[1] {
                        Expression::Literal(PropertyValue::String(s)) => s.clone(),
                        _ => return Err(NopalError::QueryExecutionError(
                            "path_knn_references: edge_model (arg 2) must be a string literal".into()
                        )),
                    };
                    let k = match &args[2] {
                        Expression::Literal(PropertyValue::Int(n)) if *n > 0 => *n as usize,
                        _ => return Err(NopalError::QueryExecutionError(
                            "path_knn_references: k (arg 3) must be a positive integer literal".into()
                        )),
                    };
                    let min_score = match &args[3] {
                        Expression::Literal(PropertyValue::Float(f)) => *f as f32,
                        Expression::Literal(PropertyValue::Int(i)) => *i as f32,
                        _ => return Err(NopalError::QueryExecutionError(
                            "path_knn_references: min_score (arg 4) must be a numeric literal (0.0..1.0)".into()
                        )),
                    };
                    let path_vec = self.build_path_embedding_vector(binding, &node_model, &edge_model)?;
                    let norm_sq: f32 = path_vec.iter().map(|x| x * x).sum();
                    if norm_sq == 0.0 {
                        return Err(NopalError::QueryExecutionError(
                            "path_knn_references: path vector has zero norm — cannot compute similarity (PathKNN E-9)".into()
                        ));
                    }
                    let references = self.graph.get_all_path_references_for_models_sync(&node_model, &edge_model)?;
                    let mut scored: Vec<(String, f32)> = references
                        .iter()
                        .filter_map(|r| {
                            if r.vector.len() != path_vec.len() {
                                return None; // dimensión distinta → ignorar silenciosamente
                            }
                            let score = cosine_similarity_f32(&path_vec, &r.vector).ok()?;
                            if score >= min_score { Some((r.name.clone(), score)) } else { None }
                        })
                        .collect();
                    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                    scored.truncate(k);
                    Ok(PropertyValue::List(
                        scored.into_iter()
                            .map(|(name, score)| PropertyValue::Object(vec![
                                ("name".to_string(), PropertyValue::String(name)),
                                ("score".to_string(), PropertyValue::Float(score as f64)),
                            ]))
                            .collect(),
                    ))
                }
                "path_anomaly_score" => {
                    // E-10: path_anomaly_score(node_model, edge_model)
                    // Score de anomalía = 1.0 - cosine_similarity(path_vec, centroid)
                    // El centroide es la media aritmética de todos los vectores de referencia
                    // para el par (node_model, edge_model). Score 0.0 = idéntico al centroide;
                    // score 1.0 = completamente opuesto (máxima anomalía).
                    // Sin referencias → retorna 1.0 (máxima anomalía por ausencia de baseline).
                    if args.len() != 2 {
                        return Err(NopalError::QueryExecutionError(format!(
                            "path_anomaly_score requires exactly 2 arguments: \
                             path_anomaly_score(node_model, edge_model), got {}",
                            args.len()
                        )));
                    }
                    let node_model = match &args[0] {
                        Expression::Literal(PropertyValue::String(s)) => s.clone(),
                        _ => return Err(NopalError::QueryExecutionError(
                            "path_anomaly_score: node_model (arg 1) must be a string literal".into()
                        )),
                    };
                    let edge_model = match &args[1] {
                        Expression::Literal(PropertyValue::String(s)) => s.clone(),
                        _ => return Err(NopalError::QueryExecutionError(
                            "path_anomaly_score: edge_model (arg 2) must be a string literal".into()
                        )),
                    };
                    let path_vec = self.build_path_embedding_vector(binding, &node_model, &edge_model)?;
                    let norm_sq: f32 = path_vec.iter().map(|x| x * x).sum();
                    if norm_sq == 0.0 {
                        return Err(NopalError::QueryExecutionError(
                            "path_anomaly_score: path vector has zero norm — cannot compute anomaly score (PathAnomaly E-10)".into()
                        ));
                    }
                    let references = self.graph.get_all_path_references_for_models_sync(&node_model, &edge_model)?;
                    if references.is_empty() {
                        // Sin baseline → máxima anomalía por definición
                        return Ok(PropertyValue::Float(1.0));
                    }
                    // Calcular centroide: media aritmética de los vectores de referencia
                    // Solo se incluyen referencias con la misma dimensión que el path vector.
                    let dim = path_vec.len();
                    let compatible: Vec<&Vec<f32>> = references
                        .iter()
                        .filter(|r| r.vector.len() == dim)
                        .map(|r| &r.vector)
                        .collect();
                    if compatible.is_empty() {
                        // Todas las referencias tienen dimensión distinta → máxima anomalía
                        return Ok(PropertyValue::Float(1.0));
                    }
                    let n = compatible.len() as f32;
                    let mut centroid = vec![0.0f32; dim];
                    for ref_vec in &compatible {
                        for (i, v) in ref_vec.iter().enumerate() {
                            centroid[i] += v;
                        }
                    }
                    for v in centroid.iter_mut() {
                        *v /= n;
                    }
                    // centroide de norma cero → todos los vectores de referencia se cancelan
                    let centroid_norm: f32 = centroid.iter().map(|x| x * x).sum::<f32>().sqrt();
                    if centroid_norm == 0.0 {
                        return Ok(PropertyValue::Float(1.0));
                    }
                    // cosine similarity entre path y centroide (centroide no es cero)
                    let dot: f32 = path_vec.iter().zip(centroid.iter()).map(|(a, b)| a * b).sum();
                    let path_norm: f32 = path_vec.iter().map(|x| x * x).sum::<f32>().sqrt();
                    let similarity = dot / (path_norm * centroid_norm);
                    // anomaly score = 1 - similarity; clamped a [0, 1]
                    let anomaly = (1.0 - similarity).clamp(0.0, 1.0);
                    Ok(PropertyValue::Float(anomaly as f64))
                }
                "pattern_embedding_similarity" => {
                    Err(NopalError::QueryExecutionError(
                        "pattern_embedding_similarity(...) is no longer the official PatternEmbedding surface in E-3; use pattern_embedding(node_model, edge_model)".into(),
                    ))
                }
                _ => Err(NopalError::QueryExecutionError(format!(
                    "Unsupported path/pattern embedding function '{}'",
                    name
                ))),
            }
        }

        #[cfg(not(feature = "embeddings"))]
        {
            let _ = (binding, name, args);
            Err(NopalError::QueryExecutionError(
                "Path/pattern embedding functions require feature `embeddings`".into(),
            ))
        }
    }

    /// Evalúa el predicado `has_embedding(n, "model")` en una cláusula WHERE.
    ///
    /// Retorna `true` si el nodo tiene un embedding persistido para ese modelo.
    /// Usa una lectura sync no-bloqueante del árbol Sled de embeddings.
    #[cfg(feature = "embeddings")]
    fn evaluate_embedding_predicate(
        &self,
        node: &crate::types::Node,
        args: &[Expression],
    ) -> Result<bool> {
        if args.len() != 2 {
            return Ok(false);
        }
        let model = match &args[1] {
            Expression::Literal(crate::types::PropertyValue::String(s)) => s.as_str(),
            _ => return Ok(false),
        };
        self.graph.try_node_embedding_exists_sync(node.id, model)
    }

    /// Evaluate expression to get a value (with special field support)
    fn evaluate_expression(
        &self,
        node: &crate::types::Node,
        expr: &Expression,
        expected_variable: &str,
    ) -> Option<PropertyValue> {
        match expr {
            Expression::Literal(val) => Some(val.clone()),
            Expression::Property { variable, property } => {
                if variable != expected_variable {
                    return None;
                }

                if property.is_empty() {
                    return Some(PropertyValue::String(node.id.to_string()));
                }

                // Handle special fields
                if property == "label" {
                    return Some(PropertyValue::String(node.label.clone()));
                }
                if property == "id" {
                    return Some(PropertyValue::String(node.id.to_string()));
                }
                node.properties.get(property).cloned()
            }
            _ => None,
        }
    }

    /// Compare two values
    fn compare_values(
        &self,
        left: &PropertyValue,
        op: &BinaryOperator,
        right: &PropertyValue,
    ) -> bool {
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

    pub fn take_path_profile_value(&self) -> Option<PropertyValue> {
        let mut guard = self.path_profile.lock().expect("path_profile poisoned");
        guard.take().map(|metrics| {
            PropertyValue::Object(vec![
                (
                    "bindings_examined".to_string(),
                    PropertyValue::Int(metrics.bindings_examined as i64),
                ),
                (
                    "bindings_emitted".to_string(),
                    PropertyValue::Int(metrics.bindings_emitted as i64),
                ),
                (
                    "frontier_states_visited".to_string(),
                    PropertyValue::Int(metrics.frontier_states_visited as i64),
                ),
                (
                    "cycle_prunes".to_string(),
                    PropertyValue::Int(metrics.cycle_prunes as i64),
                ),
                (
                    "max_depth_observed".to_string(),
                    PropertyValue::Int(metrics.max_depth_observed as i64),
                ),
            ])
        })
    }

    fn initialize_path_profile(&self) {
        let mut guard = self.path_profile.lock().expect("path_profile poisoned");
        *guard = Some(PathProfileCounters::default());
    }

    fn reset_path_profile(&self) {
        let mut guard = self.path_profile.lock().expect("path_profile poisoned");
        *guard = None;
    }

    fn update_path_profile<F>(&self, update: F)
    where
        F: FnOnce(&mut PathProfileCounters),
    {
        let mut guard = self.path_profile.lock().expect("path_profile poisoned");
        if let Some(metrics) = guard.as_mut() {
            update(metrics);
        }
    }

    fn path_metadata_from_binding(&self, binding: &LinearPatternBinding) -> PathMetadata {
        let nodes = binding
            .nodes
            .iter()
            .map(|node| {
                PropertyValue::Object(vec![
                    ("id".to_string(), PropertyValue::String(node.id.to_string())),
                    (
                        "label".to_string(),
                        PropertyValue::String(node.label.clone()),
                    ),
                ])
            })
            .collect();

        let edges = binding
            .edges
            .iter()
            .map(|edge| {
                PropertyValue::Object(vec![
                    ("id".to_string(), PropertyValue::String(edge.id.to_string())),
                    (
                        "type".to_string(),
                        PropertyValue::String(edge.edge_type.clone()),
                    ),
                    (
                        "source".to_string(),
                        PropertyValue::String(edge.source.to_string()),
                    ),
                    (
                        "target".to_string(),
                        PropertyValue::String(edge.target.to_string()),
                    ),
                ])
            })
            .collect();

        PathMetadata {
            depth: binding.edges.len(),
            nodes: PropertyValue::List(nodes),
            edges: PropertyValue::List(edges),
        }
    }

    fn query_uses_path_metadata(&self, query: &Query) -> bool {
        query
            .find
            .projections
            .iter()
            .any(|projection| match projection {
                Projection::Expression { expr, .. } => self.expression_references_path(expr),
                _ => false,
            })
            || query
                .filter
                .as_ref()
                .is_some_and(|filter| self.expression_references_path(&filter.condition))
            || query.order_by.as_ref().is_some_and(|order_by| {
                order_by
                    .items
                    .iter()
                    .any(|item| self.expression_references_path(&item.expression))
            })
            || query.group_by.as_ref().is_some_and(|group_by| {
                group_by
                    .expressions
                    .iter()
                    .any(|expr| self.expression_references_path(expr))
            })
            || query
                .having
                .as_ref()
                .is_some_and(|having| self.expression_references_path(&having.condition))
    }

    fn query_uses_quoted_path_vm(&self, query: &Query) -> bool {
        !query.init.is_empty()
            || !query.gather.is_empty()
            || query.return_expr.is_some()  // F4-C
            || projections_contain_path_eval(&query.find.projections)
            || query
                .filter
                .as_ref()
                .is_some_and(|filter| expr_contains_path_eval(&filter.condition))
            || query.order_by.as_ref().is_some_and(|order_by| {
                order_by
                    .items
                    .iter()
                    .any(|item| expr_contains_path_eval(&item.expression))
            })
            || query.group_by.as_ref().is_some_and(|group_by| {
                group_by
                    .expressions
                    .iter()
                    .any(expr_contains_path_eval)
            })
            || query
                .having
                .as_ref()
                .is_some_and(|having| expr_contains_path_eval(&having.condition))
    }

    fn validate_path_metadata_usage(&self, query: &Query) -> Result<()> {
        let find_path_props: Vec<PathPropertyKind> = query
            .find
            .projections
            .iter()
            .filter_map(|projection| match projection {
                Projection::Expression { expr, .. } => {
                    self.collect_path_property_kinds(expr).into_iter().next()
                }
                _ => None,
            })
            .collect();

        let filter_path_props = query
            .filter
            .as_ref()
            .map(|filter| self.collect_path_property_kinds(&filter.condition))
            .unwrap_or_default();

        let order_path_props: Vec<PathPropertyKind> = query
            .order_by
            .as_ref()
            .map(|order_by| {
                order_by
                    .items
                    .iter()
                    .flat_map(|item| self.collect_path_property_kinds(&item.expression))
                    .collect()
            })
            .unwrap_or_default();

        let group_path_props: Vec<PathPropertyKind> = query
            .group_by
            .as_ref()
            .map(|group_by| {
                group_by
                    .expressions
                    .iter()
                    .flat_map(|expr| self.collect_path_property_kinds(expr))
                    .collect()
            })
            .unwrap_or_default();

        let having_path_props = query
            .having
            .as_ref()
            .map(|having| self.collect_path_property_kinds(&having.condition))
            .unwrap_or_default();

        let any_path_usage = !find_path_props.is_empty()
            || !filter_path_props.is_empty()
            || !order_path_props.is_empty()
            || !group_path_props.is_empty()
            || !having_path_props.is_empty();

        if !any_path_usage {
            return Ok(());
        }

        if let Some(invalid_prop) = self.find_invalid_path_property(query) {
            return Err(NopalError::QueryExecutionError(format!(
                "Unknown path metadata property 'path.{}' in Path Queries F2",
                invalid_prop
            )));
        }

        if query.from.patterns.len() != 1
            || !self.pattern_has_relationships(&query.from.patterns[0])
        {
            return Err(NopalError::QueryExecutionError(
                "path.* is only supported for a single linear pattern with at least one relationship in Path Queries F2".into()
            ));
        }

        if filter_path_props
            .iter()
            .any(|kind| matches!(kind, PathPropertyKind::Nodes | PathPropertyKind::Edges))
        {
            return Err(NopalError::QueryExecutionError(
                "path.nodes and path.edges are only supported in FIND projections in Path Queries F2".into()
            ));
        }

        if order_path_props
            .iter()
            .any(|kind| matches!(kind, PathPropertyKind::Nodes | PathPropertyKind::Edges))
        {
            return Err(NopalError::QueryExecutionError(
                "path.nodes and path.edges are not supported in ORDER BY in Path Queries F2".into(),
            ));
        }

        if !group_path_props.is_empty() || !having_path_props.is_empty() {
            return Err(NopalError::QueryExecutionError(
                "path.* is not supported in GROUP BY or HAVING in Path Queries F2".into(),
            ));
        }

        Ok(())
    }

    fn pattern_has_relationships(&self, pattern: &Pattern) -> bool {
        pattern
            .elements
            .iter()
            .any(|element| matches!(element, PatternElement::Relationship(_)))
    }

    fn expression_references_path(&self, expr: &Expression) -> bool {
        !self.collect_path_property_kinds(expr).is_empty()
    }

    fn find_invalid_path_property(&self, query: &Query) -> Option<String> {
        for projection in &query.find.projections {
            if let Projection::Expression { expr, .. } = projection
                && let Some(prop) = self.find_invalid_path_property_in_expr(expr)
            {
                return Some(prop);
            }
        }

        if let Some(filter) = &query.filter
            && let Some(prop) = self.find_invalid_path_property_in_expr(&filter.condition)
        {
            return Some(prop);
        }

        if let Some(order_by) = &query.order_by {
            for item in &order_by.items {
                if let Some(prop) = self.find_invalid_path_property_in_expr(&item.expression) {
                    return Some(prop);
                }
            }
        }

        if let Some(group_by) = &query.group_by {
            for expr in &group_by.expressions {
                if let Some(prop) = self.find_invalid_path_property_in_expr(expr) {
                    return Some(prop);
                }
            }
        }

        if let Some(having) = &query.having
            && let Some(prop) = self.find_invalid_path_property_in_expr(&having.condition)
        {
            return Some(prop);
        }

        None
    }

    fn find_invalid_path_property_in_expr(&self, expr: &Expression) -> Option<String> {
        match expr {
            Expression::Property { variable, property } if variable == "path" => {
                match property.as_str() {
                    "depth" | "nodes" | "edges" | "start" | "end" | "state" | "result" => None,
                    _ => Some(property.clone()),
                }
            }
            Expression::BinaryOp { left, right, .. } => self
                .find_invalid_path_property_in_expr(left)
                .or_else(|| self.find_invalid_path_property_in_expr(right)),
            Expression::UnaryOp { expr, .. } => self.find_invalid_path_property_in_expr(expr),
            Expression::FunctionCall { args, .. } => args
                .iter()
                .find_map(|arg| self.find_invalid_path_property_in_expr(arg)),
            Expression::Literal(_) | Expression::Wildcard | Expression::Property { .. } => None,
        }
    }

    fn collect_path_property_kinds(&self, expr: &Expression) -> Vec<PathPropertyKind> {
        let mut kinds = Vec::new();
        self.collect_path_property_kinds_inner(expr, &mut kinds);
        kinds
    }

    fn collect_path_property_kinds_inner(
        &self,
        expr: &Expression,
        kinds: &mut Vec<PathPropertyKind>,
    ) {
        match expr {
            Expression::Property { variable, property } if variable == "path" => {
                match property.as_str() {
                    "depth" => kinds.push(PathPropertyKind::Depth),
                    "nodes" => kinds.push(PathPropertyKind::Nodes),
                    "edges" => kinds.push(PathPropertyKind::Edges),
                    "start" => kinds.push(PathPropertyKind::Start),
                    "end" => kinds.push(PathPropertyKind::End),
                    "state" => kinds.push(PathPropertyKind::State),
                    "result" => kinds.push(PathPropertyKind::Result),
                    _ => {}
                }
            }
            Expression::BinaryOp { left, right, .. } => {
                self.collect_path_property_kinds_inner(left, kinds);
                self.collect_path_property_kinds_inner(right, kinds);
            }
            Expression::UnaryOp { expr, .. } => self.collect_path_property_kinds_inner(expr, kinds),
            Expression::FunctionCall { args, .. } => {
                for arg in args {
                    self.collect_path_property_kinds_inner(arg, kinds);
                }
            }
            Expression::Literal(_) | Expression::Wildcard | Expression::Property { .. } => {}
        }
    }

    /// Apply ORDER BY to query result (I1)
    fn apply_order_by(&self, result: &mut QueryResult, order_by: &OrderByClause) {
        result.rows.sort_by(|a, b| {
            for item in &order_by.items {
                // Extract column name from expression
                let col_name = match &item.expression {
                    Expression::Property { variable, property } => {
                        if property.is_empty() {
                            variable.clone()
                        } else {
                            property_projection_key(variable, property)
                        }
                    }
                    _ => continue,
                };

                let a_val = a.get(&col_name);
                let b_val = b.get(&col_name);

                let ordering = match (a_val, b_val) {
                    (Some(av), Some(bv)) => av.cmp(bv),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                };

                let ordering = match item.order {
                    SortOrder::Desc => ordering.reverse(),
                    SortOrder::Asc => ordering,
                };

                if ordering != std::cmp::Ordering::Equal {
                    return ordering;
                }
            }
            std::cmp::Ordering::Equal
        });
    }

    fn apply_distinct_if_needed(
        &self,
        result: &mut QueryResult,
        find: &crate::query::nql::parser::ast::FindClause,
    ) {
        if !find.distinct {
            return;
        }

        let distinct_columns = result.columns.clone();
        let mut seen: HashSet<Vec<PropertyValue>> = HashSet::new();

        result.rows.retain(|row| {
            let key: Vec<PropertyValue> = distinct_columns
                .iter()
                .map(|column| row.get(column).cloned().unwrap_or(PropertyValue::Null))
                .collect();
            seen.insert(key)
        });
    }

    // ═══════════════════════════════════════════════════════════
    // P2: ORDER BY on non-projected columns support
    // ═══════════════════════════════════════════════════════════

    /// Extract ORDER BY column names that are NOT in the FIND projection.
    /// These need to be temporarily injected for sorting, then stripped.
    fn extract_order_by_extras(&self, query: &Query) -> Vec<String> {
        let order_by = match &query.order_by {
            Some(ob) => ob,
            None => return vec![],
        };

        // Collect projected column names
        let projected: std::collections::HashSet<String> = query
            .find
            .projections
            .iter()
            .filter_map(|p| match p {
                Projection::Expression { expr, alias } => {
                    if let Some(a) = alias {
                        Some(a.clone())
                    } else if let Expression::Property { variable, property } = expr {
                        Some(property_projection_key(variable, property))
                    } else {
                        None
                    }
                }
                Projection::Wildcard => None, // wildcard includes everything
                Projection::All(_) => None,
            })
            .collect();

        // If wildcard, all columns are available — no extras needed
        let has_wildcard = query
            .find
            .projections
            .iter()
            .any(|p| matches!(p, Projection::Wildcard));
        if has_wildcard {
            return vec![];
        }

        // Find ORDER BY columns not in projection
        order_by
            .items
            .iter()
            .filter_map(|item| {
                if let Expression::Property { variable, property } = &item.expression {
                    let col = property_projection_key(variable, property);
                    if !projected.contains(&col) {
                        Some(col)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    /// Project result with extra ORDER BY columns temporarily included
    async fn project_result_with_extras(
        &self,
        nodes: Vec<crate::types::Node>,
        query: &Query,
        extras: &[String],
    ) -> Result<QueryResult> {
        let mut result = self.project_result(nodes.clone(), query).await?;

        if extras.is_empty() {
            return Ok(result);
        }

        // Extract variable from pattern
        let variable = query
            .from
            .patterns
            .first()
            .and_then(|p| p.elements.first())
            .and_then(|e| match e {
                PatternElement::Node(n) => n.variable.as_deref().or(Some("n")),
                _ => None,
            })
            .unwrap_or("n");

        // Add extra columns to each row from the original nodes
        for (row, node) in result.rows.iter_mut().zip(nodes.iter()) {
            for extra_col in extras {
                // Parse "variable.property" to get property name
                let property = extra_col
                    .strip_prefix(&format!("{}.", variable))
                    .unwrap_or(extra_col);

                let value = if property == "label" {
                    Some(PropertyValue::String(node.label.clone()))
                } else if property == "id" {
                    Some(PropertyValue::String(node.id.to_string()))
                } else {
                    node.properties.get(property).cloned()
                };

                if let Some(v) = value {
                    row.set(extra_col.clone(), v);
                }
            }
        }

        // Add extra columns to the column list
        for extra in extras {
            result.columns.push(extra.clone());
        }

        Ok(result)
    }

    /// Strip temporary ORDER BY columns from the result
    fn strip_extra_columns(&self, result: &mut QueryResult, extras: &[String]) {
        // Remove from column list
        result.columns.retain(|c| !extras.contains(c));

        // Remove from each row
        for row in &mut result.rows {
            for extra in extras {
                row.values.remove(extra);
            }
        }
    }

    /// Evaluate expression on a pattern match with proper variable scoping (I3)
    fn evaluate_pattern_expression(
        &self,
        m: &operators::PatternMatch,
        expr: &Expression,
        source_var: &str,
        target_var: &str,
    ) -> Option<PropertyValue> {
        match expr {
            Expression::Literal(val) => Some(val.clone()),
            Expression::Property { variable, property } => {
                // Variable-only expression (e.g., count(r)) resolves to ID.
                if property.is_empty() {
                    if variable == source_var {
                        return Some(PropertyValue::String(m.source.id.to_string()));
                    }
                    if variable == target_var {
                        return Some(PropertyValue::String(m.target.id.to_string()));
                    }
                    if let Some(edge) = &m.edge {
                        return Some(PropertyValue::String(edge.id.to_string()));
                    }
                    return None;
                }

                // Variable-aware property lookup
                let node = if variable == source_var {
                    &m.source
                } else if variable == target_var {
                    &m.target
                } else {
                    // P2 fix: Only treat as edge if edge actually exists.
                    // Don't silently assume unknown variables are edges.
                    if let Some(edge) = &m.edge {
                        if property == "edge_type" || property == "type" {
                            return Some(PropertyValue::String(edge.edge_type.clone()));
                        }
                        return edge.properties.get(property).cloned();
                    }
                    log::warn!(
                        "Unknown variable '{}' in pattern expression (known: {}, {})",
                        variable,
                        source_var,
                        target_var
                    );
                    return None;
                };

                // Handle special fields
                if property == "label" {
                    return Some(PropertyValue::String(node.label.clone()));
                }
                if property == "id" {
                    return Some(PropertyValue::String(node.id.to_string()));
                }

                node.properties.get(property).cloned()
            }
            _ => None,
        }
    }

    /// Project result based on FIND clause
    async fn project_result(
        &self,
        nodes: Vec<crate::types::Node>,
        query: &Query,
    ) -> Result<QueryResult> {
        if nodes.is_empty() {
            return Ok(QueryResult::empty());
        }

        // Extract variable from first pattern
        let pattern = &query.from.patterns[0];
        let first_element = &pattern.elements[0];

        let variable = match first_element {
            PatternElement::Node(node_pattern) => node_pattern.variable.as_deref().unwrap_or("n"),
            PatternElement::Relationship(_) => {
                return Err(NopalError::QueryExecutionError(
                    "Pattern cannot start with relationship".into(),
                ));
            }
        };

        //CHECK FOR AGGREGATIONS
        if has_aggregations(&query.find.projections) || query.group_by.is_some() {
            return execute_aggregations(self.graph, nodes, query, variable).await;
        }

        // Check if wildcard
        let is_wildcard = query.find.projections.len() == 1
            && matches!(&query.find.projections[0], Projection::Wildcard);

        if is_wildcard {
            // Wildcard: return all properties
            self.project_wildcard(nodes, variable)
        } else {
            // Specific projections
            self.project_specific(nodes, variable, query)
        }
    }

    /// Project wildcard (all properties)
    fn project_wildcard(
        &self,
        nodes: Vec<crate::types::Node>,
        variable: &str,
    ) -> Result<QueryResult> {
        let mut result = QueryResult::new(vec![
            format!("{}.id", variable),
            format!("{}.label", variable),
        ]);

        for node in nodes {
            let mut row = Row::new();
            row.set(
                format!("{}.id", variable),
                PropertyValue::String(node.id.to_string()),
            );
            row.set(
                format!("{}.label", variable),
                PropertyValue::String(node.label.clone()),
            );

            // Add all properties
            for (key, value) in &node.properties {
                row.set(format!("{}.{}", variable, key), value.clone());
            }

            result.add_row(row);
        }

        Ok(result)
    }

    /// Project specific fields
    fn project_specific(
        &self,
        nodes: Vec<crate::types::Node>,
        variable: &str,
        query: &Query,
    ) -> Result<QueryResult> {
        // Extract column names
        let columns: Vec<String> = query
            .find
            .projections
            .iter()
            .enumerate()
            .map(|(idx, p)| match p {
                Projection::Expression { expr, alias } => {
                    if let Some(a) = alias {
                        a.clone()
                    } else if let Expression::Property {
                        variable: v,
                        property: prop,
                    } = expr
                    {
                        property_projection_key(v, prop)
                    } else {
                        format!("expr_{}", idx)
                    }
                }
                Projection::Wildcard => "*".to_string(),
                Projection::All(var) => format!("all({})", var),
            })
            .collect();

        let mut result = QueryResult::new(columns.clone());

        for node in nodes {
            let mut row = Row::new();

            for (i, projection) in query.find.projections.iter().enumerate() {
                let col_name = &columns[i];

                match projection {
                    Projection::Expression { expr, .. } => {
                        match expr {
                            Expression::Property {
                                variable: proj_var,
                                property: prop,
                            } => {
                                if proj_var != variable {
                                    continue;
                                }

                                if prop == "label" {
                                    row.set(
                                        col_name.clone(),
                                        PropertyValue::String(node.label.clone()),
                                    );
                                } else if prop == "id" || prop.is_empty() {
                                    row.set(
                                        col_name.clone(),
                                        PropertyValue::String(node.id.to_string()),
                                    );
                                } else if let Some(value) = node.properties.get(prop) {
                                    row.set(col_name.clone(), value.clone());
                                }
                            }
                            _ => {
                                // Handle other expressions
                            }
                        }
                    }
                    Projection::Wildcard => {
                        // Add all properties
                        for (key, value) in &node.properties {
                            row.set(key.clone(), value.clone());
                        }
                    }
                    Projection::All(var) => {
                        // Add all properties of specific variable
                        if variable == var {
                            for (key, value) in &node.properties {
                                row.set(format!("{}.{}", var, key), value.clone());
                            }
                        }
                    }
                }
            }

            result.add_row(row);
        }

        Ok(result)
    }

    // ═══════════════════════════════════════════════════════════
    // WRITE OPERATIONS (NQL v0.2)
    // ═══════════════════════════════════════════════════════════

    /// Execute ADD statement
    pub async fn execute_add(&self, add: &AddStmt, tx: &mut Transaction) -> Result<AddResult> {
        let write_executor = WriteExecutor::new(self.graph);
        write_executor.execute_add(add, tx).await
    }

    /// Execute DELETE statement
    pub async fn execute_delete(
        &self,
        delete: &DeleteStmt,
        tx: &mut Transaction,
    ) -> Result<DeleteResult> {
        let write_executor = WriteExecutor::new(self.graph);
        write_executor.execute_delete(delete, tx).await
    }

    /// Execute UPDATE statement
    pub async fn execute_update(
        &self,
        update: &UpdateStmt,
        tx: &mut Transaction,
    ) -> Result<UpdateResult> {
        let write_executor = WriteExecutor::new(self.graph);
        write_executor.execute_update(update, tx).await
    }

    /// Execute CREATE INDEX
    pub async fn execute_create_index(&self, stmt: CreateIndexStmt) -> Result<String> {
        // Convertir IndexType del AST a IndexType del graph
        let index_type = match stmt.index_type {
            IndexType::Hash => GraphIndexType::Hash,
            IndexType::BTree => GraphIndexType::BTree,
            IndexType::FullText => GraphIndexType::FullText,
            IndexType::Taxonomy => GraphIndexType::Taxonomy,
        };

        self.graph
            .create_index(&stmt.label, &stmt.property, index_type)
            .await
    }

    /// Execute DROP INDEX
    pub async fn execute_drop_index(&self, stmt: DropIndexStmt) -> Result<()> {
        self.graph.drop_index(&stmt.index_name).await
    }

    /// Execute EXPLAIN
    pub async fn execute_explain(&self, stmt: Statement) -> Result<String> {
        // Create planner
        let planner = self.graph.create_planner().await?;

        match stmt {
            Statement::Query(query) => {
                // Analyze query
                let plan = self.build_plan_for_query(&query, &planner).await?;
                let mut explanation = planner.explain(&plan);
                if self.query_uses_function(&query, &["community", "community_fast"]) {
                    explanation.push_str(
                        "\nCost note: community() requires global community partition computation (LIMIT applies after aggregation).",
                    );
                    explanation.push_str(
                        "\nCost note: community_fast() uses approximate local partitioning for lower latency.",
                    );
                }
                if self.query_uses_function(&query, &["leiden"]) {
                    explanation.push_str(
                        "\nCost note: leiden() runs Leiden CPM community detection (Traag et al. 2019). \
                         Guarantees well-connected communities unlike Louvain. \
                         Uses a separate cache from community() — both can coexist in the same query. \
                         Default gamma=0.1; use LeidenCommunity::with_gamma() for custom resolution.",
                    );
                }
                Ok(explanation)
            }
            _ => Ok(format!("EXPLAIN for {:?} (not yet implemented)", stmt)),
        }
    }

    /// Build execution plan for query
    async fn build_plan_for_query(
        &self,
        query: &Query,
        planner: &QueryPlanner,
    ) -> Result<PlanNode> {
        // Extract label from FROM clause
        if query.from.patterns.is_empty() {
            return Err(NopalError::custom("No patterns in FROM"));
        }

        let pattern = &query.from.patterns[0];
        if pattern.elements.is_empty() {
            return Err(NopalError::custom("Empty pattern"));
        }

        // Get label
        let label = match &pattern.elements[0] {
            PatternElement::Node(node) => node
                .label
                .as_ref()
                .ok_or_else(|| NopalError::custom("Node has no label"))?,
            _ => return Err(NopalError::custom("Pattern must start with node")),
        };

        // Check if WHERE clause references an indexed property
        let (property, has_index) = if let Some(filter) = &query.filter {
            self.find_indexed_property(label, filter).await?
        } else {
            (None, false)
        };

        // Let planner choose
        Ok(planner.choose_best_plan(label, property.as_deref(), has_index))
    }

    /// Find indexed property in WHERE clause
    async fn find_indexed_property(
        &self,
        label: &str,
        where_clause: &WhereClause,
    ) -> Result<(Option<String>, bool)> {
        let indexes = self.graph.list_indexes().await;

        // Extract property from WHERE condition
        let prop_name = self.extract_property_from_condition(&where_clause.condition)?;

        if let Some(prop) = prop_name {
            let index_name = format!("{}_{}", label, prop);
            let has_index = indexes.iter().any(|meta| meta.name == index_name);

            return Ok((Some(prop), has_index));
        }

        Ok((None, false))
    }

    /// Extract property name from condition
    fn extract_property_from_condition(&self, expr: &Expression) -> Result<Option<String>> {
        if let Expression::BinaryOp {
            left,
            op: _,
            right: _,
        } = expr
        {
            // Check if left is a property (field access)
            if let Expression::Property {
                variable: _,
                property,
            } = &**left
            {
                return Ok(Some(property.clone()));
            }
        }
        Ok(None)
    }

    fn query_uses_function(&self, query: &Query, names: &[&str]) -> bool {
        query
            .find
            .projections
            .iter()
            .any(|projection| match projection {
                Projection::Expression { expr, .. } => self.expr_contains_function(expr, names),
                _ => false,
            })
    }

    fn expr_contains_function(&self, expr: &Expression, names: &[&str]) -> bool {
        match expr {
            Expression::FunctionCall { name, args } => {
                if names
                    .iter()
                    .any(|candidate| name.eq_ignore_ascii_case(candidate))
                {
                    return true;
                }
                args.iter()
                    .any(|arg| self.expr_contains_function(arg, names))
            }
            Expression::BinaryOp { left, right, .. } => {
                self.expr_contains_function(left, names)
                    || self.expr_contains_function(right, names)
            }
            Expression::UnaryOp { expr, .. } => self.expr_contains_function(expr, names),
            _ => false,
        }
    }

    // ═══════════════════════════════════════════════════════════
    // HELPER METHODS FOR SKETCH MANAGER
    // ═══════════════════════════════════════════════════════════

    /// Match pattern against graph (for preview)
    pub async fn match_pattern(
        &self,
        pattern: &crate::query::nql::parser::ast::Pattern,
    ) -> Result<Vec<MatchedElement>> {
        let write_executor = WriteExecutor::new(self.graph);
        write_executor.match_pattern(pattern).await
    }

    /// Filter matched elements
    pub fn filter_matches(
        &self,
        matches: Vec<MatchedElement>,
        condition: &Expression,
    ) -> Result<Vec<MatchedElement>> {
        let write_executor = WriteExecutor::new(self.graph);
        write_executor.filter_matches(matches, condition)
    }

    /// Count nodes and edges
    pub fn count_elements(&self, elements: &[MatchedElement]) -> (usize, usize) {
        let write_executor = WriteExecutor::new(self.graph);
        write_executor.count_elements(elements)
    }

    /// Sample node IDs
    pub fn sample_node_ids(&self, elements: &[MatchedElement], limit: usize) -> Vec<String> {
        let write_executor = WriteExecutor::new(self.graph);
        write_executor.sample_node_ids(elements, limit)
    }

    /// Sample updates
    pub fn sample_updates(
        &self,
        elements: &[MatchedElement],
        assignments: &[crate::query::nql::parser::ast::Assignment],
        limit: usize,
    ) -> Vec<crate::query::sketch_manager::UpdateSample> {
        let write_executor = WriteExecutor::new(self.graph);
        write_executor.sample_updates(elements, assignments, limit)
    }

    /// Pre-compute similar_to() HNSW search if present in WHERE condition.
    ///
    /// Detects `similar_to(n, "reference_name", "model")` in the expression tree.
    /// Resolves the reference node by name, gets its embedding, builds the HNSW index,
    /// and returns a HashSet of the k nearest NodeIds.
    ///
    /// The k is derived from the query's LIMIT clause (default: 10).
    ///
    /// Returns None if no similar_to function is found in the condition.
    #[cfg(feature = "embeddings-index")]
    async fn precompute_similar_to(
        &self,
        condition: &Expression,
        query: &Query,
    ) -> Result<Option<HashSet<crate::types::NodeId>>> {
        // Extraer similar_to(variable, "ref_name", "model") del árbol de expresiones
        let params = extract_similar_to_params(condition);
        let (_variable, ref_name, model) = match params {
            Some(p) => p,
            None => return Ok(None),
        };

        // k para la búsqueda HNSW — del LIMIT de la query, o default 10
        let k = query.limit.as_ref().map(|l| l.limit).unwrap_or(10);

        // Resolver nodo de referencia por nombre
        let ref_node = self
            .graph
            .get_node_by_property("name", &ref_name)
            .await
            .map_err(|_| {
                NopalError::QueryExecutionError(format!(
                    "similar_to: reference node '{}' not found",
                    ref_name
                ))
            })?;

        // Obtener embedding del nodo de referencia
        let ref_embedding = self
            .graph
            .get_node_embedding(ref_node.id, &model)
            .await
            .map_err(|_| {
                NopalError::QueryExecutionError(format!(
                    "similar_to: reference node '{}' has no embedding for model '{}'",
                    ref_name, model
                ))
            })?;

        // Obtener índice HNSW desde caché (o construirlo si no existe)
        let index = self.graph.get_or_build_embedding_index(&model).await?;
        let results = index.search_knn(&ref_embedding.vector, k)?;

        let node_ids: HashSet<crate::types::NodeId> =
            results.into_iter().map(|(id, _)| id).collect();
        Ok(Some(node_ids))
    }
}

/// Extrae los parámetros de similar_to(variable, "ref_name", "model") de un árbol de expresiones.
/// Busca recursivamente en AND/OR conditions.
/// Retorna Some((variable, ref_name, model)) si encuentra la función.
#[cfg(feature = "embeddings-index")]
fn extract_similar_to_params(expr: &Expression) -> Option<(String, String, String)> {
    match expr {
        Expression::FunctionCall { name, args } if name.to_lowercase() == "similar_to" => {
            if args.len() < 2 || args.len() > 3 {
                return None;
            }
            // arg 0: variable (property access o identifier)
            let _variable = match &args[0] {
                Expression::Property { variable, .. } => variable.clone(),
                _ => "n".to_string(),
            };
            // arg 1: reference name (string literal)
            let ref_name = match &args[1] {
                Expression::Literal(PropertyValue::String(s)) => s.clone(),
                _ => return None,
            };
            // arg 2: model (optional, default "default")
            let model = if args.len() == 3 {
                match &args[2] {
                    Expression::Literal(PropertyValue::String(s)) => s.clone(),
                    _ => return None,
                }
            } else {
                "default".to_string()
            };
            Some((_variable, ref_name, model))
        }
        Expression::BinaryOp {
            left,
            op: BinaryOperator::And,
            right,
        }
        | Expression::BinaryOp {
            left,
            op: BinaryOperator::Or,
            right,
        } => extract_similar_to_params(left).or_else(|| extract_similar_to_params(right)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ExecutionMode, Executor};
    use crate::graph::Graph;
    use crate::index::TaxonomyIndex;
    use crate::query::nql::parse_query;
    use crate::types::{Edge, Node, NodeKind, PropertyValue};

    fn str_val(v: &str) -> PropertyValue {
        PropertyValue::String(v.to_string())
    }

    async fn install_financial_taxonomy(graph: &Graph) {
        let mut taxonomy = TaxonomyIndex::new();

        let financial_entity = uuid::Uuid::new_v4();
        let account = uuid::Uuid::new_v4();
        let savings_account = uuid::Uuid::new_v4();
        let document = uuid::Uuid::new_v4();

        taxonomy.register_class(financial_entity, "FinancialEntity");
        taxonomy.register_class(account, "Account");
        taxonomy.register_class(savings_account, "SavingsAccount");
        taxonomy.register_class(document, "Document");

        taxonomy.add_subclass(financial_entity, account).unwrap();
        taxonomy.add_subclass(account, savings_account).unwrap();

        graph.install_taxonomy_snapshot(taxonomy).await;
    }

    #[tokio::test]
    async fn test_determine_execution_mode_simple_single_hop_uses_fast_traverse() {
        let graph = Graph::in_memory().await.unwrap();
        let executor = Executor::new(&graph);
        let query =
            parse_query("find a.name, b.name from (a:Person)-[:KNOWS]->(b:Person)").unwrap();
        let pattern = &query.from.patterns[0];

        assert_eq!(
            executor.determine_execution_mode(&query, pattern),
            ExecutionMode::FastTraverse
        );
    }

    #[tokio::test]
    async fn test_determine_execution_mode_quantified_pattern_uses_linear_bindings() {
        let graph = Graph::in_memory().await.unwrap();
        let executor = Executor::new(&graph);
        let query =
            parse_query("find a.name, b.name from (a:Person)-[:KNOWS]->{1,2}(b:Person)").unwrap();
        let pattern = &query.from.patterns[0];

        assert_eq!(
            executor.determine_execution_mode(&query, pattern),
            ExecutionMode::LinearBindings
        );
    }

    #[tokio::test]
    async fn test_determine_execution_mode_path_metadata_uses_linear_bindings() {
        let graph = Graph::in_memory().await.unwrap();
        let executor = Executor::new(&graph);
        let query =
            parse_query("find b.name, path.depth as depth from (a:Person)-[:KNOWS]->(b:Person)")
                .unwrap();
        let pattern = &query.from.patterns[0];

        assert_eq!(
            executor.determine_execution_mode(&query, pattern),
            ExecutionMode::LinearBindings
        );
    }

    #[tokio::test]
    async fn test_determine_execution_mode_path_reducer_uses_linear_bindings() {
        let graph = Graph::in_memory().await.unwrap();
        let executor = Executor::new(&graph);
        let query = parse_query(
            "find b.name, path_sum(\"amount\") as total from (a:Account)-[:TRANSFER]->(b:Account)",
        )
        .unwrap();
        let pattern = &query.from.patterns[0];

        assert_eq!(
            executor.determine_execution_mode(&query, pattern),
            ExecutionMode::LinearBindings
        );
    }

    #[tokio::test]
    async fn test_determine_execution_mode_fixed_multihop_uses_linear_bindings() {
        let graph = Graph::in_memory().await.unwrap();
        let executor = Executor::new(&graph);
        let query =
            parse_query("find c.name from (a:Person)-[:KNOWS]->(b:Person)-[:KNOWS]->(c:Person)")
                .unwrap();
        let pattern = &query.from.patterns[0];

        assert_eq!(
            executor.determine_execution_mode(&query, pattern),
            ExecutionMode::LinearBindings
        );
    }

    #[tokio::test]
    async fn test_determine_execution_mode_semantic_path_filter_uses_linear_bindings() {
        let graph = Graph::in_memory().await.unwrap();
        let executor = Executor::new(&graph);
        let query = parse_query(
            "find b.name from (a:Account)-[:TX]->(b:Account) where path_end_instanceOf(\"FinancialEntity\")",
        )
        .unwrap();
        let pattern = &query.from.patterns[0];

        assert_eq!(
            executor.determine_execution_mode(&query, pattern),
            ExecutionMode::LinearBindings
        );
    }

    #[tokio::test]
    async fn test_path_end_instanceof_filters_paths() {
        let graph = Graph::in_memory().await.unwrap();
        install_financial_taxonomy(&graph).await;

        let mut tx = graph.begin_transaction().await.unwrap();
        let a = tx
            .add_node(Node::new("SavingsAccount").with_property("name", str_val("A")))
            .await
            .unwrap();
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await
            .unwrap();
        let c = tx
            .add_node(Node::new("Document").with_property("name", str_val("C")))
            .await
            .unwrap();
        tx.add_edge(Edge::new(a, b, "TX")).unwrap();
        tx.add_edge(Edge::new(b, c, "TX")).unwrap();
        tx.commit().await.unwrap();

        let result = graph
            .execute_nql(
                r#"
                find n.name
                from (a:SavingsAccount {name: "A"})-[:TX]->{1,2}(n)
                where path_end_instanceOf("FinancialEntity")
            "#,
            )
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(
            result.rows()[0].get("n.name"),
            Some(&PropertyValue::String("B".to_string()))
        );
    }

    #[tokio::test]
    async fn test_path_all_instanceof_rejects_mixed_semantic_paths() {
        let graph = Graph::in_memory().await.unwrap();
        install_financial_taxonomy(&graph).await;

        let mut tx = graph.begin_transaction().await.unwrap();
        let a = tx
            .add_node(Node::new("SavingsAccount").with_property("name", str_val("A")))
            .await
            .unwrap();
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await
            .unwrap();
        let c = tx
            .add_node(Node::new("Document").with_property("name", str_val("C")))
            .await
            .unwrap();
        tx.add_edge(Edge::new(a, b, "TX")).unwrap();
        tx.add_edge(Edge::new(b, c, "TX")).unwrap();
        tx.commit().await.unwrap();

        let result = graph
            .execute_nql(
                r#"
                find n.name
                from (a:SavingsAccount {name: "A"})-[:TX]->{1,2}(n)
                where path_all_instanceOf("FinancialEntity")
            "#,
            )
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(
            result.rows()[0].get("n.name"),
            Some(&PropertyValue::String("B".to_string()))
        );
    }

    #[tokio::test]
    async fn test_path_any_subclassof_matches_class_nodes() {
        let graph = Graph::in_memory().await.unwrap();
        install_financial_taxonomy(&graph).await;

        let mut tx = graph.begin_transaction().await.unwrap();
        let mut a_node = Node::new("FinancialEntity");
        a_node.kind = NodeKind::Class;
        a_node
            .properties
            .insert("name".to_string(), str_val("FinancialEntity"));
        let a = tx.add_node(a_node).await.unwrap();

        let mut b_node = Node::new("Account");
        b_node.kind = NodeKind::Class;
        b_node
            .properties
            .insert("name".to_string(), str_val("Account"));
        let b = tx.add_node(b_node).await.unwrap();

        let mut c_node = Node::new("Document");
        c_node.kind = NodeKind::Class;
        c_node
            .properties
            .insert("name".to_string(), str_val("Document"));
        let c = tx.add_node(c_node).await.unwrap();

        tx.add_edge(Edge::new(a, b, "REL")).unwrap();
        tx.add_edge(Edge::new(b, c, "REL")).unwrap();
        tx.commit().await.unwrap();

        let result = graph
            .execute_nql(
                r#"
                find n.name
                from (a:FinancialEntity {name: "FinancialEntity"})-[:REL]->{1,2}(n)
                where path_any_subClassOf("FinancialEntity")
            "#,
            )
            .await
            .unwrap();

        assert_eq!(result.len(), 2);
    }

    #[cfg(feature = "embeddings")]
    #[tokio::test]
    async fn test_path_embedding_projects_vector() {
        let graph = Graph::in_memory().await.unwrap();

        let mut tx = graph.begin_transaction().await.unwrap();
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await
            .unwrap();
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await
            .unwrap();
        let c = tx
            .add_node(Node::new("Account").with_property("name", str_val("C")))
            .await
            .unwrap();
        tx.add_edge(Edge::new(a, b, "TX")).unwrap();
        tx.add_edge(Edge::new(b, c, "TX")).unwrap();
        tx.commit().await.unwrap();

        graph
            .add_node_embedding(a, vec![1.0, 0.0], "node-minilm")
            .await
            .unwrap();
        graph
            .add_node_embedding(b, vec![1.0, 0.0], "node-minilm")
            .await
            .unwrap();
        graph
            .add_node_embedding(c, vec![0.0, 1.0], "node-minilm")
            .await
            .unwrap();
        for edge in graph.get_all_edges().await.unwrap() {
            graph
                .add_edge_embedding(edge.id, vec![10.0], "edge-relbert")
                .await
                .unwrap();
        }

        let result = graph
            .execute_nql(
                r#"
                find n.name, path_embedding("node-minilm", "edge-relbert") as path_vec
                from (a:Account {name: "A"})-[:TX]->{1,2}(n:Account)
            "#,
            )
            .await
            .unwrap();

        assert_eq!(result.len(), 2);
        let vectors: Vec<Vec<f64>> = result
            .rows()
            .iter()
            .filter_map(|row| row.get("path_vec"))
            .filter_map(|v| match v {
                PropertyValue::List(items) => Some(
                    items
                        .iter()
                        .map(|item| match item {
                            PropertyValue::Float(f) => *f,
                            other => panic!("expected float in path_vec, got {:?}", other),
                        })
                        .collect(),
                ),
                _ => None,
            })
            .collect();
        assert_eq!(vectors.len(), 2);
        assert!(vectors.iter().all(|v| v.len() == 3));
    }

    #[cfg(feature = "embeddings")]
    #[tokio::test]
    async fn test_path_embedding_without_alias_uses_unique_canonical_columns() {
        let graph = Graph::in_memory().await.unwrap();

        let mut tx = graph.begin_transaction().await.unwrap();
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await
            .unwrap();
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await
            .unwrap();
        let c = tx
            .add_node(Node::new("Account").with_property("name", str_val("C")))
            .await
            .unwrap();
        tx.add_edge(Edge::new(a, b, "TX")).unwrap();
        tx.add_edge(Edge::new(a, c, "TX")).unwrap();
        tx.commit().await.unwrap();

        graph
            .add_node_embedding(a, vec![1.0, 0.0], "node-minilm")
            .await
            .unwrap();
        graph
            .add_node_embedding(b, vec![1.0, 0.0], "node-minilm")
            .await
            .unwrap();
        graph
            .add_node_embedding(c, vec![0.0, 1.0], "node-minilm")
            .await
            .unwrap();
        for edge in graph.get_all_edges().await.unwrap() {
            graph
                .add_edge_embedding(edge.id, vec![10.0], "edge-relbert")
                .await
                .unwrap();
            graph
                .add_edge_embedding(edge.id, vec![20.0], "edge-transe")
                .await
                .unwrap();
        }

        let key_relbert = "path_embedding(\"node-minilm\", \"edge-relbert\")".to_string();
        let key_transe = "path_embedding(\"node-minilm\", \"edge-transe\")".to_string();

        let result = graph
            .execute_nql(
                r#"
                find
                    n.name,
                    path_embedding("node-minilm", "edge-relbert"),
                    path_embedding("node-minilm", "edge-transe")
                from (a:Account {name: "A"})-[:TX]->(n:Account)
                where path_has_embeddings("node-minilm", "edge-relbert")
            "#,
            )
            .await
            .unwrap();

        assert!(result.columns.contains(&key_relbert));
        assert!(result.columns.contains(&key_transe));
        assert_ne!(key_relbert, key_transe);
        for row in result.rows() {
            assert!(matches!(
                row.get(&key_relbert),
                Some(PropertyValue::List(_))
            ));
            assert!(matches!(row.get(&key_transe), Some(PropertyValue::List(_))));
        }
    }

    #[cfg(feature = "embeddings")]
    #[tokio::test]
    async fn test_path_has_embeddings_filters_missing_node_or_edge_vectors() {
        let graph = Graph::in_memory().await.unwrap();

        let mut tx = graph.begin_transaction().await.unwrap();
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await
            .unwrap();
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await
            .unwrap();
        let c = tx
            .add_node(Node::new("Account").with_property("name", str_val("C")))
            .await
            .unwrap();
        tx.add_edge(Edge::new(a, b, "TX")).unwrap();
        tx.add_edge(Edge::new(b, c, "TX")).unwrap();
        tx.commit().await.unwrap();

        graph
            .add_node_embedding(a, vec![1.0, 0.0], "node-minilm")
            .await
            .unwrap();
        graph
            .add_node_embedding(b, vec![1.0, 0.0], "node-minilm")
            .await
            .unwrap();
        // Obtener el edge A→B de forma determinista (get_all_edges no garantiza orden)
        let edge_a_b = graph
            .get_outgoing_edges(a)
            .await
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        graph
            .add_edge_embedding(edge_a_b.id, vec![10.0], "edge-relbert")
            .await
            .unwrap();

        let result = graph
            .execute_nql(
                r#"
                find n.name
                from (a:Account {name: "A"})-[:TX]->{1,2}(n:Account)
                where path_has_embeddings("node-minilm", "edge-relbert")
            "#,
            )
            .await
            .unwrap();

        assert_eq!(result.len(), 1);
        assert_eq!(
            result.rows()[0].get("n.name"),
            Some(&PropertyValue::String("B".to_string()))
        );
    }

    #[cfg(feature = "embeddings")]
    #[tokio::test]
    async fn test_path_embedding_similarity_rejected_with_migration_error() {
        let graph = Graph::in_memory().await.unwrap();

        let mut tx = graph.begin_transaction().await.unwrap();
        let a = tx
            .add_node(Node::new("Account").with_property("name", str_val("A")))
            .await
            .unwrap();
        let b = tx
            .add_node(Node::new("Account").with_property("name", str_val("B")))
            .await
            .unwrap();
        let c = tx
            .add_node(Node::new("Account").with_property("name", str_val("C")))
            .await
            .unwrap();
        tx.add_edge(Edge::new(a, b, "TX")).unwrap();
        tx.add_edge(Edge::new(a, c, "TX")).unwrap();
        tx.commit().await.unwrap();

        graph
            .add_node_embedding(a, vec![1.0, 0.0], "node-minilm")
            .await
            .unwrap();

        let err = graph
            .execute_nql(&format!(
                r#"
                find n.name, path_embedding_similarity("{b}", "node-minilm") as sem_score
                from (a:Account {{name: "A"}})-[:TX]->(n:Account)
                where true
            "#
            ))
            .await
            .expect_err("legacy path_embedding_similarity must fail with migration error");
        assert!(err.to_string().contains("requires exactly 3 arguments"));
    }
}
