// src/query/nql/executor/aggregations.rs
//
// Aggregation functions for NQL

#[cfg(feature = "algorithms")]
use std::cmp::Reverse;
#[cfg(any(feature = "algorithms", feature = "embeddings-index"))]
use std::collections::HashMap;
#[cfg(feature = "algorithms")]
use std::collections::HashSet;
use crate::error::{NopalError, Result};
use crate::query::nql::executor::result::{QueryResult, Row};
use crate::query::nql::parser::ast::{Expression, GroupByClause, Projection, Query};
use crate::types::{Node, PropertyValue};
#[cfg(feature = "algorithms")]
use crate::graph::Direction;
use crate::graph::Graph;
#[cfg(feature = "algorithms")]
use crate::types::NodeId;

// ─────────────────────────────────────────────────────────────────────────────
// AlgoResults — resultados precomputados de algoritmos de grafos
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(feature = "algorithms")]
#[derive(Default)]
pub struct AlgoResults {
    pub pagerank: Option<HashMap<NodeId, f64>>,
    pub betweenness: Option<HashMap<NodeId, f64>>,
    pub clustering: Option<HashMap<NodeId, f64>>,
    pub degree: Option<HashMap<NodeId, f64>>,
    pub community: Option<HashMap<NodeId, usize>>,
    /// Resultado de leiden() — caché separada de community() (Louvain).
    pub leiden: Option<HashMap<NodeId, usize>>,
}

#[cfg(not(feature = "algorithms"))]
#[derive(Default)]
pub struct AlgoResults {}

/// Pre-compute algorithm caches needed by a query.
///
/// Inspect the projections and HAVING clause to figure out which algorithms
/// to run. Returns an `AlgoResults` with the per-node maps populated.
/// Used by the pattern executor for per-row algorithm lookups in
/// non-aggregated queries (e.g. `find ... degree(e) ... from (e) -> (j)`).
#[cfg(feature = "algorithms")]
pub async fn precompute_for_query(
    graph: &Graph,
    nodes: &[Node],
    query: &Query,
) -> Result<AlgoResults> {
    precompute_algorithms(graph, nodes, query).await
}

#[cfg(not(feature = "algorithms"))]
pub async fn precompute_for_query(
    _graph: &Graph,
    _nodes: &[Node],
    _query: &Query,
) -> Result<AlgoResults> {
    Ok(AlgoResults::default())
}

/// Look up a single node's algorithm value from the cache by function name.
/// Returns `PropertyValue::Null` if the algorithm wasn't pre-computed or the
/// node is missing from the cache.
#[cfg(feature = "algorithms")]
pub fn lookup_algo_value(
    name: &str,
    node_id: &NodeId,
    cache: &AlgoResults,
) -> PropertyValue {
    let lower = name.to_lowercase();
    match lower.as_str() {
        "pagerank" => cache.pagerank.as_ref()
            .and_then(|m| m.get(node_id))
            .map(|v| PropertyValue::Float(*v))
            .unwrap_or(PropertyValue::Null),
        "betweenness" => cache.betweenness.as_ref()
            .and_then(|m| m.get(node_id))
            .map(|v| PropertyValue::Float(*v))
            .unwrap_or(PropertyValue::Null),
        "clustering" => cache.clustering.as_ref()
            .and_then(|m| m.get(node_id))
            .map(|v| PropertyValue::Float(*v))
            .unwrap_or(PropertyValue::Null),
        "degree" => cache.degree.as_ref()
            .and_then(|m| m.get(node_id))
            .map(|v| PropertyValue::Float(*v))
            .unwrap_or(PropertyValue::Null),
        "community" | "community_fast" => cache.community.as_ref()
            .and_then(|m| m.get(node_id))
            .map(|v| PropertyValue::Int(*v as i64))
            .unwrap_or(PropertyValue::Null),
        "leiden" => cache.leiden.as_ref()
            .and_then(|m| m.get(node_id))
            .map(|v| PropertyValue::Int(*v as i64))
            .unwrap_or(PropertyValue::Null),
        _ => PropertyValue::Null,
    }
}

#[cfg(not(feature = "algorithms"))]
pub fn lookup_algo_value(
    _name: &str,
    _node_id: &crate::NodeId,
    _cache: &AlgoResults,
) -> PropertyValue {
    PropertyValue::Null
}

// ─────────────────────────────────────────────────────────────────────────────
// QueryContext — contexto combinado para la evaluación de una query
// Une AlgoResults (algoritmos) con índices de embeddings (embeddings-index).
// Siempre presente; campos internos son condicionalmente compilados.
// ─────────────────────────────────────────────────────────────────────────────

struct QueryContext {
    #[cfg_attr(not(feature = "algorithms"), allow(dead_code))]
    algo: AlgoResults,
    #[cfg(feature = "embeddings-index")]
    emb_indices: HashMap<String, std::sync::Arc<crate::embeddings::HnswIndex>>,
}

impl QueryContext {
    fn new(algo: AlgoResults) -> Self {
        Self {
            algo,
            #[cfg(feature = "embeddings-index")]
            emb_indices: HashMap::new(),
        }
    }
}

