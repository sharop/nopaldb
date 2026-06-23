// NopalDB MCP Server — Phase E (stdio)
//
// Tools genéricos: graph_query, schema_info, get_node, get_neighbors, find_path,
//                  run_pagerank, similar_nodes, schema_by_kind,
//                  classify_node, list_instances, list_subclasses
// Tools de dominio: ver bloque gateado al final del archivo
// Resources: nopal://schema, nopal://stats
use std::sync::Arc;

use crate::tools::{
    is_write_statement, nql_result_to_tool, query_result_to_value, readonly_error, tool_error,
};
use nopaldb::Graph;
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars,
    service::RequestContext,
    tool, tool_handler, tool_router,
};
use serde::Deserialize;
use serde_json::json;

const MAX_ROWS: usize = 1000;
const DEFAULT_ROWS: u32 = 100;

// ─── Input types ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GraphQueryInput {
    /// NQL query to execute (FIND, ADD, UPDATE, DELETE, EXPLAIN, etc.)
    pub query: String,
    /// Maximum rows to return (default 100, max 1000)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetNodeInput {
    /// Node ID (UUID string). Provide either id or name.
    pub id: Option<String>,
    /// Node name property. Provide either id or name.
    pub name: Option<String>,
    /// Label to narrow the search (optional)
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetNeighborsInput {
    /// Node name or ID to start from
    pub id: String,
    /// Edge direction: "in", "out", or "both" (default "both")
    pub direction: Option<String>,
    /// Edge type filter (optional, e.g. "KNOWS")
    pub edge_type: Option<String>,
    /// Maximum neighbors to return (default 50)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindPathInput {
    /// Name of the source node
    pub from: String,
    /// Name of the target node
    pub to: String,
    /// Maximum path depth to search (default 6, max 10)
    pub max_depth: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RunPageRankInput {
    /// Label filter — compute PageRank only over nodes with this label (optional)
    pub label: Option<String>,
    /// Number of top-ranked nodes to return (default 10, max 100)
    pub top_n: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SimilarNodesInput {
    /// Name of the reference node whose embedding is used as the query vector
    pub name: String,
    /// Embedding model label used when the embeddings were stored (e.g. "minilm")
    pub model: String,
    /// Number of nearest neighbors to return (default 5, max 50)
    pub k: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ClassifyNodeInput {
    /// Name of the node to classify
    pub name: String,
    /// OWL class to test membership against (e.g. "Disease", "ViralInfection")
    pub class: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListInstancesInput {
    /// OWL class whose instances to list (e.g. "Disease")
    pub class: String,
    /// Maximum number of instances to return (default 20, max 200)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListSubclassesInput {
    /// OWL class whose subclasses to enumerate (e.g. "Disease")
    pub class: String,
}

// ─── Server struct ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct NopalMcpServer {
    graph: Arc<Graph>,
    readonly: bool,
    log_queries: bool,
    #[allow(dead_code)]
    tool_router: ToolRouter<NopalMcpServer>,
}

// ─── Tool implementations ──────────────────────────────────────────────────

impl NopalMcpServer {
    pub fn new(graph: Arc<Graph>, readonly: bool, log_queries: bool) -> Self {
        Self {
            graph,
            readonly,
            log_queries,
            tool_router: Self::build_router(),
        }
    }

    fn build_router() -> ToolRouter<NopalMcpServer> {
        Self::base_router()
    }
}

#[tool_router(router = base_router)]
impl NopalMcpServer {
    #[tool(
        description = "Execute an NQL graph query. Supports FIND, EXPLAIN, ADD, UPDATE, DELETE \
        (write ops require --no-readonly flag). Returns rows as a JSON array. \
        Example: 'find p.name, p.age from (p:Person) order by p.age desc limit 10'"
    )]
    async fn graph_query(
        &self,
        Parameters(GraphQueryInput { query, limit }): Parameters<GraphQueryInput>,
    ) -> Result<CallToolResult, McpError> {
        if self.readonly && is_write_statement(&query) {
            return Ok(readonly_error());
        }
        if self.log_queries {
            tracing::info!(query = %query, "graph_query");
        }
        let limit = limit.unwrap_or(DEFAULT_ROWS).min(MAX_ROWS as u32) as usize;
        let result = self.graph.execute_statement(&query).await;
        Ok(match result {
            Ok(r) => nql_result_to_tool(r, limit),
            Err(e) => tool_error(e),
        })
    }

    #[tool(
        description = "Return the graph schema: labels, edge types, node count, edge count. \
        No arguments required."
    )]
    async fn schema_info(
        &self,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        // Labels
        let labels_q = "find distinct n.label as label from (n) order by label";
        let labels_r = self.graph.execute_nql(labels_q).await;

        // Edge types
        let etypes_q =
            "find distinct r.label as edge_type from (a:*) -[r:*]-> (b:*) order by edge_type";
        let etypes_r = self.graph.execute_nql(etypes_q).await;

        // Counts
        let nc_q = "find count(*) as total from (n)";
        let ec_q = "find count(*) as total from (a:*) -[*]-> (b:*)";
        let nc_r = self.graph.execute_nql(nc_q).await;
        let ec_r = self.graph.execute_nql(ec_q).await;

        let labels: Vec<serde_json::Value> = labels_r
            .map(|r| {
                r.rows()
                    .iter()
                    .filter_map(|row| row.get_string("label"))
                    .map(serde_json::Value::String)
                    .collect()
            })
            .unwrap_or_default();

        let edge_types: Vec<serde_json::Value> = etypes_r
            .map(|r| {
                r.rows()
                    .iter()
                    .filter_map(|row| row.get_string("edge_type"))
                    .map(serde_json::Value::String)
                    .collect()
            })
            .unwrap_or_default();

        let node_count = nc_r
            .ok()
            .and_then(|r| r.rows().first().and_then(|row| row.get_int("total")))
            .unwrap_or(0);

        let edge_count = ec_r
            .ok()
            .and_then(|r| r.rows().first().and_then(|row| row.get_int("total")))
            .unwrap_or(0);

        Ok(CallToolResult::structured(json!({
            "labels":      labels,
            "edge_types":  edge_types,
            "node_count":  node_count,
            "edge_count":  edge_count,
            "readonly":    self.readonly,
        })))
    }

    #[tool(description = "Retrieve a single node by id or name. \
        Provide at least one of: id (UUID), name (string). \
        Optionally filter by label.")]
    async fn get_node(
        &self,
        Parameters(GetNodeInput { id, name, label }): Parameters<GetNodeInput>,
    ) -> Result<CallToolResult, McpError> {
        if id.is_none() && name.is_none() {
            return Ok(tool_error("Provide at least one of: id, name"));
        }
        // (n:Label) cuando hay filtro; (n) cuando no — label_spec no acepta *
        let node_spec = match label.as_deref() {
            Some(lbl) => format!("n:{lbl}"),
            None => "n".to_string(),
        };
        let nql = if let Some(ref node_id) = id {
            format!(
                "find n.label as label, n.name as name, n.id as id \
                 from ({node_spec}) where n.id = \"{node_id}\" limit 1"
            )
        } else {
            let node_name = name.as_deref().unwrap_or("");
            format!(
                "find n.label as label, n.name as name, n.id as id \
                 from ({node_spec}) where n.name = \"{node_name}\" limit 1"
            )
        };
        let result = self.graph.execute_nql(&nql).await;
        Ok(match result {
            Ok(r) if r.is_empty() => tool_error("Node not found"),
            Ok(r) => {
                let row = &r.rows()[0];
                let mut obj = serde_json::Map::new();
                for col in &r.columns {
                    if let Some(pv) = row.get(col) {
                        obj.insert(col.clone(), crate::tools::pv_to_json(pv));
                    }
                }
                CallToolResult::structured(serde_json::Value::Object(obj))
            }
            Err(e) => tool_error(e),
        })
    }

    #[tool(description = "Return the neighbors of a node. \
        'id' is the node name or UUID. \
        'direction': 'in', 'out', or 'both' (default 'both'). \
        'edge_type': optional edge label filter. \
        Returns an array of neighbor node summaries.")]
    async fn get_neighbors(
        &self,
        Parameters(GetNeighborsInput {
            id,
            direction,
            edge_type,
            limit,
        }): Parameters<GetNeighborsInput>,
    ) -> Result<CallToolResult, McpError> {
        let dir = direction.as_deref().unwrap_or("both");
        let lim = limit.unwrap_or(50).min(500);
        // [] = cualquier tipo de arista; [:TYPE] cuando se especifica tipo
        let edge_spec = match edge_type.as_deref() {
            Some(et) => format!(":{et}"),
            None => String::new(),
        };

        let nql = match dir {
            "out" => format!(
                "find b.label as label, b.name as name, b.id as id \
                 from (a) -[{edge_spec}]-> (b) where a.name = \"{id}\" or a.id = \"{id}\" \
                 limit {lim}"
            ),
            "in" => format!(
                "find b.label as label, b.name as name, b.id as id \
                 from (b) -[{edge_spec}]-> (a) where a.name = \"{id}\" or a.id = \"{id}\" \
                 limit {lim}"
            ),
            _ => format!(
                "find b.label as label, b.name as name, b.id as id \
                 from (a) -[{edge_spec}]- (b) where a.name = \"{id}\" or a.id = \"{id}\" \
                 limit {lim}"
            ),
        };
        let result = self.graph.execute_nql(&nql).await;
        Ok(match result {
            Ok(r) => CallToolResult::structured(query_result_to_value(&r)),
            Err(e) => tool_error(e),
        })
    }

    #[tool(description = "Find the shortest path between two nodes by name. \
        Returns path depth and the sequence of node names. \
        'from' and 'to' are node names. \
        'max_depth' limits search depth (default 6, max 10).")]
    async fn find_path(
        &self,
        Parameters(FindPathInput {
            from,
            to,
            max_depth,
        }): Parameters<FindPathInput>,
    ) -> Result<CallToolResult, McpError> {
        let depth = max_depth.unwrap_or(6).min(10);
        let nql = format!(
            "find path.depth as depth, path.start as start_node, path.end as end_node \
             from (a) -[]->{{1,{depth}}} (b) \
             where a.name = \"{from}\" and b.name = \"{to}\" \
             order by depth asc limit 1"
        );
        let result = self.graph.execute_nql(&nql).await;
        Ok(match result {
            Ok(r) if r.is_empty() => CallToolResult::structured(json!({
                "found": false,
                "message": format!("No path found between '{}' and '{}' within depth {}", from, to, depth),
            })),
            Ok(r) => {
                let row = &r.rows()[0];
                let d = row.get_int("depth").unwrap_or(-1);
                let mut v = json!({
                    "found": true,
                    "depth": d,
                    "from": from,
                    "to": to,
                });
                if let Some(start) = row.get("start_node") {
                    v["start_node"] = crate::tools::pv_to_json(start);
                }
                if let Some(end) = row.get("end_node") {
                    v["end_node"] = crate::tools::pv_to_json(end);
                }
                CallToolResult::structured(v)
            }
            Err(e) => tool_error(e),
        })
    }

    #[tool(
        description = "Run PageRank on the graph and return the top-N highest-ranked nodes. \
        Optionally filter by node label. \
        Returns [{name, id, label, score}] sorted by score descending. \
        Useful for finding the most influential nodes in a network."
    )]
    async fn run_pagerank(
        &self,
        Parameters(RunPageRankInput { label, top_n }): Parameters<RunPageRankInput>,
    ) -> Result<CallToolResult, McpError> {
        let top_n = top_n.unwrap_or(10).min(100) as usize;

        let scores = nopaldb::algorithms::PageRank::with_defaults()
            .compute(&*self.graph)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let mut ranked: Vec<(nopaldb::types::NodeId, f64)> = scores.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let label_filter = label.as_deref();
        let mut results = Vec::new();
        for (node_id, score) in ranked {
            if results.len() >= top_n {
                break;
            }
            let Ok(node) = self.graph.get_node(node_id).await else {
                continue;
            };
            if let Some(lbl) = label_filter
                && node.label != lbl
            {
                continue;
            }
            let name = node
                .properties
                .get("name")
                .and_then(|pv| {
                    if let nopaldb::types::PropertyValue::String(s) = pv {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| node_id.to_string());
            results.push(json!({
                "name":  name,
                "id":    node_id.to_string(),
                "label": node.label,
                "score": score,
            }));
        }

        Ok(CallToolResult::structured(json!({
            "top_n":   top_n,
            "label":   label,
            "results": results,
        })))
    }

    #[tool(
        description = "Find nodes semantically similar to a reference node using HNSW vector search. \
        Requires that node embeddings were previously stored with add_node_embedding. \
        'name' is the reference node name; 'model' is the embedding model label (e.g. 'minilm'). \
        Returns [{name, id, label, distance}] sorted by ascending cosine distance (0 = identical)."
    )]
    async fn similar_nodes(
        &self,
        Parameters(SimilarNodesInput { name, model, k }): Parameters<SimilarNodesInput>,
    ) -> Result<CallToolResult, McpError> {
        let k = k.unwrap_or(5).min(50) as usize;

        // Find reference node by name
        let ref_node = self
            .graph
            .get_node_by_property("name", &name)
            .await
            .map_err(|_| McpError::invalid_params(format!("Node '{}' not found", name), None))?;

        // Load its embedding
        let embedding = self
            .graph
            .get_node_embedding(ref_node.id, &model)
            .await
            .map_err(|_| {
                McpError::invalid_params(
                    format!("Node '{}' has no embedding for model '{}'", name, model),
                    None,
                )
            })?;

        // Build (or reuse cached) HNSW index and search
        let index = self
            .graph
            .get_or_build_embedding_index(&model)
            .await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let neighbors = index
            .search_knn(&embedding.vector, k)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let mut results = Vec::new();
        for (node_id, distance) in neighbors {
            let Ok(node) = self.graph.get_node(node_id).await else {
                continue;
            };
            let node_name = node
                .properties
                .get("name")
                .and_then(|pv| {
                    if let nopaldb::types::PropertyValue::String(s) = pv {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| node_id.to_string());
            results.push(json!({
                "name":     node_name,
                "id":       node_id.to_string(),
                "label":    node.label,
                "distance": distance,
            }));
        }

        Ok(CallToolResult::structured(json!({
            "reference": name,
            "model":     model,
            "k":         k,
            "results":   results,
        })))
    }

    // ── Ontology tools (require nopaldb built with `reasoner` feature) ──────

    #[tool(description = "Check whether a node is an instance of an OWL class — \
        including transitive membership via subclass hierarchy (CR1). \
        'name' is the node name; 'class' is the OWL class label (e.g. 'Disease'). \
        Returns is_instance: true/false with an explanation. \
        Requires OWL data loaded via import_turtle; returns false gracefully otherwise.")]
    async fn classify_node(
        &self,
        Parameters(ClassifyNodeInput { name, class }): Parameters<ClassifyNodeInput>,
    ) -> Result<CallToolResult, McpError> {
        let nql = format!(
            "find n.name as nome, n.label as lbl \
             from (n) \
             where instanceOf(n, \"{class}\") and n.name = \"{name}\" limit 1"
        );
        let result = self.graph.execute_nql(&nql).await;
        Ok(match result {
            Ok(r) => {
                let is_instance = !r.is_empty();
                CallToolResult::structured(json!({
                    "node":        name,
                    "class":       class,
                    "is_instance": is_instance,
                    "explanation": if is_instance {
                        format!("'{name}' IS an instance of '{class}' (transitively via subclass chain)")
                    } else {
                        format!("'{name}' is NOT an instance of '{class}'")
                    }
                }))
            }
            Err(e) => tool_error(e),
        })
    }

    #[tool(description = "List all individuals (instances) of an OWL class — \
        including those inherited transitively through the subclass hierarchy. \
        For example, list_instances('Disease') returns ViralInfections and \
        BacterialInfections as well, because they are subclasses of Disease. \
        'class' is the OWL class label. 'limit' caps results (default 20, max 200). \
        Requires OWL data loaded via import_turtle.")]
    async fn list_instances(
        &self,
        Parameters(ListInstancesInput { class, limit }): Parameters<ListInstancesInput>,
    ) -> Result<CallToolResult, McpError> {
        let lim = limit.unwrap_or(20).min(200);
        let nql = format!(
            "find n.name as name, n.label as label \
             from (n) \
             where instanceOf(n, \"{class}\") \
             order by n.label, n.name \
             limit {lim}"
        );
        let result = self.graph.execute_nql(&nql).await;
        Ok(match result {
            Ok(r) if r.is_empty() => CallToolResult::structured(json!({
                "class":     class,
                "instances": [],
                "note": format!(
                    "No instances of '{}' found. \
                     Verify the class name exists and OWL data has been imported.",
                    class
                )
            })),
            Ok(r) => {
                let instances: Vec<serde_json::Value> = r
                    .rows()
                    .iter()
                    .map(|row| {
                        json!({
                            "name":  row.get_string("name").unwrap_or_default(),
                            "label": row.get_string("label").unwrap_or_default(),
                        })
                    })
                    .collect();
                CallToolResult::structured(json!({
                    "class":     class,
                    "count":     instances.len(),
                    "instances": instances,
                }))
            }
            Err(e) => tool_error(e),
        })
    }

    #[tool(
        description = "List the subclasses of an OWL class — both direct (1-hop) \
        and indirect (transitive, up to 5 levels deep). \
        Example: list_subclasses('Disease') returns Infection (direct) and \
        ViralInfection, BacterialInfection (indirect via Infection). \
        'class' is the OWL class label. \
        Requires OWL data loaded via import_turtle."
    )]
    async fn list_subclasses(
        &self,
        Parameters(ListSubclassesInput { class }): Parameters<ListSubclassesInput>,
    ) -> Result<CallToolResult, McpError> {
        let direct_q = format!(
            "find sub.label as subclass \
             from (sub) -[:subClassOf]-> (parent) \
             where parent.label = \"{class}\" \
             order by subclass"
        );
        let transitive_q = format!(
            "find sub.label as subclass \
             from (sub) -[:subClassOf]->{{1,5}} (ancestor) \
             where ancestor.label = \"{class}\" \
             order by subclass"
        );

        let direct: Vec<String> = self
            .graph
            .execute_nql(&direct_q)
            .await
            .ok()
            .map(|r| {
                r.rows()
                    .iter()
                    .filter_map(|row| row.get_string("subclass"))
                    .collect()
            })
            .unwrap_or_default();

        let all_transitive: Vec<String> = self
            .graph
            .execute_nql(&transitive_q)
            .await
            .ok()
            .map(|r| {
                r.rows()
                    .iter()
                    .filter_map(|row| row.get_string("subclass"))
                    .collect()
            })
            .unwrap_or_default();

        let indirect: Vec<String> = all_transitive
            .iter()
            .filter(|s| !direct.contains(s))
            .cloned()
            .collect();

        if direct.is_empty() && all_transitive.is_empty() {
            return Ok(CallToolResult::structured(json!({
                "class":              class,
                "direct_subclasses":   [],
                "indirect_subclasses": [],
                "note": format!(
                    "No subclasses of '{}' found. \
                     Check the class name and that OWL data has been imported.",
                    class
                )
            })));
        }

        Ok(CallToolResult::structured(json!({
            "class":               class,
            "direct_subclasses":   direct,
            "indirect_subclasses": indirect,
            "total":               all_transitive.len(),
        })))
    }

    #[tool(
        description = "Return the graph schema grouped by node label, including counts. \
        More granular than schema_info: shows per-label node counts sorted by frequency. \
        Use this to understand the distribution of entity types before writing queries."
    )]
    async fn schema_by_kind(
        &self,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let q = "find n.label as label, count(*) as cnt \
                 from (n) \
                 group by n.label \
                 order by cnt desc";
        let result = self.graph.execute_nql(q).await;
        Ok(match result {
            Ok(r) => {
                let breakdown: Vec<serde_json::Value> = r
                    .rows()
                    .iter()
                    .map(|row| {
                        let label = row.get_string("label").unwrap_or_default();
                        let cnt = row.get_int("cnt").unwrap_or(0);
                        json!({ "label": label, "count": cnt })
                    })
                    .collect();
                CallToolResult::structured(json!({ "by_label": breakdown }))
            }
            Err(e) => tool_error(e),
        })
    }
}

