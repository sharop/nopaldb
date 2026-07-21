// NopalDB MCP Server — Phase G (stdio/sse, 15 tools + Defensive Architecture)
//
// Tools: graph_query, schema_info, get_node, get_neighbors, find_path,
//        run_pagerank, similar_nodes, schema_by_kind,
//        classify_node, list_instances, list_subclasses,
//        export_context_arrow, index_project_structure, record_episodic_event,
//        validate_and_commit_pr_context
// Resources: nopal://schema, nopal://stats
use std::sync::Arc;

use nopaldb::Graph;
use nopaldb::types::{Edge, NodeId};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars, tool, tool_handler, tool_router,
    service::RequestContext,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use crate::tools::{
    is_write_statement, json_to_pv, nql_result_to_tool, query_result_to_value, tool_error,
    readonly_error,
};

const MAX_ROWS: usize = 1000;
const DEFAULT_ROWS: u32 = 100;

/// Parse a link JSON object into a `LinkSpec` for the upsert tool.
fn parse_link_json(v: &serde_json::Value) -> Result<nopaldb::LinkSpec, String> {
    let obj = v.as_object().ok_or("each link must be a JSON object")?;
    let s = |name: &str| -> Result<String, String> {
        obj.get(name)
            .and_then(|x| x.as_str())
            .map(|x| x.to_string())
            .ok_or_else(|| format!("link missing string '{name}'"))
    };
    let target_key_value = obj
        .get("target_key_value")
        .ok_or("link missing 'target_key_value'")?;
    let props = obj
        .get("props")
        .and_then(|p| p.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), json_to_pv(v))).collect())
        .unwrap_or_default();
    Ok(nopaldb::LinkSpec {
        edge_type: s("type")?,
        target_label: s("target_label")?,
        target_key: s("target_key")?,
        target_key_value: json_to_pv(target_key_value),
        props,
        create_target_stub: obj.get("stub").and_then(|b| b.as_bool()).unwrap_or(false),
    })
}