/// Check if query projections contain TRUE aggregations or per-node algorithms
/// or embedding aggregations — i.e. anything que requiera enrutarse al engine
/// de agregaciones (con pre-cómputo de algoritmos).
///
/// Mantiene el comportamiento histórico de routing. Para distinguir
/// aggregations puras de algorithms, usar `has_real_aggregations` y
/// `has_algorithm_projections` por separado.
pub fn has_aggregations(projections: &[Projection]) -> bool {
    projections.iter().any(|p| match p {
        Projection::Expression { expr, .. } => is_aggregation_or_algorithm_expr(expr),
        _ => false,
    })
}

/// Returns true ONLY for true aggregations (count/sum/avg/min/max).
/// Used to decide if the engine should collapse rows.
pub fn has_real_aggregations(projections: &[Projection]) -> bool {
    projections.iter().any(|p| match p {
        Projection::Expression { expr, .. } => expr.is_aggregation(),
        _ => false,
    })
}

/// Returns true if any projection contains a per-node algorithm function.
pub fn has_algorithm_projections(projections: &[Projection]) -> bool {
    projections.iter().any(|p| match p {
        Projection::Expression { expr, .. } => expr.is_algorithm(),
        _ => false,
    })
}

fn is_aggregation_or_algorithm_expr(expr: &Expression) -> bool {
    match expr {
        Expression::FunctionCall { name, .. } => {
            if matches!(name.as_str(), "count" | "sum" | "avg" | "min" | "max") {
                return true;
            }
            if matches!(name.as_str(), "pagerank" | "betweenness" | "clustering" | "degree" | "community" | "community_fast" | "leiden" | "shortestPath") {
                return true;
            }
            #[cfg(feature = "embeddings")]
            if matches!(name.as_str(), "embedding_similarity" | "knn_nodes") {
                return true;
            }
            false
        }
        _ => false,
    }
}

/// Execute aggregation query
pub async fn execute_aggregations(
    graph: &Graph,
    nodes: Vec<Node>,
    query: &Query,
    variable: &str,
) -> Result<QueryResult> {
    let algo_results = precompute_algorithms(graph, &nodes, query).await?;
    #[cfg(feature = "embeddings-index")]
    let mut ctx = QueryContext::new(algo_results);
    #[cfg(not(feature = "embeddings-index"))]
    let ctx = QueryContext::new(algo_results);

    // Precomputar índices HNSW para cualquier función knn_nodes(n, k, "model")
    // detectada en las proyecciones. Un índice por modelo único.
    #[cfg(feature = "embeddings-index")]
    for projection in &query.find.projections {
        if let Projection::Expression { expr, .. } = projection
            && let Expression::FunctionCall { name, args } = expr
            && name == "knn_nodes"
            && args.len() == 3
            && let Expression::Literal(PropertyValue::String(model)) = &args[2]
            && !ctx.emb_indices.contains_key(model)
            && let Ok(idx) = graph.get_or_build_embedding_index(model).await
        {
            ctx.emb_indices.insert(model.clone(), idx);
        }
    }

    // Check if there's a GROUP BY clause
    if let Some(group_by) = &query.group_by {
        execute_grouped_aggregation(graph, nodes, query, variable, group_by, &ctx).await
    } else {
        // SQL-like convenience: if projections mix aggregations with plain
        // property expressions, treat plain expressions as an implicit GROUP BY.
        let implicit_group_exprs: Vec<Expression> = query.find.projections.iter()
            .filter_map(|projection| {
                if let Projection::Expression { expr, .. } = projection
                    && !is_aggregation_or_algorithm_expr(expr)
                    && matches!(expr, Expression::Property { .. }) {
                        return Some(expr.clone());
                }
                None
            })
            .collect();

        if implicit_group_exprs.is_empty() {
            execute_simple_aggregation(graph, nodes, query, variable, &ctx).await
        } else {
            let implicit_group_by = GroupByClause {
                expressions: implicit_group_exprs,
            };
            execute_grouped_aggregation(graph, nodes, query, variable, &implicit_group_by, &ctx).await
        }
    }
}

/// Execute simple aggregation (no GROUP BY)
async fn execute_simple_aggregation(
    graph: &Graph,
    nodes: Vec<Node>,
    query: &Query,
    variable: &str,
    ctx: &QueryContext,
) -> Result<QueryResult> {
    let mut columns = Vec::new();
    let mut row = Row::new();

    for projection in &query.find.projections {
        if let Projection::Expression { expr, alias } = projection {
            let (key, value) = evaluate_aggregation(graph, expr, &nodes, variable, alias, ctx).await?;
            columns.push(key.clone());
            row.set(key, value);
        }
    }

    let mut result = QueryResult::new(columns);
    result.add_row(row);
    Ok(result)
}