// ─── ServerHandler ─────────────────────────────────────────────────────────

#[tool_handler(router = self.tool_router)]
impl ServerHandler for NopalMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::from_build_env())
        .with_instructions({
            let mut instructions = String::from(
                "NopalDB MCP Server: query a graph database using NQL. \
                 Start with 'schema_info' or 'schema_by_kind' to discover labels and edge types. \
                 Use 'graph_query' for arbitrary NQL, 'get_node'/'get_neighbors' for \
                 targeted lookup, 'find_path' for shortest paths, \
                 'run_pagerank' for influence ranking, 'similar_nodes' for \
                 semantic similarity search (requires pre-stored embeddings). ",
            );
            instructions.push_str(
                "For OWL ontologies: 'classify_node' checks class membership (transitive), \
                 'list_instances' enumerates all individuals of a class, \
                 'list_subclasses' maps the subclass hierarchy. \
                 Ontology tools require OWL data imported via import_turtle.",
            );
            instructions
        })
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        Ok(ListResourcesResult {
            resources: vec![
                RawResource::new("nopal://schema", "Graph Schema").no_annotation(),
                RawResource::new("nopal://stats", "Graph Statistics").no_annotation(),
            ],
            next_cursor: None,
            meta: None,
        })
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        match request.uri.as_str() {
            "nopal://schema" => {
                let labels_q = "find distinct n.label as label from (n) order by label";
                let etypes_q = "find distinct r.label as edge_type from (a:*) -[r:*]-> (b:*) order by edge_type";
                let labels = self
                    .graph
                    .execute_nql(labels_q)
                    .await
                    .map(|r| {
                        r.rows()
                            .iter()
                            .filter_map(|row| row.get_string("label"))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let edge_types = self
                    .graph
                    .execute_nql(etypes_q)
                    .await
                    .map(|r| {
                        r.rows()
                            .iter()
                            .filter_map(|row| row.get_string("edge_type"))
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let schema = json!({ "labels": labels, "edge_types": edge_types });
                Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    schema.to_string(),
                    request.uri,
                )]))
            }
            "nopal://stats" => {
                let nc = self
                    .graph
                    .execute_nql("find count(*) as total from (n)")
                    .await
                    .ok()
                    .and_then(|r| r.rows().first().and_then(|row| row.get_int("total")))
                    .unwrap_or(0);
                let ec = self
                    .graph
                    .execute_nql("find count(*) as total from (a:*) -[*]-> (b:*)")
                    .await
                    .ok()
                    .and_then(|r| r.rows().first().and_then(|row| row.get_int("total")))
                    .unwrap_or(0);
                let stats = json!({ "node_count": nc, "edge_count": ec });
                Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    stats.to_string(),
                    request.uri,
                )]))
            }
            _ => Err(McpError::resource_not_found(
                "Resource not found",
                Some(json!({ "uri": request.uri })),
            )),
        }
    }
}