// ─── Input types ───────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GraphQueryInput {
    /// NQL query to execute (FIND, ADD, UPDATE, DELETE, EXPLAIN, etc.)
    pub query: String,
    /// Maximum rows to return (default 100, max 1000)
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpsertNodeInput {
    /// Node label.
    pub label: String,
    /// Name of the identity property (must be present in `props`).
    pub key: String,
    /// Full desired property map as a JSON object (includes the key property).
    pub props: serde_json::Map<String, serde_json::Value>,
    /// Optional embedding vector (requires `model`).
    pub vector: Option<Vec<f32>>,
    /// Optional embedding model name (requires `vector`).
    pub model: Option<String>,
    /// Optional outgoing links to reconcile. Each: {type, target_label,
    /// target_key, target_key_value, props?, stub?}.
    pub links: Option<Vec<serde_json::Value>>,
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

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NqlSyntaxInput {
    /// Statement type to get syntax for: find, add, update, delete, explain, profile, sketch, commit, create_index, drop_index.
    /// Leave empty to get full synopsis.
    pub statement: Option<String>,
}

// ─── Dual-Plane & Semantic Commit Inputs ─────────────────────────────────

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ExportContextArrowInput {
    /// Consulta NQL para extraer el contexto masivo que se exportará (Zero-Copy).
    pub query: String,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")] // Serializa a "active" o "archived" en JSON/DB
pub enum EntityStatus {
    Active,
    Archived,
}

impl Default for EntityStatus {
    fn default() -> Self {
        EntityStatus::Active
    }
}

#[derive(Debug, Deserialize, Serialize, schemars::JsonSchema, Clone)]
pub struct ProjectEntity {
    /// El tipo de componente (ej. "File", "Class", "Function", "Module")
    pub kind: String,
    /// El nombre exacto extraído por el IDE
    pub name: String,
    /// La ruta relativa del archivo en el proyecto
    pub path: String,
    /// Jerarquía opcional (ej. si es un método, a qué clase pertenece)
    pub belongs_to: Option<String>,
    
    // Al usar #[serde(default)], si TypeScript no envía 'status', 
    // Rust invoca EntityStatus::default() y lo hace Active automáticamente.
    #[serde(default)] 
    pub status: EntityStatus, 
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct IndexProjectInput {
    /// Lista masiva de entidades para el Cold Start del grafo.
    pub entities: Vec<ProjectEntity>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RecordEpisodicEventInput {
    /// Archivo físico donde ocurrió el evento
    pub file_path: String,
    /// El "Por Qué" de la decisión (Rationale)
    pub rationale: String,
    /// Entidades lógicas extraídas vía JIT en el momento del commit
    pub affected_entities: Vec<ProjectEntity>,
    /// Reglas arquitectónicas aplicadas (ej. "SOLID", "Zero-Copy")
    pub applied_rules: Vec<String>,
    /// Hash del commit físico de Git (opcional)
    pub commit_hash: Option<String>,
    /// Timestamp exacto del momento del commit
    pub timestamp: Option<String>,
    /// Hash previo del HEAD antes de un `git commit --amend`.
    /// Si viene con valor, el servidor actualizará el evento existente
    /// en lugar de crear uno nuevo (Arquitectura Defensiva contra amends).
    pub previous_head_hash: Option<String>,
    /// Origen del evento: "developer_commit", "github_bootstrap", "ci_cd_pipeline", etc.
    pub source: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PrContextInput {
    /// Rationale consolidado de todos los mensajes arch: del PR
    pub consolidated_rationale: String,
    /// Hash definitivo del commit en main después del Squash & Merge
    pub final_main_commit_hash: String,
    /// Lista de archivos modificados en el PR
    pub changed_files: Vec<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RenameProjectEntityInput {
    pub old_path: String,
    pub new_path: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct LocalContextInput {
    pub filepath: String,
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

    /// Implementación Bulk de Causal Governance ("Find or Create")
    /// Bypass NQL para lograr resolución O(1) basada en el índice nativo de Storage,
    /// garantizando que los UUIDs se preservan para no romper las aristas AFFECTS.
    async fn upsert_entities_bulk(&self, entities: &[ProjectEntity]) -> Vec<Option<NodeId>> {
        let storage = self.graph.storage();
        let mut results = Vec::with_capacity(entities.len());
        let mut to_insert = Vec::new();

        for entity in entities {
            let path_val = nopaldb::types::PropertyValue::String(entity.path.clone());
            let candidates = storage.get_nodes_by_property("path", &path_val).await.unwrap_or_default();
            
            let mut found_id = None;
            for cid in candidates {
                if let Ok(node) = storage.get_node(cid).await {
                    if node.label == entity.kind {
                        if let Some(nopaldb::types::PropertyValue::String(n)) = node.properties.get("name") {
                            if n == &entity.name {
                                found_id = Some(cid);
                                break;
                            }
                        }
                    }
                }
            }

            if let Some(id) = found_id {
                results.push(Some(id));
            } else {
                let new_id = uuid::Uuid::new_v4();
                let mut node = nopaldb::types::Node::new(entity.kind.clone());
                node.id = new_id;
                node.properties.insert("name".to_string(), nopaldb::types::PropertyValue::String(entity.name.clone()));
                node.properties.insert("path".to_string(), path_val.clone());
                if let Some(ref parent) = entity.belongs_to {
                    node.properties.insert("belongs_to".to_string(), nopaldb::types::PropertyValue::String(parent.clone()));
                }
                
                let status_str = match entity.status {
                    EntityStatus::Active => "active",
                    EntityStatus::Archived => "archived",
                };
                node.properties.insert("status".to_string(), nopaldb::types::PropertyValue::String(status_str.to_string()));

                to_insert.push(node);
                results.push(Some(new_id));
            }
        }

        if !to_insert.is_empty() {
            // Bulk insert atómico en Sled
            let _ = storage.insert_nodes_batch(&to_insert).await;
            
            // Actualizar índices de propiedades
            for node in to_insert {
                for (prop, val) in &node.properties {
                    let _ = self.graph.storage_add_property_index(prop, val, node.id).await;
                }
            }
        }
        
        results
    }
}

#[tool_router(router = base_router)]
impl NopalMcpServer {
    #[tool(description = "Execute an NQL graph query. Supports FIND, EXPLAIN, ADD, UPDATE, DELETE \
        (write ops require --no-readonly flag). Returns rows as a JSON array. \
        Example: 'find p.name, p.age from (p:Person) order by p.age desc limit 10'")]
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
            Ok(nopaldb::NqlResult::Query(mut qr)) => {
                // ── Enriquecimiento Semántico: inyectar labels faltantes ──
                // Detectar prefijos de variables (ej. "a" de "a.id") y si
                // no existe "<prefix>.label" en las columnas, buscarlo en
                // la base de datos usando el ID del nodo.
                let prefixes: Vec<String> = qr.columns.iter()
                    .filter_map(|col| {
                        if col.ends_with(".id") {
                            Some(col.trim_end_matches(".id").to_string())
                        } else {
                            None
                        }
                    })
                    .collect();

                let missing_labels: Vec<String> = prefixes.iter()
                    .filter(|p| !qr.columns.contains(&format!("{}.label", p)))
                    .cloned()
                    .collect();

                if !missing_labels.is_empty() {
                    // Añadir columnas de label faltantes
                    for prefix in &missing_labels {
                        qr.columns.push(format!("{}.label", prefix));
                    }

                    // Enriquecer cada fila
                    for row in &mut qr.rows {
                        for prefix in &missing_labels {
                            let id_col = format!("{}.id", prefix);
                            let label_col = format!("{}.label", prefix);

                            let id_str = row.get(&id_col)
                                .and_then(|v| {
                                    if let nopaldb::PropertyValue::String(s) = v {
                                        Some(s.clone())
                                    } else {
                                        None
                                    }
                                });

                            if let Some(id_str) = id_str {
                                if let Ok(node_id) = id_str.parse::<NodeId>() {
                                    if let Ok(node) = self.graph.get_node(node_id).await {
                                        row.set(
                                            label_col,
                                            nopaldb::PropertyValue::String(node.label.clone()),
                                        );
                                    }
                                }
                            }
                        }
                    }
                }

                nql_result_to_tool(nopaldb::NqlResult::Query(qr), limit)
            }
            Ok(r) => nql_result_to_tool(r, limit),
            Err(e) => tool_error(e),
        })
    }

    #[tool(description = "Idempotently upsert a node keyed by (label, key): create if absent, \
        update if changed, no-op if identical. `props` is the full desired property map (a JSON \
        object including the key property). Optional `vector`+`model` attach an embedding; optional \
        `links` reconcile outgoing edges. Requires --no-readonly. Returns {outcome, node_id} where \
        outcome is created|updated|unchanged.")]
    async fn upsert_node(
        &self,
        Parameters(UpsertNodeInput { label, key, props, vector, model, links }): Parameters<
            UpsertNodeInput,
        >,
    ) -> Result<CallToolResult, McpError> {
        if self.readonly {
            return Ok(readonly_error());
        }
        // Build the request from JSON.
        let props: std::collections::HashMap<String, nopaldb::types::PropertyValue> = props
            .iter()
            .map(|(k, v)| (k.clone(), json_to_pv(v)))
            .collect();
        let embedding = match (vector, model) {
            (Some(v), Some(m)) => Some((v, m)),
            (None, None) => None,
            _ => return Ok(tool_error("provide both 'vector' and 'model', or neither")),
        };
        let mut link_specs = Vec::new();
        for l in links.unwrap_or_default() {
            match parse_link_json(&l) {
                Ok(spec) => link_specs.push(spec),
                Err(e) => return Ok(tool_error(e)),
            }
        }
        let req = nopaldb::UpsertRequest {
            label,
            key,
            props,
            embedding,
            links: link_specs,
        };
        match self.graph.upsert_node(req).await {
            Ok((outcome, id)) => Ok(CallToolResult::structured(json!({
                "outcome": outcome.as_str(),
                "node_id": id.to_string(),
            }))),
            Err(e) => Ok(tool_error(e)),
        }
    }

    #[tool(description = "Return the graph schema: labels, edge types, node count, edge count. \
        No arguments required.")]
    async fn schema_info(
        &self,
        _ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        // Labels
        let labels_q = "find distinct n.label as label from (n) order by label";
        let labels_r = self.graph.execute_nql(labels_q).await;

        // Edge types
        let etypes_q = "find distinct r.label as edge_type from (a) -[r]-> (b) order by edge_type";
        let etypes_r = self.graph.execute_nql(etypes_q).await;

        // Counts
        let nc_q = "find count(*) as total from (n)";
        let ec_q = "find count(*) as total from (a) -[]-> (b)";
        let nc_r = self.graph.execute_nql(nc_q).await;
        let ec_r = self.graph.execute_nql(ec_q).await;

        let labels: Vec<serde_json::Value> = labels_r.map(|r| {
            r.rows().iter()
                .filter_map(|row| row.get_string("label"))
                .map(serde_json::Value::String)
                .collect()
        }).unwrap_or_default();

        let edge_types: Vec<serde_json::Value> = etypes_r.map(|r| {
            r.rows().iter()
                .filter_map(|row| row.get_string("edge_type"))
                .map(serde_json::Value::String)
                .collect()
        }).unwrap_or_default();

        let node_count = nc_r.ok()
            .and_then(|r| r.rows().first().and_then(|row| row.get_int("total")))
            .unwrap_or(0);

        let edge_count = ec_r.ok()
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

    #[tool(description = "Return NQL (Nopal Query Language) syntax reference. \
        Call this BEFORE writing any NQL query to learn the correct syntax. \
        Optionally filter by statement type: find, add, update, delete, explain, profile, sketch, commit, create_index, drop_index. \
        Without a filter, returns a complete synopsis of all statement types with examples.")]
    async fn nql_syntax(
        &self,
        Parameters(NqlSyntaxInput { statement }): Parameters<NqlSyntaxInput>,
    ) -> Result<CallToolResult, McpError> {
        // Gramática PEG embebida en tiempo de compilación — siempre en sync con el parser real.
        const NQL_GRAMMAR: &str = include_str!("../../nopaldb/src/query/nql/parser/nql.pest");

        // Mapa de statement types → reglas raíz y ejemplos canónicos
        let sections: Vec<(&str, &str, Vec<&str>)> = vec![
            ("find", "query", vec![
                "find n.name, n.label from (n:Person) where n.age > 30 order by n.name limit 10",
                "find a.name, b.name from (a:Person)-[:KNOWS]->(b:Person)",
                "find count(*) as total from (n:File) group by n.label",
                "find distinct n.label as label from (n) order by label",
            ]),
            ("add", "add_stmt", vec![
                "add (n:Person { name: \"Alice\", age: 30 })",
                "add (a:Person { name: \"Alice\" })-[:KNOWS]->(b:Person { name: \"Bob\" })",
                "add (n:File { name: \"main.rs\", path: \"src/main.rs\" })",
            ]),
            ("update", "update_stmt", vec![
                "update (n:Person) set n.age = 31 where n.name = \"Alice\"",
                "update (n:File) set n.status = \"archived\" where n.path = \"old.rs\" limit 1",
            ]),
            ("delete", "delete_stmt", vec![
                "delete (n:TempNode)",
                "delete (n:Person) where n.name = \"test_user\"",
                "delete (a)-[r:OLD_LINK]->(b) limit 100",
            ]),
            ("explain", "explain_stmt", vec![
                "explain find n.name from (n:Person) where n.age > 25",
            ]),
            ("profile", "profile_stmt", vec![
                "profile find count(*) as total from (n)",
            ]),
            ("sketch", "sketch_stmt", vec![
                "sketch my_change = add (n:Feature { name: \"new_feature\" })",
                "sketch cleanup = delete (n:TempNode)",
            ]),
            ("commit", "commit_stmt", vec![
                "commit my_change",
            ]),
            ("create_index", "create_index_stmt", vec![
                "create index on Person(name)",
                "create index on File(path) type fulltext",
                "create index on Class(name) type btree",
            ]),
            ("drop_index", "drop_index_stmt", vec![
                "drop index Person_name",
            ]),
        ];

        let filter = statement.as_deref().map(|s| s.to_lowercase());

        let mut output = String::new();

        // Cabecera
        output.push_str("# NQL (Nopal Query Language) — Referencia de Sintaxis\n\n");
        output.push_str("⚠️ IMPORTANTE: NQL NO es Cypher ni SQL. Reglas clave:\n");
        output.push_str("  - Insertar nodos/aristas: `add`, NO `create`\n");
        output.push_str("  - Consultar: `find ... from (pattern)`\n");
        output.push_str("  - Modificar: `update (pattern) set prop = value where ...`\n");
        output.push_str("  - Eliminar: `delete (pattern) [where ...]`\n");
        output.push_str("  - Valores string: comillas dobles `\"texto\"` o simples `'texto'`\n");
        output.push_str("  - Nodos: `(variable:Label { prop: value })`\n");
        output.push_str("  - Aristas: `-[:TYPE]->`, `<-[:TYPE]-`, `-[r:TYPE]->`\n\n");

        for (name, rule_name, examples) in &sections {
            if let Some(ref f) = filter {
                if f != name { continue; }
            }

            output.push_str(&format!("## {}\n", name.to_uppercase()));

            // Extraer la regla formal de la gramática
            let rule_marker = format!("{} = {{", rule_name);
            if let Some(start) = NQL_GRAMMAR.find(&rule_marker) {
                // Capturar hasta el cierre de la regla (buscar la siguiente línea con `}`)
                let rule_slice = &NQL_GRAMMAR[start..];
                let mut depth = 0;
                let mut end_pos = 0;
                for (i, ch) in rule_slice.char_indices() {
                    if ch == '{' { depth += 1; }
                    if ch == '}' {
                        depth -= 1;
                        if depth == 0 { end_pos = i + 1; break; }
                    }
                }
                if end_pos > 0 {
                    output.push_str("Gramática formal:\n```pest\n");
                    output.push_str(&rule_slice[..end_pos]);
                    output.push_str("\n```\n");
                }
            }

            output.push_str("Ejemplos:\n```nql\n");
            for ex in examples {
                output.push_str(ex);
                output.push('\n');
            }
            output.push_str("```\n\n");
        }

        // Si se filtró y no hubo match, indicar
        if let Some(ref f) = filter {
            if !sections.iter().any(|(name, _, _)| *name == f.as_str()) {
                output.push_str(&format!(
                    "Statement type '{}' no reconocido. Tipos válidos: find, add, update, delete, \
                     explain, profile, sketch, commit, create_index, drop_index\n", f
                ));
            }
        }

        Ok(CallToolResult::structured(json!({
            "syntax_reference": output,
            "grammar_version": "NQL PEG v0.2",
            "total_grammar_rules": NQL_GRAMMAR.lines().filter(|l| l.contains(" = {") || l.contains(" = @{") || l.contains(" = _{")).count(),
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
            Some(lbl) => format!("n:{}", escape_nql_string(lbl)),
            None      => "n".to_string(),
        };
        let nql = if let Some(ref node_id) = id {
            format!(
                "find n.label as label, n.name as name, n.id as id \
                 from ({node_spec}) where n.id = \"{}\" limit 1",
                escape_nql_string(node_id)
            )
        } else {
            let node_name = name.as_deref().unwrap_or("");
            format!(
                "find n.label as label, n.name as name, n.id as id \
                 from ({node_spec}) where n.name = \"{}\" limit 1",
                escape_nql_string(node_name)
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
        Parameters(GetNeighborsInput { id, direction, edge_type, limit }): Parameters<GetNeighborsInput>,
    ) -> Result<CallToolResult, McpError> {
        let dir = direction.as_deref().unwrap_or("both");
        let lim = limit.unwrap_or(50).min(500);
        // [] = cualquier tipo de arista; [:TYPE] cuando se especifica tipo
        let edge_spec = match edge_type.as_deref() {
            Some(et) => format!(":{}", escape_nql_string(et)),
            None     => String::new(),
        };

        let safe_id = escape_nql_string(&id);
        let nql = match dir {
            "out" => format!(
                "find b.label as label, b.name as name, b.id as id \
                 from (a) -[{edge_spec}]-> (b) where a.name = \"{safe_id}\" or a.id = \"{safe_id}\" \
                 limit {lim}"
            ),
            "in" => format!(
                "find b.label as label, b.name as name, b.id as id \
                 from (b) -[{edge_spec}]-> (a) where a.name = \"{safe_id}\" or a.id = \"{safe_id}\" \
                 limit {lim}"
            ),
            _ => format!(
                "find b.label as label, b.name as name, b.id as id \
                 from (a) -[{edge_spec}]- (b) where a.name = \"{safe_id}\" or a.id = \"{safe_id}\" \
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
        Parameters(FindPathInput { from, to, max_depth }): Parameters<FindPathInput>,
    ) -> Result<CallToolResult, McpError> {
        let depth = max_depth.unwrap_or(6).min(10);
        let safe_from = escape_nql_string(&from);
        let safe_to = escape_nql_string(&to);
        let nql = format!(
            "find path.depth as depth, path.start as start_node, path.end as end_node \
             from (a) -[]->{{1,{depth}}} (b) \
             where a.name = \"{safe_from}\" and b.name = \"{safe_to}\" \
             order by depth asc limit 1"
        );
        let result = self.graph.execute_nql(&nql).await;
        Ok(match result {
            Ok(r) if r.is_empty() => {
                CallToolResult::structured(json!({
                    "found": false,
                    "message": format!("No path found between '{}' and '{}' within depth {}", from, to, depth),
                }))
            }
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

    #[tool(description = "Run PageRank on the graph and return the top-N highest-ranked nodes. \
        Optionally filter by node label. \
        Returns [{name, id, label, score}] sorted by score descending. \
        Useful for finding the most influential nodes in a network.")]
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
            let Ok(node) = self.graph.get_node(node_id).await else { continue };
            if let Some(lbl) = label_filter
                && node.label != lbl {
                continue;
            }
            let name = node.properties.get("name")
                .and_then(|pv| if let nopaldb::types::PropertyValue::String(s) = pv { Some(s.clone()) } else { None })
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

    #[tool(description = "Find nodes semantically similar to a reference node using HNSW vector search. \
        Requires that node embeddings were previously stored with add_node_embedding. \
        'name' is the reference node name; 'model' is the embedding model label (e.g. 'minilm'). \
        Returns [{name, id, label, distance}] sorted by ascending cosine distance (0 = identical).")]
    async fn similar_nodes(
        &self,
        Parameters(SimilarNodesInput { name, model, k }): Parameters<SimilarNodesInput>,
    ) -> Result<CallToolResult, McpError> {
        let k = k.unwrap_or(5).min(50) as usize;

        // Find reference node by name
        let ref_node = self.graph.get_node_by_property("name", &name).await
            .map_err(|_| McpError::invalid_params(
                format!("Node '{}' not found", name), None,
            ))?;

        // Load its embedding
        let embedding = self.graph.get_node_embedding(ref_node.id, &model).await
            .map_err(|_| McpError::invalid_params(
                format!("Node '{}' has no embedding for model '{}'", name, model), None,
            ))?;

        // Build (or reuse cached) HNSW index and search
        let index = self.graph.get_or_build_embedding_index(&model).await
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let neighbors = index.search_knn(&embedding.vector, k)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let mut results = Vec::new();
        for (node_id, distance) in neighbors {
            let Ok(node) = self.graph.get_node(node_id).await else { continue };
            let node_name = node.properties.get("name")
                .and_then(|pv| if let nopaldb::types::PropertyValue::String(s) = pv { Some(s.clone()) } else { None })
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
        let safe_name = escape_nql_string(&name);
        let safe_class = escape_nql_string(&class);
        let nql = format!(
            "find n.name as nome, n.label as lbl \
             from (n) \
             where instanceOf(n, \"{safe_class}\") and n.name = \"{safe_name}\" limit 1"
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
        let safe_class = escape_nql_string(&class);
        let nql = format!(
            "find n.name as name, n.label as label \
             from (n) \
             where instanceOf(n, \"{safe_class}\") \
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
                let instances: Vec<serde_json::Value> = r.rows().iter().map(|row| {
                    json!({
                        "name":  row.get_string("name").unwrap_or_default(),
                        "label": row.get_string("label").unwrap_or_default(),
                    })
                }).collect();
                CallToolResult::structured(json!({
                    "class":     class,
                    "count":     instances.len(),
                    "instances": instances,
                }))
            }
            Err(e) => tool_error(e),
        })
    }

    #[tool(description = "List the subclasses of an OWL class — both direct (1-hop) \
        and indirect (transitive, up to 5 levels deep). \
        Example: list_subclasses('Disease') returns Infection (direct) and \
        ViralInfection, BacterialInfection (indirect via Infection). \
        'class' is the OWL class label. \
        Requires OWL data loaded via import_turtle.")]
    async fn list_subclasses(
        &self,
        Parameters(ListSubclassesInput { class }): Parameters<ListSubclassesInput>,
    ) -> Result<CallToolResult, McpError> {
        let safe_class = escape_nql_string(&class);
        let direct_q = format!(
            "find sub.label as subclass \
             from (sub) -[:subClassOf]-> (parent) \
             where parent.label = \"{safe_class}\" \
             order by subclass"
        );
        let transitive_q = format!(
            "find sub.label as subclass \
             from (sub) -[:subClassOf]->{{1,5}} (ancestor) \
             where ancestor.label = \"{class}\" \
             order by subclass"
        );

        let direct: Vec<String> = self.graph.execute_nql(&direct_q).await.ok()
            .map(|r| r.rows().iter()
                .filter_map(|row| row.get_string("subclass"))
                .collect())
            .unwrap_or_default();

        let all_transitive: Vec<String> = self.graph.execute_nql(&transitive_q).await.ok()
            .map(|r| r.rows().iter()
                .filter_map(|row| row.get_string("subclass"))
                .collect())
            .unwrap_or_default();

        let indirect: Vec<String> = all_transitive.iter()
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

    #[tool(description = "Return the graph schema grouped by node label, including counts. \
        More granular than schema_info: shows per-label node counts sorted by frequency. \
        Use this to understand the distribution of entity types before writing queries.")]
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
                let breakdown: Vec<serde_json::Value> = r.rows().iter().map(|row| {
                    let label = row.get_string("label").unwrap_or_default();
                    let cnt   = row.get_int("cnt").unwrap_or(0);
                    json!({ "label": label, "count": cnt })
                }).collect();
                CallToolResult::structured(json!({ "by_label": breakdown }))
            }
            Err(e) => tool_error(e),
        })
    }

    // ─── Dual-Plane Tools: Data Plane & Governance ─────────────────────────

    #[tool(description = "Exporta un subgrafo masivo a memoria compartida (vía mmap). \
        Devuelve la ruta del archivo. Úsalo para recuperar redes de impacto gigante sin colapsar el IDE.")]
    async fn export_context_arrow(
        &self,
        Parameters(ExportContextArrowInput { query }): Parameters<ExportContextArrowInput>,
    ) -> Result<CallToolResult, McpError> {
        // Validar readonly: no permitir writes disfrazados de export
        if self.readonly && is_write_statement(&query) {
            return Ok(readonly_error());
        }

        let shm_path = crate::ipc_export::export_to_mmap(&self.graph, &query).await
            .map_err(|e| McpError::internal_error(format!("IPC Export Error: {}", e), None))?;

        Ok(CallToolResult::structured(json!({
            "status": "success",
            "data_plane": "mmap_json_buffer",
            "shm_path": shm_path,
        })))
    }

    #[tool(description = "Indexación inicial (Cold Start) del proyecto. \
        Crea la topología base (archivos, clases, funciones) en el grafo para permitir futuros commits semánticos.")]
    async fn index_project_structure(
        &self,
        Parameters(input): Parameters<IndexProjectInput>,
    ) -> Result<CallToolResult, McpError> {
        if self.readonly { return Ok(readonly_error()); }

        use std::collections::HashSet;
        
        // 1. Extraer el "Radar Vivo" (Las rutas que VS Code nos acaba de enviar)
        let incoming_paths: HashSet<String> = input.entities
            .iter()
            .map(|e| e.path.clone())
            .collect();

        // 2. Ingesta de Datos Vivos (Upsert) - Bulk Process
        let mut entities_indexed = 0;
        
        let upserted_ids = self.upsert_entities_bulk(&input.entities).await;
        
        for (i, entity) in input.entities.iter().enumerate() {
            let status_str = match entity.status {
                EntityStatus::Active => "active",
                EntityStatus::Archived => "archived",
            };
            
            if let Some(id) = upserted_ids[i] {
                // Actualizar status si cambió (lógica específica de index_project)
                if let Ok(mut node) = self.graph.get_node(id).await {
                    use nopaldb::types::PropertyValue;
                    let old_status = node.properties.get("status").cloned();
                    let new_status = PropertyValue::String(status_str.to_string());
                    
                    if old_status != Some(new_status.clone()) {
                        node.properties.insert("status".to_string(), new_status.clone());
                        let _ = self.graph.storage_insert_node(&node).await;
                        if let Some(old) = old_status {
                            let _ = self.graph.storage_remove_property_index("status", &old, id).await;
                        }
                        let _ = self.graph.storage_add_property_index("status", &new_status, id).await;
                    }
                }
                entities_indexed += 1;
            }
        }

        // 3. La Cacería de Zombis (El Diffing)
        // Pedimos a la DB TODOS los paths que actualmente cree que están vivos
        let active_q = "find n.id as id, n.path as path from (n) where n.label in [\"File\", \"Class\", \"Function\", \"Module\"] and n.status = \"active\"";
        let mut zombies_archived = 0;
        
        if let Ok(r) = self.graph.execute_nql(active_q).await {
            for row in r.rows() {
                if let (Some(path), Some(id)) = (row.get_string("path"), row.get_string("id")) {
                    // Si la base de datos tiene una ruta que VS Code ya NO vio hoy...
                    if !incoming_paths.contains(&path) {
                        // Aplicamos el Tombstone (Soft Delete)
                        if let Ok(uuid) = id.parse::<nopaldb::types::NodeId>() {
                            if let Ok(mut node) = self.graph.get_node(uuid).await {
                                use nopaldb::types::PropertyValue;
                                let old_status = node.properties.get("status").cloned();
                                let new_status = PropertyValue::String("archived".to_string());
                                
                                node.properties.insert("status".to_string(), new_status.clone());
                                if self.graph.storage_insert_node(&node).await.is_ok() {
                                    if let Some(old) = old_status {
                                        let _ = self.graph.storage_remove_property_index("status", &old, uuid).await;
                                    }
                                    let _ = self.graph.storage_add_property_index("status", &new_status, uuid).await;
                                    zombies_archived += 1;
                                }
                            }
                        }
                    }
                }
            }
        }

        Ok(CallToolResult::structured(json!({
            "status": "success",
            "entities_indexed": entities_indexed,
            "zombies_archived": zombies_archived,
            "note": "Topología base sincronizada (Tombstoning activado)."
        })))
    }

    #[tool(description = "Registra un Commit Semántico con Arquitectura Defensiva. \
        Si previous_head_hash está presente, actualiza el evento existente en lugar de duplicar \
        (tolerancia a git commit --amend). Realiza sincronización JIT de entidades.")]
    async fn record_episodic_event(
        &self,
        Parameters(input): Parameters<RecordEpisodicEventInput>,
    ) -> Result<CallToolResult, McpError> {
        if self.readonly { return Ok(readonly_error()); }

        let hash_val = input.commit_hash.unwrap_or_else(|| "-".to_string());
        let ts = input.timestamp.unwrap_or_else(|| "No registrado".to_string());

        // ── Arquitectura Defensiva: Detección de Amend ──────────────────────
        // Si el hook envía previous_head_hash, buscamos el evento viejo y lo
        // actualizamos in-place en lugar de crear un nodo nuevo.
        if let Some(ref prev_hash) = input.previous_head_hash {
            let find_q = format!(
                "find n.id as id from (n:EpisodicEvent) where n.commit_hash = \"{}\"",
                escape_nql_string(prev_hash)
            );
            if let Ok(res) = self.graph.execute_nql(&find_q).await {
                if let Some(row) = res.rows().first() {
                    if let Some(id_str) = row.get_string("id") {
                        if let Ok(existing_id) = id_str.parse::<NodeId>() {
                            if let Ok(mut node) = self.graph.get_node(existing_id).await {
                                use nopaldb::types::PropertyValue;
                                // Mutación in-place: actualizar hash y timestamp
                                node.properties.insert(
                                    "commit_hash".to_string(),
                                    PropertyValue::String(hash_val.clone()),
                                );
                                node.properties.insert(
                                    "timestamp".to_string(),
                                    PropertyValue::String(ts.clone()),
                                );
                                node.properties.insert(
                                    "rationale".to_string(),
                                    PropertyValue::String(input.rationale.clone()),
                                );

                                if self.graph.storage_insert_node(&node).await.is_ok() {
                                    return Ok(CallToolResult::structured(json!({
                                        "status": "amended",
                                        "event_id": id_str,
                                        "previous_hash": prev_hash,
                                        "new_hash": hash_val,
                                        "note": "Evento actualizado in-place (Arquitectura Defensiva: amend detectado)"
                                    })));
                                }
                            }
                        }
                    }
                }
            }
            // Si el hash previo no existía en la DB, procedemos con creación normal.
            // Esto es intencional: tolerancia ante hashes huérfanos o bases limpias.
        }

        // ── Flujo Normal: Crear nuevo EpisodicEvent ─────────────────────────
        let mut event_props = format!(
            "rationale: \"{}\", file: \"{}\"",
            escape_nql_string(&input.rationale), escape_nql_string(&input.file_path)
        );
        if !input.applied_rules.is_empty() {
            let rules_str = input.applied_rules.join(", ");
            event_props.push_str(&format!(", applied_rules: \"{}\"", escape_nql_string(&rules_str)));
        }
        event_props.push_str(&format!(", commit_hash: \"{}\"", escape_nql_string(&hash_val)));
        event_props.push_str(&format!(", timestamp: \"{}\"", escape_nql_string(&ts)));
        if let Some(ref source) = input.source {
            event_props.push_str(&format!(", source: \"{}\"", escape_nql_string(source)));
        }

        let event_nql = format!("add (ev:EpisodicEvent {{{}}})", event_props);
        let event_res = self.graph.execute_statement(&event_nql).await
            .map_err(|e| McpError::internal_error(format!("Fallo al crear evento: {}", e), None))?;

        let event_node_id: NodeId = match event_res {
            nopaldb::query::nql::NqlResult::Write(ref w) if !w.created_ids.is_empty() => {
                w.created_ids[0].parse().map_err(|_|
                    McpError::internal_error("UUID inválido en created_ids", None)
                )?
            }
            _ => return Err(McpError::internal_error("No se pudo obtener el ID del evento creado", None)),
        };
        let event_id_str = event_node_id.to_string();

        // ── Sincronización JIT y Enlace Causal ──────────────────────────────
        let mut links_created = 0;
        let upserted_ids = self.upsert_entities_bulk(&input.affected_entities).await;
        
        for (i, _) in input.affected_entities.iter().enumerate() {
            if let Some(target_id) = upserted_ids[i] {
                let edge = Edge::new(event_node_id, target_id, "AFFECTS");
                if self.graph.add_edge(edge).await.is_ok() {
                    links_created += 1;
                }
            }
        }

        Ok(CallToolResult::structured(json!({
            "status": "success",
            "event_id": event_id_str,
            "entities_linked": links_created,
        })))

    }

    #[tool(description = "Renombra la ruta física de una entidad estructural en el grafo \
        para preservar la continuidad del historial causal (Gobernanza Causal).")]
    async fn rename_project_entity(
        &self,
        Parameters(input): Parameters<RenameProjectEntityInput>,
    ) -> Result<CallToolResult, McpError> {
        if self.readonly { return Ok(readonly_error()); }

        let find_q = format!("find n.id as id from (n) where n.path = \"{}\"", input.old_path);
        
        let result = match self.graph.execute_nql(&find_q).await {
            Ok(r) => r,
            Err(e) => return Err(McpError::internal_error(format!("Error en consulta NQL: {}", e), None)),
        };

        if result.is_empty() {
            return Ok(CallToolResult::structured(json!({
                "status": "skipped",
                "message": format!("Nodo no encontrado, nada que actualizar para: {}", input.old_path)
            })));
        }

        use nopaldb::types::PropertyValue;
        let mut updated_count = 0;
        
        let old_path_val = PropertyValue::String(input.old_path.clone());
        let new_path_val = PropertyValue::String(input.new_path.clone());
        
        let new_file_name = std::path::Path::new(&input.new_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&input.new_path)
            .to_string();

        for row in result.rows() {
            let id_str = match row.get_string("id") {
                Some(s) => s,
                None => continue,
            };
            
            let id: nopaldb::types::NodeId = match id_str.parse() {
                Ok(uuid) => uuid,
                Err(_) => continue,
            };

            let mut node = match self.graph.get_node(id).await {
                Ok(n) => n,
                Err(_) => continue,
            };

            // Update path
            node.properties.insert("path".to_string(), new_path_val.clone());
            
            // Update name if it's a File
            let old_name_val = node.properties.get("name").cloned();
            let mut name_updated = false;
            let new_name_val = PropertyValue::String(new_file_name.clone());
            
            if node.label == "File" {
                node.properties.insert("name".to_string(), new_name_val.clone());
                name_updated = true;
            }

            if self.graph.storage_insert_node(&node).await.is_ok() {
                let _ = self.graph.storage_remove_property_index("path", &old_path_val, id).await;
                let _ = self.graph.storage_add_property_index("path", &new_path_val, id).await;
                
                if name_updated {
                    if let Some(old_name) = old_name_val {
                        let _ = self.graph.storage_remove_property_index("name", &old_name, id).await;
                    }
                    let _ = self.graph.storage_add_property_index("name", &new_name_val, id).await;
                }
                
                updated_count += 1;
            }
        }

        Ok(CallToolResult::structured(json!({
            "status": "success",
            "message": format!("{} nodos actualizados: {} -> {}", updated_count, input.old_path, input.new_path)
        })))
    }

    #[tool(description = "Obtiene el contexto causal (decisiones arquitectónicas) asociado \
        exclusivamente a un archivo físico o a una entidad estructural (función, clase, módulo) \
        por su nombre. Diseñado para inyección en el prompt de agentes autónomos.")]
    async fn get_local_causal_context(
        &self,
        Parameters(input): Parameters<LocalContextInput>,
    ) -> Result<CallToolResult, McpError> {
        // Stage 1: Fetch distinct entities with decisions in a completely static, injection-free query
        let q = "find distinct c.name as name, c.label as label, c.path as path from (e:EpisodicEvent)-[]->(c)";
        let result = self.graph.execute_nql(q).await
            .map_err(|e| McpError::internal_error(format!("Error en consulta NQL de entidades: {}", e), None))?;

        let input_norm = input.filepath.replace('\\', "/");
        let mut matched_paths = Vec::new();
        let mut matched_names = Vec::new();

        tracing::debug!(
            "get_local_causal_context: input_norm='{}', total_rows_fetched={}",
            input_norm,
            result.rows().len()
        );

        // Stage 2: Filter matched files and names in Rust
        for row in result.rows() {
            let path = row.get_string("path").unwrap_or_default();
            let name = row.get_string("name").unwrap_or_default();
            let label = row.get_string("label").unwrap_or_default();

            let is_match = if label == "File" {
                paths_match(&path, &input_norm)
            } else {
                // If it's a function/class, match if the name is exactly the input,
                // OR if the path containing it matches the input.
                name == input.filepath || (!path.is_empty() && paths_match(&path, &input_norm))
            };

            if is_match {
                if !path.is_empty() {
                    matched_paths.push(path);
                }
                if !name.is_empty() {
                    matched_names.push((name, label));
                }
            }
        }

        if matched_paths.is_empty() && matched_names.is_empty() {
            let output = format!(
                "[REGLAS ARQUITECTÓNICAS PARA: {}]\n- No hay decisiones arquitectónicas registradas para este archivo o entidad.",
                input.filepath
            );
            return Ok(CallToolResult::structured(json!({
                "context": output
            })));
        }

        // Stage 3: Fetch the final context using safe query populated only with matched strings
        let mut conditions = Vec::new();
        for p in &matched_paths {
            conditions.push(format!("c.path = \"{}\"", escape_nql_string(p)));
        }
        for (n, _) in &matched_names {
            conditions.push(format!("c.name = \"{}\"", escape_nql_string(n)));
        }

        let final_q = format!(
            "find e.rationale as rationale, e.timestamp as timestamp, e.commit_hash as commit_hash, c.name as name, c.label as label \
             from (e:EpisodicEvent)-[]->(c) \
             where {}",
            conditions.join(" or ")
        );

        let causal_result = self.graph.execute_nql(&final_q).await
            .map_err(|e| McpError::internal_error(format!("Error en consulta NQL de contexto: {}", e), None))?;

        if causal_result.is_empty() {
            let output = format!(
                "[REGLAS ARQUITECTÓNICAS PARA: {}]\n- No hay decisiones arquitectónicas registradas para este archivo o entidad.",
                input.filepath
            );
            return Ok(CallToolResult::structured(json!({
                "context": output
            })));
        }

        let mut output = format!("[REGLAS ARQUITECTÓNICAS PARA: {}]\n", input.filepath);
        for row in causal_result.rows() {
            let rationale = row.get_string("rationale").unwrap_or_else(|| "Sin justificación".to_string());
            let timestamp = row.get_string("timestamp").unwrap_or_else(|| "Desconocido".to_string());
            let commit_hash = row.get_string("commit_hash").unwrap_or_else(|| "-".to_string());
            let name = row.get_string("name").unwrap_or_else(|| "Desconocido".to_string());
            let label = row.get_string("label").unwrap_or_else(|| "Desconocido".to_string());

            output.push_str(&format!(
                "- Decisión: {}\n  Afecta a: [{}] {}\n  Fecha: {} | Commit: {}\n",
                rationale, label, name, timestamp, commit_hash
            ));
        }

        Ok(CallToolResult::structured(json!({
            "context": output
        })))
    }

    #[tool(description = "Validador CI/CD: Sella la Gobernanza Causal en la rama principal. \
        Diseñado para ejecutarse en el pipeline cuando un PR es aprobado y se hace Squash & Merge. \
        Ignora los hashes locales caóticos y acuña un único nodo inmutable con el hash definitivo de main.")]
    async fn validate_and_commit_pr_context(
        &self,
        Parameters(input): Parameters<PrContextInput>,
    ) -> Result<CallToolResult, McpError> {
        if self.readonly { return Ok(readonly_error()); }

        let safe_rationale = escape_nql_string(&input.consolidated_rationale);

        // 1. Crear nodo EpisodicEvent inmutable sellado con el hash de main
        let event_props = format!(
            "rationale: \"{}\", commit_hash: \"{}\", timestamp: \"{}\", \
             source: \"ci_cd_pipeline\", sealed: true",
            safe_rationale,
            escape_nql_string(&input.final_main_commit_hash),
            chrono_now_iso()
        );

        let event_nql = format!("add (ev:EpisodicEvent {{{}}})", event_props);
        let event_res = self.graph.execute_statement(&event_nql).await
            .map_err(|e| McpError::internal_error(
                format!("Fallo al crear evento CI/CD: {}", e), None
            ))?;

        let event_node_id: NodeId = match event_res {
            nopaldb::query::nql::NqlResult::Write(ref w) if !w.created_ids.is_empty() => {
                w.created_ids[0].parse().map_err(|_|
                    McpError::internal_error("UUID inválido en created_ids", None)
                )?
            }
            _ => return Err(McpError::internal_error(
                "No se pudo obtener el ID del evento CI/CD", None
            )),
        };
        let event_id_str = event_node_id.to_string();

        // 2. Enlazar a los archivos modificados (upsert de File nodes)
        let mut links_created = 0;
        for file_path in &input.changed_files {
            let file_name = std::path::Path::new(file_path)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(file_path);

            let find_q = format!(
                "find n.id as id from (n:File) where n.path = \"{}\"",
                escape_nql_string(file_path)
            );
            let file_node_id: Option<NodeId> = match self.graph.execute_nql(&find_q).await {
                Ok(r) if !r.is_empty() => {
                    r.rows()[0].get_string("id")
                        .and_then(|s| s.parse().ok())
                }
                _ => {
                    let add_q = format!(
                        "add (n:File {{name: \"{}\", path: \"{}\", status: \"active\"}})",
                        escape_nql_string(file_name), escape_nql_string(file_path)
                    );
                    match self.graph.execute_statement(&add_q).await {
                        Ok(nopaldb::query::nql::NqlResult::Write(ref w)) if !w.created_ids.is_empty() => {
                            w.created_ids[0].parse().ok()
                        }
                        _ => None,
                    }
                }
            };

            if let Some(target_id) = file_node_id {
                let edge = Edge::new(event_node_id, target_id, "AFFECTS");
                if self.graph.add_edge(edge).await.is_ok() {
                    links_created += 1;
                }
            }
        }

        Ok(CallToolResult::structured(json!({
            "status": "sealed",
            "event_id": event_id_str,
            "main_commit_hash": input.final_main_commit_hash,
            "files_linked": links_created,
            "note": "Gobernanza Causal sellada en main (fuente de verdad inmutable)"
        })))
    }
}

/// Helper: genera un timestamp ISO 8601 UTC sin depender de chrono.
fn chrono_now_iso() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = dur.as_secs();
    // Cálculo simplificado de fecha — suficiente para timestamps de auditoría
    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;
    // Epoch: 1970-01-01. Cálculo de año/mes/día via Rata Die.
    let (y, m, d) = days_to_ymd(days as i64);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z", y, m, d, hours, minutes, seconds)
}

fn days_to_ymd(days: i64) -> (i64, u32, u32) {
    // Algoritmo de conversión de días desde epoch a (año, mes, día)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
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
                 IMPORTANT: call 'nql_syntax' FIRST to learn correct NQL syntax before writing queries. \
                 NQL is NOT Cypher — it uses 'add' (not 'create'), 'find' (not 'match'). \
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
                 Ontology tools require OWL data imported via import_turtle. \
                 Dual-Plane tools: 'export_context_arrow' for massive zero-copy subgraph extraction, \
                 'index_project_structure' for Cold Start project topology indexing, \
                 'record_episodic_event' for Semantic Commits with JIT entity sync (Defensive Architecture: amend tolerance), \
                 'validate_and_commit_pr_context' for CI/CD pipeline PR governance sealing."
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
                let etypes_q = "find distinct r.label as edge_type from (a) -[r]-> (b) order by edge_type";
                let labels = self.graph.execute_nql(labels_q).await
                    .map(|r| r.rows().iter().filter_map(|row| row.get_string("label")).collect::<Vec<_>>())
                    .unwrap_or_default();
                let edge_types = self.graph.execute_nql(etypes_q).await
                    .map(|r| r.rows().iter().filter_map(|row| row.get_string("edge_type")).collect::<Vec<_>>())
                    .unwrap_or_default();
                let schema = json!({ "labels": labels, "edge_types": edge_types });
                Ok(ReadResourceResult::new(vec![ResourceContents::text(
                    schema.to_string(),
                    request.uri,
                )]))
            }
            "nopal://stats" => {
                let nc = self.graph.execute_nql("find count(*) as total from (n)").await
                    .ok().and_then(|r| r.rows().first().and_then(|row| row.get_int("total"))).unwrap_or(0);
                let ec = self.graph.execute_nql("find count(*) as total from (a) -[]-> (b)").await
                    .ok().and_then(|r| r.rows().first().and_then(|row| row.get_int("total"))).unwrap_or(0);
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

fn paths_match(db_path: &str, input_norm: &str) -> bool {
    let db_norm = db_path.replace('\\', "/");
    if db_norm.is_empty() || input_norm.is_empty() {
        return false;
    }
    
    if db_norm == input_norm {
        return true;
    }
    
    // Check if db_path ends with input_path (e.g. "nopaldb-mcp/src/server.rs" ends with "src/server.rs" or "/src/server.rs")
    let input_suffix = format!("/{}", input_norm);
    if db_norm.ends_with(&input_suffix) || db_norm.ends_with(input_norm) {
        return true;
    }
    
    // Check if input_path ends with db_path (e.g. "/absolute/path/nopaldb-mcp/src/server.rs" ends with "nopaldb-mcp/src/server.rs")
    let db_suffix = format!("/{}", db_norm);
    if input_norm.ends_with(&db_suffix) || input_norm.ends_with(&db_norm) {
        return true;
    }
    
    false
}

/// Escapa una cadena para interpolación segura en queries NQL.
/// Previene inyección NQL en queries construidas con format!().
fn escape_nql_string(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}