/// Execute grouped aggregation (with GROUP BY)
async fn execute_grouped_aggregation(
    graph: &Graph,
    nodes: Vec<Node>,
    query: &Query,
    variable: &str,
    group_by: &crate::query::nql::parser::ast::GroupByClause,
    ctx: &QueryContext,
) -> Result<QueryResult> {
    // Group nodes by ALL GROUP BY expressions (I2: multi-column support).
    // BTreeMap gives deterministic iteration order (lexicographic by group key),
    // so `limit 1` always returns the same row for the same data.
    let mut groups: std::collections::BTreeMap<String, Vec<Node>> = std::collections::BTreeMap::new();

    for node in nodes {
        let group_key = group_by.expressions.iter()
            .map(|expr| evaluate_group_key(expr, &node, variable))
            .collect::<Result<Vec<_>>>()?
            .join("|");
        groups.entry(group_key).or_default().push(node);
    }

    // Always inject `var.id` as the first column so that the graph-hint system
    // can detect UUID values without needing a separate fallback query.
    // This column is populated with the UUID of the representative node in each group.
    let node_id_col = format!("{}.id", variable);
    let mut columns = vec![node_id_col.clone()];

    // Add ALL GROUP BY columns (I2), skipping `var.id` if the user already wrote it.
    // Si la proyección declara un alias para esta expresión (e.g. `n.label as etiqueta`),
    // usar el alias como nombre de columna para que el resultado lo exponga correctamente.
    for expr in &group_by.expressions {
        if let Expression::Property { variable: v, property } = expr {
            let raw_col = format!("{}.{}", v, property);
            if raw_col == node_id_col {
                continue;
            }
            let col_name = query.find.projections.iter()
                .find_map(|p| {
                    if let Projection::Expression { expr: pexpr, alias: Some(a) } = p
                        && let Expression::Property { variable: pv, property: pp } = pexpr
                        && pv == v && pp == property
                    {
                        Some(a.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or(raw_col);
            columns.push(col_name);
        }
    }

    // Add aggregation columns
    for projection in &query.find.projections {
        if let Projection::Expression { expr, alias } = projection
            && is_aggregation_or_algorithm_expr(expr)
            && let Expression::FunctionCall { name, .. } = expr {
                let col_name = alias.clone().unwrap_or_else(|| name.clone());
                columns.push(col_name);
        }
    }

    let mut result = QueryResult::new(columns);

    for (group_key, group_nodes) in groups {
        let mut row = Row::new();

        // Inject the UUID of the representative node for this group so the
        // graph-hint system (row_graph_hints_from_query_result) can focus correctly.
        if let Some(first_node) = group_nodes.first() {
            row.set(node_id_col.clone(), PropertyValue::String(first_node.id.to_string()));
        }

        // Parse composite key back and add all group columns (I2).
        // Usar el mismo nombre de columna (con alias si aplica) calculado arriba.
        let key_parts: Vec<&str> = group_key.split('|').collect();
        for (i, expr) in group_by.expressions.iter().enumerate() {
            if let Expression::Property { variable: v, property } = expr {
                let raw_key = format!("{}.{}", v, property);
                if raw_key == node_id_col {
                    continue;
                }
                let row_key = query.find.projections.iter()
                    .find_map(|p| {
                        if let Projection::Expression { expr: pexpr, alias: Some(a) } = p
                            && let Expression::Property { variable: pv, property: pp } = pexpr
                            && pv == v && pp == property
                        {
                            Some(a.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or(raw_key);
                let value = key_parts.get(i)
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| "null".to_string());
                row.set(row_key, PropertyValue::String(value));
            }
        }

        // Compute aggregations for this group
        for projection in &query.find.projections {
            if let Projection::Expression { expr, alias } = projection
                && is_aggregation_or_algorithm_expr(expr) {
                    let (key, value) = evaluate_aggregation(graph, expr, &group_nodes, variable, alias, ctx).await?;
                    row.set(key, value);
            }
        }

        result.add_row(row);
    }

    Ok(result)
}

/// Evaluate GROUP BY key
fn evaluate_group_key(expr: &Expression, node: &Node, variable: &str) -> Result<String> {
    match expr {
        Expression::Property { variable: var, property } => {
            if var != variable {
                return Err(NopalError::QueryExecutionError(
                    format!("Unknown variable in GROUP BY: {}", var)
                ));
            }

            // ✨ SPECIAL CASE: "label" is a Node field, not a property
            if property == "label" {
                return Ok(node.label.clone());
            }

            // ✨ SPECIAL CASE: "id" is a Node field, not a property
            if property == "id" {
                return Ok(node.id.to_string());
            }

            // Otherwise, get from properties
            Ok(node.properties.get(property)
                .map(|v| match v {
                    PropertyValue::String(s) => s.clone(),
                    PropertyValue::Int(i) => i.to_string(),
                    PropertyValue::Float(f) => f.to_string(),
                    PropertyValue::Bool(b) => b.to_string(),
                    PropertyValue::Null => "null".to_string(),
                    PropertyValue::Bytes(_) => "<bytes>".to_string(),
                    PropertyValue::List(_) => "<list>".to_string(),
                    PropertyValue::Object(_) => "<object>".to_string(),
                })
                .unwrap_or_else(|| "null".to_string()))
        }
        _ => Err(NopalError::QueryExecutionError(
            "GROUP BY only supports property expressions".into()
        )),
    }
}

/// Evaluate aggregation expression
#[allow(unused_variables)]
async fn evaluate_aggregation(
    graph: &Graph,
    expr: &Expression,
    nodes: &[Node],
    variable: &str,
    alias: &Option<String>,
    ctx: &QueryContext,
) -> Result<(String, PropertyValue)> {
    match expr {
        Expression::FunctionCall { name, args } => {
            let result_key = alias.clone().unwrap_or_else(|| name.clone());

            match name.as_str() {
                "count" => {
                    let count = nodes.len() as i64;
                    Ok((result_key, PropertyValue::Int(count)))
                }
                "sum" => {
                    if args.is_empty() {
                        return Err(NopalError::QueryExecutionError(
                            "sum() requires an argument".into()
                        ));
                    }

                    let sum = sum_property(nodes, &args[0], variable)?;
                    Ok((result_key, PropertyValue::Float(sum)))
                }
                "avg" => {
                    if args.is_empty() {
                        return Err(NopalError::QueryExecutionError(
                            "avg() requires an argument".into()
                        ));
                    }

                    let sum = sum_property(nodes, &args[0], variable)?;
                    let avg = sum / nodes.len() as f64;
                    Ok((result_key, PropertyValue::Float(avg)))
                }
                "min" => {
                    if args.is_empty() {
                        return Err(NopalError::QueryExecutionError(
                            "min() requires an argument".into()
                        ));
                    }

                    let min = min_property(nodes, &args[0], variable)?;
                    Ok((result_key, min))
                }
                "max" => {
                    if args.is_empty() {
                        return Err(NopalError::QueryExecutionError(
                            "max() requires an argument".into()
                        ));
                    }

                    let max = max_property(nodes, &args[0], variable)?;
                    Ok((result_key, max))
                }
                #[cfg(feature = "algorithms")]
                "pagerank" => {
                    let ranks = ctx.algo.pagerank.as_ref().ok_or_else(|| NopalError::QueryExecutionError("pagerank algorithm not precomputed".into()))?;
                    Ok((result_key, PropertyValue::Float(average_algo_scores(nodes, ranks))))
                }
                #[cfg(feature = "algorithms")]
                "betweenness" => {
                    let scores = ctx.algo.betweenness.as_ref().ok_or_else(|| NopalError::QueryExecutionError("betweenness algorithm not precomputed".into()))?;
                    Ok((result_key, PropertyValue::Float(average_algo_scores(nodes, scores))))
                }
                #[cfg(feature = "algorithms")]
                "clustering" => {
                    let coeffs = ctx.algo.clustering.as_ref().ok_or_else(|| NopalError::QueryExecutionError("clustering algorithm not precomputed".into()))?;
                    Ok((result_key, PropertyValue::Float(average_algo_scores(nodes, coeffs))))
                }
                #[cfg(feature = "algorithms")]
                "degree" => {
                    let degrees = ctx.algo.degree.as_ref().ok_or_else(|| NopalError::QueryExecutionError("degree algorithm not precomputed".into()))?;
                    Ok((result_key, PropertyValue::Float(average_algo_scores(nodes, degrees))))
                }
                #[cfg(feature = "algorithms")]
                "community" | "community_fast" => {
                    let communities = ctx.algo.community.as_ref().ok_or_else(|| NopalError::QueryExecutionError("community algorithm not precomputed".into()))?;
                    Ok((result_key, PropertyValue::Float(average_algo_scores_usize(nodes, communities))))
                }
                #[cfg(feature = "algorithms")]
                // leiden(n) — Constant Potts Model community detection (Traag et al. 2019).
                // Retorna el ID de comunidad Leiden como Float.
                // Usa caché separada de community() para que ambos puedan coexistir en la misma query.
                "leiden" => {
                    let communities = ctx.algo.leiden.as_ref().ok_or_else(|| NopalError::QueryExecutionError(
                        "leiden algorithm not precomputed — asegúrate de tener feature `algorithms`".into()
                    ))?;
                    Ok((result_key, PropertyValue::Float(average_algo_scores_usize(nodes, communities))))
                }
                #[cfg(feature = "algorithms")]
                "shortestPath" => {
                    execute_shortest_path(graph, args, &result_key).await
                }
                #[cfg(not(feature = "algorithms"))]
                "pagerank" | "betweenness" | "clustering" | "degree" | "community" | "community_fast" | "leiden" | "shortestPath" => {
                    Err(NopalError::QueryExecutionError(
                        format!("{}() requires feature `algorithms` (enable `--features core` or `--features algorithms`)", name)
                    ))
                }

                // ── Embedding functions ────────────────────────────────────
                //
                // embedding_similarity(n, "ref-uuid", "model")
                //   Retorna la similitud coseno promedio entre todos los nodos
                //   del grupo y el nodo de referencia. Para grupos de un nodo
                //   (implicit GROUP BY) equivale a la similitud individual.
                //
                // knn_nodes(n, k, "model")
                //   Retorna los k vecinos más cercanos al primer nodo del
                //   grupo como JSON array de UUIDs. Usa el EmbeddingIndex
                //   HNSW precomputado en QueryContext.
                #[cfg(feature = "embeddings")]
                "embedding_similarity" => {
                    execute_embedding_similarity(graph, args, nodes, &result_key).await
                }
                #[cfg(feature = "embeddings-index")]
                "knn_nodes" => {
                    execute_knn_nodes(graph, args, nodes, &result_key, ctx).await
                }

                _ => Err(NopalError::QueryExecutionError(
                    format!("Unknown aggregation function: {}", name)
                ))
            }
        }
        _ => Err(NopalError::QueryExecutionError(
            "Not an aggregation expression".into()
        ))
    }
}

/// Sum numeric property
fn sum_property(nodes: &[Node], expr: &Expression, variable: &str) -> Result<f64> {
    match expr {
        Expression::Property { variable: var, property } => {
            if var != variable {
                return Err(NopalError::QueryExecutionError(
                    format!("Unknown variable: {}", var)
                ));
            }

            let mut sum = 0.0;
            for node in nodes {
                if let Some(value) = node.properties.get(property) {
                    sum += match value {
                        PropertyValue::Int(i) => *i as f64,
                        PropertyValue::Float(f) => *f,
                        _ => 0.0,
                    };
                }
            }
            Ok(sum)
        }
        _ => Err(NopalError::QueryExecutionError(
            "Aggregation argument must be a property".into()
        ))
    }
}

/// Find minimum property value
fn min_property(nodes: &[Node], expr: &Expression, variable: &str) -> Result<PropertyValue> {
    match expr {
        Expression::Property { variable: var, property } => {
            if var != variable {
                return Err(NopalError::QueryExecutionError(
                    format!("Unknown variable: {}", var)
                ));
            }

            let mut min: Option<PropertyValue> = None;
            for node in nodes {
                if let Some(value) = node.properties.get(property)
                    && min.as_ref().is_none_or(|m| is_less_than(value, m)) {
                        min = Some(value.clone());
                }
            }

            min.ok_or_else(|| NopalError::QueryExecutionError(
                "No values found for min()".into()
            ))
        }
        _ => Err(NopalError::QueryExecutionError(
            "Aggregation argument must be a property".into()
        ))
    }
}

/// Find maximum property value
fn max_property(nodes: &[Node], expr: &Expression, variable: &str) -> Result<PropertyValue> {
    match expr {
        Expression::Property { variable: var, property } => {
            if var != variable {
                return Err(NopalError::QueryExecutionError(
                    format!("Unknown variable: {}", var)
                ));
            }

            let mut max: Option<PropertyValue> = None;
            for node in nodes {
                if let Some(value) = node.properties.get(property)
                    && max.as_ref().is_none_or(|m| is_greater_than(value, m)) {
                        max = Some(value.clone());
                }
            }

            max.ok_or_else(|| NopalError::QueryExecutionError(
                "No values found for max()".into()
            ))
        }
        _ => Err(NopalError::QueryExecutionError(
            "Aggregation argument must be a property".into()
        ))
    }
}

/// Compare PropertyValues (less than)
fn is_less_than(a: &PropertyValue, b: &PropertyValue) -> bool {
    match (a, b) {
        (PropertyValue::Int(a), PropertyValue::Int(b)) => a < b,
        (PropertyValue::Float(a), PropertyValue::Float(b)) => a < b,
        (PropertyValue::Int(a), PropertyValue::Float(b)) => (*a as f64) < *b,
        (PropertyValue::Float(a), PropertyValue::Int(b)) => *a < (*b as f64),
        _ => false,
    }
}

/// Compare PropertyValues (greater than)
fn is_greater_than(a: &PropertyValue, b: &PropertyValue) -> bool {
    match (a, b) {
        (PropertyValue::Int(a), PropertyValue::Int(b)) => a > b,
        (PropertyValue::Float(a), PropertyValue::Float(b)) => a > b,
        (PropertyValue::Int(a), PropertyValue::Float(b)) => (*a as f64) > *b,
        (PropertyValue::Float(a), PropertyValue::Int(b)) => *a > (*b as f64),
        _ => false,
    }
}

#[cfg(feature = "algorithms")]
async fn precompute_algorithms(
    graph: &Graph,
    nodes: &[Node],
    query: &Query,
) -> Result<AlgoResults> {
    let mut results = AlgoResults::default();

    // Collect every algorithm function name referenced anywhere in the
    // query: projections, WHERE, HAVING. Bug #67: previously we only
    // inspected projections, so `where degree(e) > 3` had no pre-computed
    // cache and post-filtering returned 0 rows.
    let mut needed: std::collections::HashSet<String> = std::collections::HashSet::new();
    for projection in &query.find.projections {
        if let Projection::Expression { expr, .. } = projection {
            collect_algorithm_names(expr, &mut needed);
        }
    }
    if let Some(filter) = &query.filter {
        collect_algorithm_names(&filter.condition, &mut needed);
    }
    if let Some(having) = &query.having {
        collect_algorithm_names(&having.condition, &mut needed);
    }

    if needed.is_empty() && !has_aggregations(&query.find.projections) {
        return Ok(results);
    }

    use crate::graph::view::Subgraph;
    let subgraph = Subgraph::from_nodes(graph, nodes);

    for name in &needed {
        match name.as_str() {
            "pagerank" if results.pagerank.is_none() => {
                use crate::algorithms::pagerank::{PageRank, PageRankConfig};
                let pr = PageRank::new(PageRankConfig { damping: 0.85, max_iterations: 100, tolerance: 1e-6, parallel: false });
                results.pagerank = Some(pr.compute(graph).await?);
            }
            "betweenness" if results.betweenness.is_none() => {
                use crate::algorithms::betweenness::{BetweennessCentrality, BetweennessConfig};
                let bc = BetweennessCentrality::new(BetweennessConfig { normalize: true, parallel: false });
                results.betweenness = Some(bc.compute(&subgraph).await?);
            }
            "clustering" if results.clustering.is_none() => {
                use crate::algorithms::clustering::{ClusteringCoefficient, ClusteringConfig};
                let cc = ClusteringCoefficient::new(ClusteringConfig { weighted: false });
                results.clustering = Some(cc.compute(&subgraph).await?);
            }
            "degree" if results.degree.is_none() => {
                use crate::algorithms::degree::{DegreeCentrality, DegreeConfig, DegreeType};
                let dc = DegreeCentrality::new(DegreeConfig { degree_type: DegreeType::Total, normalize: false });
                results.degree = Some(dc.compute(graph).await?);
            }
            "community" if results.community.is_none() => {
                results.community = Some(get_or_compute_community_exact(graph).await?);
            }
            "community_fast" if results.community.is_none() => {
                results.community = Some(compute_community_fast_map(graph, nodes).await?);
            }
            "leiden" if results.leiden.is_none() => {
                results.leiden = Some(get_or_compute_leiden(graph).await?);
            }
            _ => {}
        }
    }
    Ok(results)
}

#[cfg(feature = "algorithms")]
fn collect_algorithm_names(expr: &Expression, out: &mut std::collections::HashSet<String>) {
    match expr {
        Expression::FunctionCall { name, args } => {
            let lower = name.to_lowercase();
            if matches!(lower.as_str(),
                "pagerank" | "betweenness" | "clustering" | "degree"
                | "community" | "community_fast" | "leiden")
            {
                out.insert(lower);
            }
            for a in args {
                collect_algorithm_names(a, out);
            }
        }
        Expression::BinaryOp { left, right, .. } => {
            collect_algorithm_names(left, out);
            collect_algorithm_names(right, out);
        }
        Expression::UnaryOp { expr, .. } => collect_algorithm_names(expr, out),
        _ => {}
    }
}

#[cfg(not(feature = "algorithms"))]
async fn precompute_algorithms(
    _graph: &Graph,
    _nodes: &[Node],
    _query: &Query,
) -> Result<AlgoResults> {
    Ok(AlgoResults::default())
}

#[cfg(feature = "algorithms")]
fn average_algo_scores(nodes: &[Node], scores: &HashMap<NodeId, f64>) -> f64 {
    if nodes.is_empty() { return 0.0; }
    let sum: f64 = nodes.iter().filter_map(|n| scores.get(&n.id).copied()).sum();
    sum / nodes.len() as f64
}

#[cfg(feature = "algorithms")]
fn average_algo_scores_usize(nodes: &[Node], scores: &HashMap<NodeId, usize>) -> f64 {
    if nodes.is_empty() { return 0.0; }
    let sum: f64 = nodes.iter().filter_map(|n| scores.get(&n.id).map(|&v| v as f64)).sum();
    sum / nodes.len() as f64
}

#[cfg(feature = "algorithms")]
/// Get exact Louvain partition from cache if topology is unchanged; otherwise compute and store.
async fn get_or_compute_community_exact(graph: &Graph) -> Result<HashMap<uuid::Uuid, usize>> {
    use crate::algorithms::community::LouvainCommunity;

    let current_topology = graph.topology_version();
    if let Some((cached_topology, assignments)) = graph.get_cached_community_partition_exact().await
        && cached_topology == current_topology {
            return Ok(assignments);
    }

    let lc = LouvainCommunity::with_defaults();
    let assignments = lc.detect(graph).await?;
    graph
        .set_cached_community_partition_exact(current_topology, assignments.clone())
        .await;
    Ok(assignments)
}

#[cfg(feature = "algorithms")]
/// Obtiene la partición Leiden desde caché si la topología no cambió; si no, recomputa y guarda.
///
/// La caché es independiente de la caché de Louvain (`get_or_compute_community_exact`):
/// leiden() y community() pueden coexistir en la misma query sin interferencia.
///
/// El parámetro gamma usado aquí es el default de `LeidenConfig` (0.1).
/// Para gamma personalizado, usar la Rust API directamente: `LeidenCommunity::with_gamma(g)`.
async fn get_or_compute_leiden(graph: &Graph) -> Result<HashMap<uuid::Uuid, usize>> {
    use crate::algorithms::community::LeidenCommunity;

    let current_topology = graph.topology_version();
    if let Some((cached_topology, assignments)) = graph.get_cached_leiden_partition().await
        && cached_topology == current_topology
    {
        return Ok(assignments);
    }

    let leiden = LeidenCommunity::with_defaults();
    let assignments = leiden.detect(graph).await?;
    graph.set_cached_leiden_partition(current_topology, assignments.clone()).await;
    Ok(assignments)
}

#[cfg(feature = "algorithms")]
/// Approximate community labels via local induced-subgraph label propagation (few iterations).
///
/// Returns per-node community assignments as a map. Used by `community_fast()` in NQL.
/// This avoids global Louvain computation and is intended for low-latency exploration.
async fn compute_community_fast_map(
    graph: &Graph,
    nodes: &[Node],
) -> Result<HashMap<NodeId, usize>> {
    if nodes.is_empty() {
        return Ok(HashMap::new());
    }

    let selected_ids: HashSet<uuid::Uuid> = nodes.iter().map(|n| n.id).collect();
    let mut adjacency: HashMap<uuid::Uuid, Vec<uuid::Uuid>> = HashMap::new();

    for node in nodes {
        let mut local = graph.neighbors(node.id, Direction::Both).await?
            .into_iter()
            .filter(|nid| selected_ids.contains(nid))
            .collect::<Vec<_>>();
        local.sort_unstable();
        local.dedup();
        adjacency.insert(node.id, local);
    }

    let mut labels: HashMap<uuid::Uuid, usize> = HashMap::new();
    for (idx, node) in nodes.iter().enumerate() {
        labels.insert(node.id, idx);
    }

    // Small fixed number of iterations keeps runtime predictable.
    for _ in 0..3 {
        let mut changed = false;
        let mut next_labels = labels.clone();

        for node in nodes {
            let neighbors = adjacency
                .get(&node.id)
                .map(|n| n.as_slice())
                .unwrap_or(&[]);
            if neighbors.is_empty() {
                continue;
            }

            let mut counts: HashMap<usize, usize> = HashMap::new();
            for neighbor in neighbors {
                if let Some(label) = labels.get(neighbor) {
                    *counts.entry(*label).or_insert(0) += 1;
                }
            }
            if counts.is_empty() {
                continue;
            }

            let best_label = counts
                .into_iter()
                .max_by_key(|(label, count)| (*count, Reverse(*label)))
                .map(|(label, _)| label)
                .unwrap_or_else(|| labels[&node.id]);

            if best_label != labels[&node.id] {
                changed = true;
                next_labels.insert(node.id, best_label);
            }
        }

        labels = next_labels;
        if !changed {
            break;
        }
    }

    // Densify label IDs to stable 0..N ordering for predictable averages.
    let mut compact_map: HashMap<usize, usize> = HashMap::new();
    let mut next_compact = 0usize;
    for node in nodes {
        let label = labels[&node.id];
        compact_map.entry(label).or_insert_with(|| {
            let current = next_compact;
            next_compact += 1;
            current
        });
    }

    let result: HashMap<NodeId, usize> = nodes
        .iter()
        .map(|n| (n.id, compact_map[&labels[&n.id]]))
        .collect();

    Ok(result)
}

#[cfg(feature = "algorithms")]
/// Execute Shortest Path algorithm.
///
/// Expects exactly two String-literal arguments that are node UUID strings:
///   `shortestPath("uuid-of-source", "uuid-of-target")`
///
/// Returns the path distance as Float, or -1.0 when no path exists.
async fn execute_shortest_path(
    graph: &Graph,
    args: &[Expression],
    result_key: &str,
) -> Result<(String, PropertyValue)> {
    use crate::algorithms::shortest_path::{ShortestPath, ShortestPathConfig};
    use uuid::Uuid;

    if args.len() != 2 {
        return Err(NopalError::QueryExecutionError(
            "shortestPath() requires exactly two arguments (source_id, target_id)".into(),
        ));
    }

    // Extract source UUID string from arg 0
    let source_str = match &args[0] {
        Expression::Literal(PropertyValue::String(s)) => s.clone(),
        _ => return Err(NopalError::QueryExecutionError(
            "shortestPath() arguments must be string literals (node UUIDs)".into(),
        )),
    };

    // Extract target UUID string from arg 1
    let target_str = match &args[1] {
        Expression::Literal(PropertyValue::String(s)) => s.clone(),
        _ => return Err(NopalError::QueryExecutionError(
            "shortestPath() arguments must be string literals (node UUIDs)".into(),
        )),
    };

    let source = Uuid::parse_str(&source_str).map_err(|_| {
        NopalError::QueryExecutionError(format!("Invalid node UUID for source: {}", source_str))
    })?;
    let target = Uuid::parse_str(&target_str).map_err(|_| {
        NopalError::QueryExecutionError(format!("Invalid node UUID for target: {}", target_str))
    })?;

    let sp = ShortestPath::new(ShortestPathConfig::default());
    let result = sp.find_path(graph, source, target).await?;

    let distance = match result {
        Some(path_result) => path_result.distance,
        None => -1.0, // No path found
    };

    Ok((result_key.to_string(), PropertyValue::Float(distance)))
}

// ─────────────────────────────────────────────────────────────────────────────
// Embedding NQL functions
// ─────────────────────────────────────────────────────────────────────────────

/// `embedding_similarity(n, "ref-uuid", "model")`
///
/// Retorna la similitud coseno promedio entre los embeddings de todos los nodos
/// del grupo y el embedding del nodo de referencia.
/// Nodos sin embedding son ignorados. Si ningún nodo tiene embedding, retorna 0.0.
#[cfg(feature = "embeddings")]
async fn execute_embedding_similarity(
    graph: &Graph,
    args: &[Expression],
    nodes: &[Node],
    result_key: &str,
) -> Result<(String, PropertyValue)> {
    if args.len() != 3 {
        return Err(NopalError::QueryExecutionError(
            "embedding_similarity(n, \"ref-uuid\", \"model\") requires 3 arguments".into(),
        ));
    }
    let ref_uuid_str = match &args[1] {
        Expression::Literal(PropertyValue::String(s)) => s.clone(),
        _ => return Err(NopalError::QueryExecutionError(
            "embedding_similarity: second argument must be a UUID string".into(),
        )),
    };
    let model = match &args[2] {
        Expression::Literal(PropertyValue::String(s)) => s.clone(),
        _ => return Err(NopalError::QueryExecutionError(
            "embedding_similarity: third argument must be a model name string".into(),
        )),
    };

    let ref_id = uuid::Uuid::parse_str(&ref_uuid_str).map_err(|_| {
        NopalError::QueryExecutionError(format!(
            "embedding_similarity: invalid UUID '{}'", ref_uuid_str
        ))
    })?;

    // Cargar el embedding de referencia una sola vez
    let ref_emb = graph.get_node_embedding(ref_id, &model).await
        .map_err(|_| NopalError::QueryExecutionError(format!(
            "embedding_similarity: no embedding found for reference node '{}' model '{}'",
            ref_uuid_str, model
        )))?;

    // Calcular similitud promedio sobre los nodos del grupo
    let mut total = 0.0_f64;
    let mut count = 0_usize;
    for node in nodes {
        if let Ok(emb) = graph.get_node_embedding(node.id, &model).await {
            total += emb.cosine_similarity(&ref_emb) as f64;
            count += 1;
        }
    }

    let avg = if count > 0 { total / count as f64 } else { 0.0 };
    Ok((result_key.to_string(), PropertyValue::Float(avg)))
}

/// `knn_nodes(n, k, "model")`
///
/// Retorna los k nodos más cercanos al primer nodo del grupo (o al único nodo
/// si no hay GROUP BY) en el espacio de embeddings del modelo dado.
///
/// Usa el `HnswIndex` HNSW precomputado en `QueryContext`.
/// Devuelve los UUIDs como JSON array en un `PropertyValue::String`.
/// Si el nodo no tiene embedding, retorna un array vacío `"[]"`.
#[cfg(feature = "embeddings-index")]
async fn execute_knn_nodes(
    graph: &Graph,
    args: &[Expression],
    nodes: &[Node],
    result_key: &str,
    ctx: &QueryContext,
) -> Result<(String, PropertyValue)> {
    if args.len() != 3 {
        return Err(NopalError::QueryExecutionError(
            "knn_nodes(n, k, \"model\") requires 3 arguments".into(),
        ));
    }
    let k = match &args[1] {
        Expression::Literal(PropertyValue::Int(k)) => *k as usize,
        _ => return Err(NopalError::QueryExecutionError(
            "knn_nodes: second argument must be an integer k".into(),
        )),
    };
    let model = match &args[2] {
        Expression::Literal(PropertyValue::String(s)) => s.clone(),
        _ => return Err(NopalError::QueryExecutionError(
            "knn_nodes: third argument must be a model name string".into(),
        )),
    };

    // Tomar el primer nodo del grupo como representante
    let Some(node) = nodes.first() else {
        return Ok((result_key.to_string(), PropertyValue::String("[]".into())));
    };

    // Cargar su embedding
    let emb = match graph.get_node_embedding(node.id, &model).await {
        Ok(e) => e,
        Err(_) => return Ok((result_key.to_string(), PropertyValue::String("[]".into()))),
    };

    // Buscar en el índice precomputado
    let index = ctx.emb_indices.get(&model).ok_or_else(|| {
        NopalError::QueryExecutionError(format!(
            "knn_nodes: no embedding index for model '{}' (not precomputed)",
            model
        ))
    })?;

    let results = index.search_knn(&emb.vector, k)?;
    let json = format!(
        "[{}]",
        results.iter()
            .map(|(id, _dist)| format!("\"{}\"", id))
            .collect::<Vec<_>>()
            .join(",")
    );
    Ok((result_key.to_string(), PropertyValue::String(json)))
}
