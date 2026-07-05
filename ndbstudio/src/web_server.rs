use std::sync::Arc;

use anyhow::{Context, Result};
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tokio::{net::TcpListener, sync::RwLock};

use crate::session::{SessionState, UiPreferences};
use crate::workbench::{
    FindingCreateRequest, FindingUpdateRequest, ProjectEntry, QueryRunRequest, TabCreateRequest,
    TabQueryRequest, TabRenameRequest, WorkbenchState,
};

const INDEX_HTML: &str = include_str!("../web/index.html");
const MAIN_JS: &str = include_str!("../web/main.js");
const API_JS: &str = include_str!("../web/api.js");
const UI_JS: &str = include_str!("../web/ui.js");
const EDITOR_JS: &str = include_str!("../web/editor.js");
const GRAPH_JS: &str = include_str!("../web/graph.js");
const STYLES_CSS: &str = include_str!("../web/styles.css");
const ICON_SVG: &str = include_str!("../web/icon.svg");

#[derive(Clone)]
struct WebState {
    workbench: Arc<RwLock<WorkbenchState>>,
}

type WebError = (StatusCode, String);

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    mode: &'static str,
    db_path: String,
}

#[derive(Debug, Deserialize)]
struct TimelineQuery {
    limit: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct TimelineImpactQuery {
    limit: Option<usize>,
    threshold: Option<u8>,
}

#[derive(Debug, Deserialize)]
struct GraphSubgraphQuery {
    focus_node_id: Option<String>,
    depth: Option<usize>,
    limit: Option<usize>,
    label: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct OpenSessionRequest {
    db_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CreateProjectRequest {
    name: String,
    db_path: Option<String>,
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UpdateProjectRequest {
    db_path: String,
    name: Option<String>,
    description: Option<String>,
    notes: Option<String>,
    tags: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct DeleteProjectRequest {
    db_path: String,
    #[serde(default)]
    delete_files: bool,
}

#[derive(Debug, Deserialize)]
struct PinProjectRequest {
    db_path: String,
}

#[derive(Debug, Deserialize)]
struct SaveQueryRequest {
    name: String,
    query: String,
}

#[derive(Debug, Serialize)]
struct ReferencePayload {
    sections: ReferenceSections,
}

#[derive(Debug, Serialize)]
struct ReferenceSections {
    nql_basics: ReferenceSection,
    algorithms: ReferenceSection,
    embeddings: ReferenceSection,
    examples: ReferenceSection,
    test_db: ReferenceSection,
}

#[derive(Debug, Serialize)]
struct ReferenceSection {
    title: &'static str,
    intro: &'static str,
    items: Vec<ReferenceItem>,
}

#[derive(Debug, Serialize)]
struct ReferenceItem {
    label: &'static str,
    description: &'static str,
    snippet: &'static str,
    kind: &'static str,
    runnable: bool,
}

#[derive(Debug, Serialize)]
struct OkResponse {
    ok: bool,
}

pub async fn serve(db_path: Option<String>, bind_addr: String) -> Result<()> {
    let workbench = WorkbenchState::open(db_path.as_deref()).await?;

    let state = WebState {
        workbench: Arc::new(RwLock::new(workbench)),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/app", get(index))
        .route("/assets/main.js", get(main_js))
        .route("/assets/api.js", get(api_js))
        .route("/assets/ui.js", get(ui_js))
        .route("/assets/editor.js", get(editor_js))
        .route("/assets/graph.js", get(graph_js))
        .route("/assets/styles.css", get(styles_css))
        .route("/assets/icon.svg", get(icon_svg))
        .route("/favicon.svg", get(icon_svg))
        .route("/api/health", get(health))
        .route("/api/session/open", post(open_session))
        .route("/api/session/state", get(session_state))
        .route("/api/tabs", post(create_tab))
        .route("/api/tabs/{tab_id}/activate", post(activate_tab))
        .route("/api/tabs/{tab_id}/rename", post(rename_tab))
        .route("/api/tabs/{tab_id}/query", post(update_tab_query))
        .route("/api/tabs/{tab_id}", axum::routing::delete(close_tab))
        .route("/api/schema", get(schema))
        .route("/api/graph/snapshot", get(graph_snapshot))
        .route("/api/graph/subgraph", get(graph_subgraph))
        .route("/api/timeline", get(timeline))
        .route("/api/timeline/dag/{recent_index}", get(timeline_dag))
        .route("/api/timeline/impact/{recent_index}", get(timeline_impact))
        .route("/api/timeline/pin/{recent_index}", post(toggle_timeline_pin))
        .route("/api/timeline/rerun/{recent_index}", post(rerun_timeline))
        .route("/api/query/run", post(run_query))
        .route("/api/projects", get(list_projects))
        .route("/api/projects/create", post(create_project))
        .route("/api/projects/close", post(close_project))
        .route("/api/projects/update", axum::routing::put(update_project))
        .route("/api/projects/remove", axum::routing::delete(remove_project))
        .route("/api/projects/pin", post(pin_project))
        .route("/api/session/ui-prefs", post(save_ui_prefs))
        .route("/api/reference", get(reference))
        .route("/api/queries/save", post(save_query))
        .route("/api/queries/{query_id}", axum::routing::delete(delete_query))
        .route("/api/findings", post(create_finding))
        .route(
            "/api/findings/{finding_id}",
            axum::routing::put(update_finding).delete(delete_finding),
        )
        .with_state(state);

    let listener = TcpListener::bind(&bind_addr)
        .await
        .with_context(|| format!("failed to bind NDBStudio Web server at {}", bind_addr))?;

    println!("NDBStudio Web listening on http://{}", bind_addr);
    match db_path.as_deref() {
        Some(path) if !path.is_empty() => println!("Requested project path: {}", path),
        _ => println!("Requested project path: <launcher mode>"),
    }

    axum::serve(listener, app)
        .await
        .context("NDBStudio Web server failed")?;

    Ok(())
}

async fn index() -> Response {
    (
        [
            ("content-type", "text/html; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        INDEX_HTML,
    )
        .into_response()
}

async fn main_js() -> Response {
    (
        [
            ("content-type", "application/javascript; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        MAIN_JS,
    )
        .into_response()
}

async fn api_js() -> Response {
    (
        [
            ("content-type", "application/javascript; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        API_JS,
    )
        .into_response()
}

async fn ui_js() -> Response {
    (
        [
            ("content-type", "application/javascript; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        UI_JS,
    )
        .into_response()
}

async fn editor_js() -> Response {
    (
        [
            ("content-type", "application/javascript; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        EDITOR_JS,
    )
        .into_response()
}

async fn graph_js() -> Response {
    (
        [
            ("content-type", "application/javascript; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        GRAPH_JS,
    )
        .into_response()
}

async fn styles_css() -> Response {
    (
        [
            ("content-type", "text/css; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        STYLES_CSS,
    )
        .into_response()
}

async fn icon_svg() -> Response {
    (
        [
            ("content-type", "image/svg+xml; charset=utf-8"),
            ("cache-control", "no-store"),
        ],
        ICON_SVG,
    )
        .into_response()
}

async fn health(State(state): State<WebState>) -> Json<HealthResponse> {
    let workbench = state.workbench.read().await;
    Json(HealthResponse {
        status: "ok",
        mode: "ndbstudio-web",
        db_path: workbench.db_path().to_string(),
    })
}

async fn open_session(
    State(state): State<WebState>,
    payload: Option<Json<OpenSessionRequest>>,
) -> Result<Json<crate::workbench::SessionOpenSnapshot>, WebError> {
    if let Some(Json(request)) = payload
        && let Some(db_path) = request.db_path.as_deref().map(str::trim)
        && !db_path.is_empty()
    {
        let mut workbench = state.workbench.write().await;
        workbench.open_db(db_path).await.map_err(internal_error)?;
        let snapshot = workbench
            .session_open_snapshot()
            .await
            .map_err(internal_error)?;
        return Ok(Json(snapshot));
    }

    let workbench = state.workbench.read().await;
    let snapshot = workbench
        .session_open_snapshot()
        .await
        .map_err(internal_error)?;
    Ok(Json(snapshot))
}

async fn session_state(
    State(state): State<WebState>,
) -> Result<Json<SessionState>, WebError> {
    let workbench = state.workbench.read().await;
    Ok(Json(workbench.session().clone()))
}

async fn create_tab(
    State(state): State<WebState>,
    Json(request): Json<TabCreateRequest>,
) -> Result<Json<SessionState>, WebError> {
    let mut workbench = state.workbench.write().await;
    let session = workbench
        .create_tab(request.title.as_deref())
        .map_err(internal_error)?;
    Ok(Json(session))
}

async fn activate_tab(
    State(state): State<WebState>,
    Path(tab_id): Path<String>,
) -> Result<Json<SessionState>, WebError> {
    let mut workbench = state.workbench.write().await;
    let session = workbench
        .activate_tab(&tab_id)
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, format!("tab not found: {}", tab_id)))?;
    Ok(Json(session))
}

async fn rename_tab(
    State(state): State<WebState>,
    Path(tab_id): Path<String>,
    Json(request): Json<TabRenameRequest>,
) -> Result<Json<SessionState>, WebError> {
    let mut workbench = state.workbench.write().await;
    let session = workbench
        .rename_tab(&tab_id, &request.title)
        .map_err(internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, format!("unable to rename tab: {}", tab_id)))?;
    Ok(Json(session))
}

async fn update_tab_query(
    State(state): State<WebState>,
    Path(tab_id): Path<String>,
    Json(request): Json<TabQueryRequest>,
) -> Result<Json<SessionState>, WebError> {
    let mut workbench = state.workbench.write().await;
    let session = workbench
        .update_tab_query(&tab_id, &request.query_text)
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, format!("tab not found: {}", tab_id)))?;
    Ok(Json(session))
}

async fn close_tab(
    State(state): State<WebState>,
    Path(tab_id): Path<String>,
) -> Result<Json<SessionState>, WebError> {
    let mut workbench = state.workbench.write().await;
    let session = workbench
        .close_tab(&tab_id)
        .map_err(internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, format!("unable to close tab: {}", tab_id)))?;
    Ok(Json(session))
}

async fn schema(
    State(state): State<WebState>,
) -> Result<Json<crate::workbench::SchemaSnapshot>, WebError> {
    let workbench = state.workbench.read().await;
    let snapshot = workbench.schema_snapshot().await.map_err(internal_error)?;
    Ok(Json(snapshot))
}

async fn graph_snapshot(
    State(state): State<WebState>,
) -> Result<Json<crate::workbench::GraphSnapshot>, WebError> {
    let workbench = state.workbench.read().await;
    let snapshot = workbench.graph_snapshot().await.map_err(internal_error)?;
    Ok(Json(snapshot))
}

async fn reference() -> Json<ReferencePayload> {
    Json(ReferencePayload {
        sections: ReferenceSections {
            nql_basics: ReferenceSection {
                title: "NQL Basics",
                intro: "Sintaxis mínima y modos de trabajo para moverte rápido dentro del workbench.",
                items: vec![
                    ReferenceItem {
                        label: "Estructura mínima",
                        description: "Usa `find ... from ... where ... limit ...` como base de exploración.",
                        snippet: "find n from (n) limit 25",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Propiedades",
                        description: "Proyecta propiedades para entender rápidamente el shape de un tipo activo en la base.",
                        snippet: "find f.name, f.faction from (f:Family) limit 25",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Relaciones",
                        description: "Trae nodos conectados para inspeccionar patrones reales del grafo demo.",
                        snippet: "find a, r, b from (a:Family)-[r:MARRIAGE]->(b:Family) limit 25",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Modos",
                        description: "Run ejecuta, Explain muestra plan, Profile ayuda a entender costo.",
                        snippet: "run | explain | profile",
                        kind: "note",
                        runnable: false,
                    },
                ],
            },
            algorithms: ReferenceSection {
                title: "Algorithms",
                intro: "Funciones integradas para exploración de influencia, estructura y comunidades.",
                items: vec![
                    ReferenceItem {
                        label: "PageRank",
                        description: "Mide influencia global o importancia relativa.",
                        snippet: "find pagerank(n) as rank from (n) limit 10",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Betweenness",
                        description: "Detecta nodos puente o intermediarios clave.",
                        snippet: "find betweenness(n) as bridge_score from (n) limit 10",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Degree",
                        description: "Cuenta conectividad local visible.",
                        snippet: "find degree(n) as deg from (n) limit 10",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Clustering",
                        description: "Sirve para ver cohesión local alrededor de un nodo.",
                        snippet: "find clustering(n) as coeff from (n) limit 10",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Community / community_fast",
                        description: "Agrupa nodos por comunidad (Louvain); `community_fast` es más ligera para exploración.",
                        snippet: "find community_fast(n) as cluster_fast from (n) limit 10",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Leiden — community detection con garantía CPM",
                        description: "Detecta comunidades garantizando que estén internamente bien-conectadas (Traag et al. 2019). Más estable que Louvain; resultado determinista. Usa caché separada de community().",
                        snippet: "find n.name, leiden(n) as grupo\nfrom (n)\norder by grupo\nlimit 20",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Leiden vs Community — coexistencia en la misma query",
                        description: "leiden() y community() usan cachés independientes y pueden combinarse en la misma query para comparar particiones.",
                        snippet: "find n.name,\n       community(n) as louvain,\n       leiden(n)   as leiden_comm\nfrom (n)\nlimit 10",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Shortest Path",
                        description: "Se usa como caso aparte desde API/algoritmos; no sigue el mismo patrón de agregación simple.",
                        snippet: "Ver docs/ALGORITHMS.md para el flujo exacto de shortest_path.",
                        kind: "note",
                        runnable: false,
                    },
                ],
            },
            embeddings: ReferenceSection {
                title: "Embeddings & Path Similarity",
                intro: "Funciones de búsqueda semántica sobre nodos, aristas y paths (requieren feature `embeddings`).",
                items: vec![
                    ReferenceItem {
                        label: "similar_to — búsqueda ANN",
                        description: "Filtra nodos similares a una referencia usando HNSW. Requiere embeddings de nodo y `embeddings-index`.",
                        snippet: "find n.name from (n:Company)\nwhere similar_to(n, \"Atlas Fiduciary Group\", \"minilm\")\nlimit 10",
                        kind: "query",
                        runnable: false,
                    },
                    ReferenceItem {
                        label: "embedding_similarity — coseno exacto vs referencia",
                        description: "Cosine similarity del nodo actual contra un nodo de referencia registrado.",
                        snippet: "find n.name,\n     embedding_similarity(n, \"ref-uuid\", \"minilm\") as score\nfrom (n:Company)\norder by score desc\nlimit 10",
                        kind: "query",
                        runnable: false,
                    },
                    ReferenceItem {
                        label: "path_embedding_similarity (E-8) — similitud de path",
                        description: "Cosine similarity del vector del path actual (E-7) contra una PathReferenceEmbedding persistida.",
                        snippet: "find a.id, b.id,\n     path_embedding_similarity(\"fraud_ring_v1\", \"node-m\", \"edge-m\") as score\nfrom (a:Account)-[:TX]->(b:Account)\nwhere path_embedding_similarity(\"fraud_ring_v1\", \"node-m\", \"edge-m\") > 0.82\norder by score desc",
                        kind: "query",
                        runnable: false,
                    },
                    ReferenceItem {
                        label: "path_knn_references (E-9/E-10) — top-k referencias",
                        description: "Devuelve las top-k PathReferenceEmbedding más similares al path actual. Retorna List<{name, score}>.",
                        snippet: "find a.id,\n     path_knn_references(\"node-m\", \"edge-m\", 5, 0.7) as refs\nfrom (a:Account)-[:TX]->(b:Account)",
                        kind: "query",
                        runnable: false,
                    },
                    ReferenceItem {
                        label: "path_anomaly_score (E-10) — detección de anomalías",
                        description: "Score 0=típico, 1=máxima anomalía. Calculado como 1-cosine(path_vec, centroide_de_referencias).",
                        snippet: "find a.id, b.id,\n     path_anomaly_score(\"node-m\", \"edge-m\") as anom\nfrom (a:Account)-[:TX]->(b:Account)\nwhere path_anomaly_score(\"node-m\", \"edge-m\") > 0.7\norder by anom desc",
                        kind: "query",
                        runnable: false,
                    },
                ],
            },
            examples: ReferenceSection {
                title: "Examples",
                intro: "Snippets cortos para producir hallazgos rápidos y comparables.",
                items: vec![
                    ReferenceItem {
                        label: "Explorar un tipo",
                        description: "Empieza por una muestra simple de un label relevante.",
                        snippet: "find f from (f:Family) limit 25",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Ver una relación",
                        description: "Inspecciona un patrón relacional completo.",
                        snippet: "find a, r, b from (a)-[r:MARRIAGE]->(b) limit 25",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Proyectar propiedades",
                        description: "Reduce ruido y enfócate en atributos legibles.",
                        snippet: "find f.name, f.faction from (f:Family) limit 25",
                        kind: "query",
                        runnable: true,
                    },
                    ReferenceItem {
                        label: "Explorar comunidades",
                        description: "Obtén una primera agrupación exploratoria por nodo.",
                        snippet: "find n.name, community_fast(n) as cluster from (n) limit 25",
                        kind: "query",
                        runnable: true,
                    },
                ],
            },
            test_db: ReferenceSection {
                title: "Test DB",
                intro: "Cómo generar rápidamente una base de prueba útil para NDBStudio Web.",
                items: vec![
                    ReferenceItem {
                        label: "Florentine Families",
                        description: "Dataset pequeño y útil para probar queries, graph y algoritmos.",
                        snippet: "python nopaldb/examples/florentine_families_dataset.py --output /tmp/florentine_families.db --force",
                        kind: "command",
                        runnable: false,
                    },
                    ReferenceItem {
                        label: "Abrir desde launcher",
                        description: "También puedes crear un proyecto nuevo vacío desde la UI y luego poblarlo por script o API.",
                        snippet: "Project Launcher -> Create project",
                        kind: "note",
                        runnable: false,
                    },
                ],
            },
        },
    })
}

async fn graph_subgraph(
    State(state): State<WebState>,
    Query(query): Query<GraphSubgraphQuery>,
) -> Result<Json<crate::workbench::GraphSubgraphResponse>, WebError> {
    let workbench = state.workbench.read().await;
    let snapshot = workbench
        .graph_subgraph(
            query.focus_node_id.as_deref(),
            query.depth.unwrap_or(1),
            query.limit.unwrap_or(50),
            query.label.as_deref(),
        )
        .await
        .map_err(internal_error)?;
    Ok(Json(snapshot))
}

async fn run_query(
    State(state): State<WebState>,
    Json(request): Json<QueryRunRequest>,
) -> Result<Json<crate::workbench::QueryRunResponse>, WebError> {
    let mut workbench = state.workbench.write().await;
    let response = workbench.run_query(request).await.map_err(internal_error)?;
    Ok(Json(response))
}

async fn timeline(
    State(state): State<WebState>,
    Query(query): Query<TimelineQuery>,
) -> Result<Json<crate::workbench::TimelineSnapshot>, WebError> {
    let workbench = state.workbench.read().await;
    Ok(Json(workbench.timeline_snapshot(query.limit.unwrap_or(100)).map_err(internal_error)?))
}

async fn timeline_dag(
    State(state): State<WebState>,
    Path(recent_index): Path<usize>,
    Query(query): Query<TimelineQuery>,
) -> Result<Json<crate::workbench::TimelineDagResponse>, WebError> {
    let workbench = state.workbench.read().await;
    let response = workbench
        .timeline_dag_for_recent(recent_index.saturating_sub(1), query.limit.unwrap_or(200))
        .ok_or((StatusCode::NOT_FOUND, format!("timeline entry not found: {}", recent_index)))?;
    Ok(Json(response))
}

async fn timeline_impact(
    State(state): State<WebState>,
    Path(recent_index): Path<usize>,
    Query(query): Query<TimelineImpactQuery>,
) -> Result<Json<crate::workbench::TimelineImpactResponse>, WebError> {
    let workbench = state.workbench.read().await;
    let response = workbench
        .timeline_impact_for_recent(
            recent_index.saturating_sub(1),
            query.limit.unwrap_or(200),
            query.threshold.unwrap_or(35).min(100),
        )
        .ok_or((StatusCode::NOT_FOUND, format!("timeline entry not found: {}", recent_index)))?;
    Ok(Json(response))
}

async fn toggle_timeline_pin(
    State(state): State<WebState>,
    Path(recent_index): Path<usize>,
) -> Result<Json<crate::workbench::TimelinePinResponse>, WebError> {
    let mut workbench = state.workbench.write().await;
    let response = workbench
        .toggle_timeline_pin_recent(recent_index.saturating_sub(1))
        .ok_or((StatusCode::NOT_FOUND, format!("timeline entry not found: {}", recent_index)))?;
    Ok(Json(response))
}

async fn rerun_timeline(
    State(state): State<WebState>,
    Path(recent_index): Path<usize>,
    Json(request): Json<crate::workbench::TimelineRerunRequest>,
) -> Result<Json<crate::workbench::QueryRunResponse>, WebError> {
    let mut workbench = state.workbench.write().await;
    let response = workbench
        .rerun_timeline_recent(recent_index.saturating_sub(1), request.run_mode)
        .await
        .map_err(internal_error)?;
    Ok(Json(response))
}

async fn list_projects(
    State(state): State<WebState>,
) -> Result<Json<Vec<ProjectEntry>>, WebError> {
    let workbench = state.workbench.read().await;
    Ok(Json(workbench.session_open_snapshot().await.map_err(internal_error)?.projects))
}

async fn create_project(
    State(state): State<WebState>,
    Json(request): Json<CreateProjectRequest>,
) -> Result<Json<crate::workbench::SessionOpenSnapshot>, WebError> {
    let name = request.name.trim().to_string();
    if name.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "project name is required".to_string()));
    }
    let mut workbench = state.workbench.write().await;
    workbench
        .create_project(&name, request.db_path.as_deref(), request.description.as_deref())
        .await
        .map_err(internal_error)?;
    let snapshot = workbench
        .session_open_snapshot()
        .await
        .map_err(internal_error)?;
    Ok(Json(snapshot))
}

async fn close_project(
    State(state): State<WebState>,
) -> Result<Json<crate::workbench::SessionOpenSnapshot>, WebError> {
    let mut workbench = state.workbench.write().await;
    workbench.close_project().map_err(internal_error)?;
    let snapshot = workbench
        .session_open_snapshot()
        .await
        .map_err(internal_error)?;
    Ok(Json(snapshot))
}

async fn update_project(
    State(state): State<WebState>,
    Json(request): Json<UpdateProjectRequest>,
) -> Result<Json<ProjectEntry>, WebError> {
    let mut workbench = state.workbench.write().await;
    let updated = workbench
        .update_project_metadata(
            &request.db_path,
            request.name.as_deref(),
            request.description.as_deref(),
            request.notes.as_deref(),
            request.tags,
        )
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, format!("project not found: {}", request.db_path)))?;
    Ok(Json(updated))
}

async fn remove_project(
    State(state): State<WebState>,
    Json(request): Json<DeleteProjectRequest>,
) -> Result<Json<OkResponse>, WebError> {
    let mut workbench = state.workbench.write().await;
    let removed = workbench
        .delete_project(&request.db_path, request.delete_files)
        .map_err(internal_error)?;
    if !removed {
        return Err((StatusCode::NOT_FOUND, format!("project not found: {}", request.db_path)));
    }
    Ok(Json(OkResponse { ok: true }))
}

async fn pin_project(
    State(state): State<WebState>,
    Json(request): Json<PinProjectRequest>,
) -> Result<Json<ProjectEntry>, WebError> {
    let mut workbench = state.workbench.write().await;
    let updated = workbench
        .toggle_project_pin(&request.db_path)
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, format!("project not found: {}", request.db_path)))?;
    Ok(Json(updated))
}

async fn save_ui_prefs(
    State(state): State<WebState>,
    Json(prefs): Json<UiPreferences>,
) -> Result<Json<OkResponse>, WebError> {
    let mut workbench = state.workbench.write().await;
    workbench
        .save_ui_preferences(prefs)
        .map_err(internal_error)?;
    Ok(Json(OkResponse { ok: true }))
}

async fn save_query(
    State(state): State<WebState>,
    Json(request): Json<SaveQueryRequest>,
) -> Result<Json<OkResponse>, WebError> {
    let mut workbench = state.workbench.write().await;
    let saved = workbench
        .save_query_to_session(&request.name, &request.query)
        .map_err(internal_error)?;
    if !saved {
        return Err((StatusCode::BAD_REQUEST, "query name and text are required".to_string()));
    }
    Ok(Json(OkResponse { ok: true }))
}

async fn delete_query(
    State(state): State<WebState>,
    Path(query_id): Path<String>,
) -> Result<Json<OkResponse>, WebError> {
    let mut workbench = state.workbench.write().await;
    let deleted = workbench
        .delete_saved_query(&query_id)
        .map_err(internal_error)?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, format!("saved query not found: {}", query_id)));
    }
    Ok(Json(OkResponse { ok: true }))
}

async fn create_finding(
    State(state): State<WebState>,
    Json(request): Json<FindingCreateRequest>,
) -> Result<Json<crate::session::FindingEntry>, WebError> {
    let mut workbench = state.workbench.write().await;
    let finding = workbench
        .create_finding(request)
        .map_err(internal_error)?
        .ok_or((StatusCode::BAD_REQUEST, "finding title or body is required".to_string()))?;
    Ok(Json(finding))
}

async fn update_finding(
    State(state): State<WebState>,
    Path(finding_id): Path<String>,
    Json(request): Json<FindingUpdateRequest>,
) -> Result<Json<crate::session::FindingEntry>, WebError> {
    let mut workbench = state.workbench.write().await;
    let finding = workbench
        .update_finding(&finding_id, request)
        .map_err(internal_error)?
        .ok_or((StatusCode::NOT_FOUND, format!("finding not found: {}", finding_id)))?;
    Ok(Json(finding))
}

async fn delete_finding(
    State(state): State<WebState>,
    Path(finding_id): Path<String>,
) -> Result<Json<OkResponse>, WebError> {
    let mut workbench = state.workbench.write().await;
    let deleted = workbench
        .delete_finding(&finding_id)
        .map_err(internal_error)?;
    if !deleted {
        return Err((StatusCode::NOT_FOUND, format!("finding not found: {}", finding_id)));
    }
    Ok(Json(OkResponse { ok: true }))
}

fn internal_error(err: anyhow::Error) -> WebError {
    eprintln!("ndbstudio-web error: {err:#}");
    let message = err.to_string();
    if message.contains("No project is open. Create or open a project first.") {
        (StatusCode::PRECONDITION_FAILED, message)
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}
