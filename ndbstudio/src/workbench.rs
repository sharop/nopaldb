use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
#[cfg(feature = "web")]
use std::time::Instant;

use anyhow::{Context, Result};
use nopaldb::{parse_query, Graph, NqlResult, PropertyValue};
use nopaldb::query::nql::QueryResult;
use nopaldb::query::nql::parser::ast::{PatternElement, Projection};
use serde::{Deserialize, Serialize};

use crate::engine::mapper::to_tabular_result;
use crate::session::{ChangeKind, RunMode};
#[cfg(feature = "web")]
use crate::session::{
    default_session_path, load_session_state, save_session_state, CacheStatus, FindingDraft,
    FindingEntry, ImpactedRun, QueryGraphEdge, QueryGraphNode, QueryTabResultSnapshot,
    ResultGraphEdge, ResultGraphHint, ResultGraphMode, SavedQuery, SessionState, TimelineEntry,
    UiPreferences,
};
#[cfg(feature = "web")]
use std::collections::{BTreeSet, VecDeque};

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub db_path: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub pinned: bool,
    pub created_at: String,
    pub last_opened_at: String,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectRegistry {
    pub version: u32,
    pub projects: Vec<ProjectEntry>,
}

#[cfg(feature = "web")]
impl Default for ProjectRegistry {
    fn default() -> Self {
        Self {
            version: 1,
            projects: Vec::new(),
        }
    }
}

#[cfg(feature = "web")]
#[derive(Clone)]
pub struct WorkbenchState {
    db_path: String,
    graph: Option<Graph>,
    session: SessionState,
    session_restored: bool,
    registry: ProjectRegistry,
    pending_db_path: Option<String>,
}

#[cfg(feature = "web")]
impl WorkbenchState {
    pub async fn open(db_path: Option<&str>) -> Result<Self> {
        let registry = load_project_registry()?;
        let mut state = Self {
            db_path: String::new(),
            graph: None,
            session: SessionState::new(""),
            session_restored: false,
            registry,
            pending_db_path: None,
        };

        match db_path.map(str::trim).filter(|path| !path.is_empty()) {
            Some(path) if Path::new(path).exists() => state.open_db(path).await?,
            Some(path) => state.open_pending_path(path)?,
            None => {}
        }

        Ok(state)
    }

    pub fn db_path(&self) -> &str {
        &self.db_path
    }

    pub fn is_project_open(&self) -> bool {
        self.graph.is_some() && !self.db_path.is_empty()
    }

    #[allow(dead_code)]
    pub fn launcher_mode(&self) -> bool {
        !self.is_project_open()
    }

    #[allow(dead_code)]
    pub fn pending_db_path(&self) -> Option<&str> {
        self.pending_db_path.as_deref()
    }

    pub fn session(&self) -> &SessionState {
        &self.session
    }

    #[allow(dead_code)]
    pub fn registry(&self) -> &ProjectRegistry {
        &self.registry
    }

    fn open_pending_path(&mut self, db_path: &str) -> Result<()> {
        self.db_path.clear();
        self.graph = None;
        self.session = SessionState::new("");
        self.session_restored = false;
        self.pending_db_path = Some(db_path.to_string());
        save_project_registry(&self.registry)?;
        Ok(())
    }

    fn require_graph(&self) -> Result<&Graph> {
        self.graph
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No project is open. Create or open a project first."))
    }

    pub async fn open_db(&mut self, db_path: &str) -> Result<()> {
        let normalized = db_path.trim();
        if normalized.is_empty() {
            self.open_pending_path("")?;
            return Ok(());
        }

        if !Path::new(normalized).exists() {
            self.open_pending_path(normalized)?;
            return Ok(());
        }

        if self.db_path == normalized && self.graph.is_some() {
            touch_project(&mut self.registry, db_path);
            save_project_registry(&self.registry)?;
            let _ = update_recent_dbs(db_path);
            self.persist_session()?;
            return Ok(());
        }
        let graph = open_graph(normalized).await?;
        let (session, session_restored) = load_or_create_session(normalized)?;
        touch_project(&mut self.registry, normalized);
        save_project_registry(&self.registry)?;
        let _ = update_recent_dbs(normalized);

        self.db_path = normalized.to_string();
        self.graph = Some(graph);
        self.session = session;
        self.session_restored = session_restored;
        self.pending_db_path = None;
        self.persist_session()?;
        Ok(())
    }

    pub async fn create_project(
        &mut self,
        name: &str,
        db_path: Option<&str>,
        description: Option<&str>,
    ) -> Result<ProjectEntry> {
        let sanitized = sanitize_db_name(name);
        let resolved_path = match db_path {
            Some(p) if !p.trim().is_empty() => p.trim().to_string(),
            _ => {
                let databases_dir = ndbstudio_root_dir()?.join("databases");
                databases_dir
                    .join(format!("{}.db", sanitized))
                    .to_string_lossy()
                    .to_string()
            }
        };

        // Ensure parent directory exists before Graph::open
        if let Some(parent) = Path::new(&resolved_path).parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create database directory {}", parent.display()))?;
        }

        let graph = open_graph(&resolved_path).await?;
        let (session, session_restored) = load_or_create_session(&resolved_path)?;

        let now = chrono::Utc::now().to_rfc3339();
        let entry = ProjectEntry {
            db_path: resolved_path.clone(),
            name: name.trim().to_string(),
            description: description.unwrap_or("").to_string(),
            notes: String::new(),
            tags: Vec::new(),
            pinned: false,
            created_at: now.clone(),
            last_opened_at: now,
        };

        // Remove existing entry for same path, then insert at front
        self.registry.projects.retain(|p| p.db_path != resolved_path);
        self.registry.projects.insert(0, entry.clone());
        save_project_registry(&self.registry)?;
        let _ = update_recent_dbs(&resolved_path);

        self.db_path = resolved_path;
        self.graph = Some(graph);
        self.session = session;
        self.session_restored = session_restored;
        self.pending_db_path = None;
        self.persist_session()?;

        Ok(entry)
    }

    pub fn update_project_metadata(
        &mut self,
        db_path: &str,
        name: Option<&str>,
        description: Option<&str>,
        notes: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Result<Option<ProjectEntry>> {
        let Some(project) = self.registry.projects.iter_mut().find(|p| p.db_path == db_path) else {
            return Ok(None);
        };
        if let Some(n) = name {
            let trimmed = n.trim();
            if !trimmed.is_empty() {
                project.name = trimmed.to_string();
            }
        }
        if let Some(d) = description {
            project.description = d.to_string();
        }
        if let Some(n) = notes {
            project.notes = n.to_string();
        }
        if let Some(t) = tags {
            project.tags = t;
        }
        let updated = project.clone();
        save_project_registry(&self.registry)?;
        Ok(Some(updated))
    }

    pub fn toggle_project_pin(&mut self, db_path: &str) -> Result<Option<ProjectEntry>> {
        let Some(project) = self.registry.projects.iter_mut().find(|p| p.db_path == db_path) else {
            return Ok(None);
        };
        project.pinned = !project.pinned;
        let updated = project.clone();
        save_project_registry(&self.registry)?;
        Ok(Some(updated))
    }

    pub fn delete_project(&mut self, db_path: &str, delete_files: bool) -> Result<bool> {
        let before = self.registry.projects.len();
        self.registry.projects.retain(|p| p.db_path != db_path);
        if self.registry.projects.len() == before {
            return Ok(false);
        }
        save_project_registry(&self.registry)?;

        if delete_files {
            let path = Path::new(db_path);
            if path.is_dir() {
                std::fs::remove_dir_all(path)
                    .with_context(|| format!("failed to delete database directory {}", db_path))?;
            } else if path.exists() {
                std::fs::remove_file(path)
                    .with_context(|| format!("failed to delete database file {}", db_path))?;
            }
        }

        if self.db_path == db_path {
            self.db_path.clear();
            self.graph = None;
            self.session = SessionState::new("");
            self.session_restored = false;
            self.pending_db_path = None;
        }

        Ok(true)
    }

    pub fn close_project(&mut self) -> Result<()> {
        self.db_path.clear();
        self.graph = None;
        self.session = SessionState::new("");
        self.session_restored = false;
        self.pending_db_path = None;
        Ok(())
    }

    pub fn save_ui_preferences(&mut self, prefs: UiPreferences) -> Result<()> {
        self.session.ui_preferences = Some(prefs);
        if self.is_project_open() {
            self.persist_session()
        } else {
            Ok(())
        }
    }

    pub fn save_query_to_session(&mut self, name: &str, query: &str) -> Result<bool> {
        let saved = self.session.save_query(name, query);
        if saved {
            self.persist_session()?;
        }
        Ok(saved)
    }

    pub fn delete_saved_query(&mut self, query_id: &str) -> Result<bool> {
        let deleted = self.session.delete_saved_query(query_id);
        if deleted {
            self.persist_session()?;
        }
        Ok(deleted)
    }

    pub fn create_finding(&mut self, request: FindingCreateRequest) -> Result<Option<FindingEntry>> {
        let finding = self.session.add_finding(FindingDraft {
            title: request.title,
            body: request.body,
            tab_id: request.tab_id,
            run_id: request.run_id,
            query_text: request.query_text,
            summary: request.summary,
            row_index: request.row_index,
            graph_focus_node_id: request.graph_focus_node_id,
        });
        if finding.is_some() {
            self.persist_session()?;
        }
        Ok(finding)
    }

    pub fn update_finding(
        &mut self,
        finding_id: &str,
        request: FindingUpdateRequest,
    ) -> Result<Option<FindingEntry>> {
        let finding = self
            .session
            .update_finding(finding_id, request.title.as_deref(), request.body.as_deref());
        if finding.is_some() {
            self.persist_session()?;
        }
        Ok(finding)
    }

    pub fn delete_finding(&mut self, finding_id: &str) -> Result<bool> {
        let deleted = self.session.delete_finding(finding_id);
        if deleted {
            self.persist_session()?;
        }
        Ok(deleted)
    }

    pub fn create_tab(&mut self, title: Option<&str>) -> Result<SessionState> {
        self.session.create_tab(title);
        self.persist_session()?;
        Ok(self.session.clone())
    }

    pub fn activate_tab(&mut self, tab_id: &str) -> Result<Option<SessionState>> {
        if !self.session.activate_tab_by_id(tab_id) {
            return Ok(None);
        }
        self.persist_session()?;
        Ok(Some(self.session.clone()))
    }

    pub fn rename_tab(&mut self, tab_id: &str, title: &str) -> Result<Option<SessionState>> {
        if !self.session.rename_tab(tab_id, title) {
            return Ok(None);
        }
        self.persist_session()?;
        Ok(Some(self.session.clone()))
    }

    pub fn close_tab(&mut self, tab_id: &str) -> Result<Option<SessionState>> {
        if !self.session.close_tab_by_id(tab_id) {
            return Ok(None);
        }
        self.persist_session()?;
        Ok(Some(self.session.clone()))
    }

    pub fn update_tab_query(&mut self, tab_id: &str, query_text: &str) -> Result<Option<SessionState>> {
        if !self.session.set_query_text_for_tab(tab_id, query_text) {
            return Ok(None);
        }
        self.persist_session()?;
        Ok(Some(self.session.clone()))
    }

    pub async fn session_open_snapshot(&self) -> Result<SessionOpenSnapshot> {
        build_session_open_snapshot(
            self.graph.as_ref(),
            &self.session,
            &self.registry,
            self.session_restored,
            self.pending_db_path.as_deref(),
        )
        .await
    }

    pub async fn schema_snapshot(&self) -> Result<SchemaSnapshot> {
        let graph = self.require_graph()?;
        build_schema_snapshot(graph, &self.db_path).await
    }

    pub async fn graph_snapshot(&self) -> Result<GraphSnapshot> {
        let graph = self.require_graph()?;
        build_graph_snapshot(graph).await
    }

    pub async fn graph_subgraph(
        &self,
        focus_node_id: Option<&str>,
        depth: usize,
        limit: usize,
        label: Option<&str>,
    ) -> Result<GraphSubgraphResponse> {
        let graph = self.require_graph()?;
        build_graph_subgraph(graph, focus_node_id, depth, limit, label).await
    }

    pub fn timeline_snapshot(&self, limit: usize) -> Result<TimelineSnapshot> {
        if !self.is_project_open() {
            return Err(anyhow::anyhow!(
                "No project is open. Create or open a project first."
            ));
        }
        Ok(TimelineSnapshot {
            entries: self
                .session
                .recent_timeline(limit)
                .into_iter()
                .cloned()
                .collect(),
            active_tab_id: self.session.active_tab_id.clone(),
            graph_nodes: self.session.query_graph.nodes.len(),
            graph_edges: self.session.query_graph.edges.len(),
        })
    }

    pub fn timeline_dag_for_recent(
        &self,
        recent_index: usize,
        limit: usize,
    ) -> Option<TimelineDagResponse> {
        if !self.is_project_open() {
            return None;
        }
        let recent = self.session.recent_timeline(limit);
        let target = recent.get(recent_index)?;
        let run_id = target.id.clone();

        let mut node_ids = std::collections::BTreeSet::new();
        let _ = node_ids.insert(run_id.clone());
        let mut edges = Vec::new();

        for edge in &self.session.query_graph.edges {
            if edge.from_run_id == run_id || edge.to_run_id == run_id {
                let _ = node_ids.insert(edge.from_run_id.clone());
                let _ = node_ids.insert(edge.to_run_id.clone());
                edges.push(edge.clone());
            }
        }

        let nodes = self
            .session
            .query_graph
            .nodes
            .iter()
            .filter(|node| node_ids.contains(&node.run_id))
            .cloned()
            .collect();

        Some(TimelineDagResponse {
            target_run_id: run_id,
            summary: self.session.lineage_summary_for_recent(recent_index, limit),
            nodes,
            edges,
        })
    }

    pub fn timeline_impact_for_recent(
        &self,
        recent_index: usize,
        limit: usize,
        threshold: u8,
    ) -> Option<TimelineImpactResponse> {
        if !self.is_project_open() {
            return None;
        }
        let recent = self.session.recent_timeline(limit);
        let target = recent.get(recent_index)?;
        let impacted = self
            .session
            .impacted_dependent_queries_scored_for_recent(recent_index, limit)
            .into_iter()
            .filter(|run| run.impact_score >= threshold)
            .collect();

        Some(TimelineImpactResponse {
            target_run_id: target.id.clone(),
            threshold,
            impacted,
        })
    }

    pub fn toggle_timeline_pin_recent(
        &mut self,
        recent_index: usize,
    ) -> Option<TimelinePinResponse> {
        if !self.session.toggle_recent_timeline_pin(recent_index) {
            return None;
        }
        let _ = self.persist_session();
        let pinned = self
            .session
            .recent_timeline(1000)
            .get(recent_index)
            .map(|entry| entry.pinned)?;
        Some(TimelinePinResponse {
            recent_index: recent_index + 1,
            pinned,
        })
    }

    pub async fn rerun_timeline_recent(
        &mut self,
        recent_index: usize,
        run_mode: Option<RunMode>,
    ) -> Result<QueryRunResponse> {
        let (query, mode) = {
            let recent = self.session.recent_timeline(1000);
            let entry = recent.get(recent_index).ok_or_else(|| {
                anyhow::anyhow!("timeline entry {} not found", recent_index + 1)
            })?;
            (entry.query.clone(), run_mode.unwrap_or(entry.run_mode))
        };
        self.run_query(QueryRunRequest { query, run_mode: mode }).await
    }

    pub async fn run_query(&mut self, request: QueryRunRequest) -> Result<QueryRunResponse> {
        let graph = self.require_graph()?.clone();
        self.session.set_active_query_text(&request.query);
        let started_at = Instant::now();
        let query = request.query.clone();

        match execute_query(&graph, &request).await {
            Ok(result) => {
                let row_count = result.row_count();
                self.session.record_success(
                    result.run_mode,
                    Some(CacheStatus::Miss),
                    map_invalidation_to_change_kind(result.invalidation),
                    result.touched_labels.clone(),
                    result.touched_edge_types.clone(),
                    result.touched_properties.clone(),
                    &query,
                    &result.summary,
                    result.rows.len(),
                    started_at.elapsed().as_secs_f64() * 1000.0,
                );
                let _ = self.session.set_active_tab_result(QueryTabResultSnapshot {
                    headers: result.headers.clone(),
                    rows: result.rows.clone(),
                    summary: result.summary.clone(),
                    row_count,
                    duration_ms: Some(started_at.elapsed().as_secs_f64() * 1000.0),
                    run_mode: result.run_mode,
                    error: None,
                    graph_hint: result.graph_hint.clone(),
                    row_graph_hints: result.row_graph_hints.clone(),
                });
                self.persist_session()?;

                Ok(QueryRunResponse {
                    run_mode: result.run_mode,
                    summary: result.summary,
                    headers: result.headers,
                    rows: result.rows,
                    row_graph_hints: result.row_graph_hints,
                    row_count,
                    duration_ms: started_at.elapsed().as_secs_f64() * 1000.0,
                    change_kind: map_invalidation_to_change_kind(result.invalidation),
                    touched_labels: result.touched_labels,
                    touched_edge_types: result.touched_edge_types,
                    touched_properties: result.touched_properties,
                    graph_hint: result.graph_hint,
                })
            }
            Err(err) => {
                let touched = extract_query_entities(&query);
                let duration_ms = started_at.elapsed().as_secs_f64() * 1000.0;
                let formatted_error = format_error_chain(&err);
                self.session.record_failure(
                    request.run_mode,
                    Some(CacheStatus::Miss),
                    classify_change_kind_from_query(&query),
                    touched.labels,
                    touched.edge_types,
                    touched.properties,
                    &query,
                    &formatted_error,
                    Some(duration_ms),
                );
                let _ = self.session.set_active_tab_result(QueryTabResultSnapshot {
                    headers: vec!["error".to_string()],
                    rows: vec![vec![formatted_error.clone()]],
                    summary: "Query failed".to_string(),
                    row_count: 1,
                    duration_ms: Some(duration_ms),
                    run_mode: request.run_mode,
                    error: Some(formatted_error),
                    graph_hint: None,
                    row_graph_hints: vec![None],
                });
                let _ = self.persist_session();
                Err(err)
            }
        }
    }

    pub fn persist_session(&self) -> Result<()> {
        if !self.is_project_open() {
            return Ok(());
        }
        let session_path = session_path_for_db(&self.db_path)?;
        save_session_state(&session_path, &self.session)
    }
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize)]
pub struct SessionOpenSnapshot {
    pub session_id: String,
    pub db_path: String,
    pub total_nodes: usize,
    pub total_edges: usize,
    pub opened_at: String,
    pub active_tab_id: String,
    pub tabs: usize,
    pub timeline_count: usize,
    pub projects: Vec<ProjectEntry>,
    pub ui_preferences: Option<UiPreferences>,
    pub saved_queries: Vec<SavedQuery>,
    pub session_restored: bool,
    pub project_open: bool,
    pub launcher_mode: bool,
    pub pending_db_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemaSnapshot {
    pub db_path: String,
    pub db_name: String,
    pub total_nodes: usize,
    pub total_edges: usize,
    pub avg_degree: f64,
    pub density: f64,
    pub node_types: Vec<SchemaTypeSnapshot>,
    pub edge_types: Vec<SchemaTypeSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SchemaTypeSnapshot {
    pub name: String,
    pub count: usize,
    pub properties: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphSnapshot {
    pub nodes: Vec<GraphNodeSnapshot>,
    pub edges: Vec<GraphEdgeSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphNodeSnapshot {
    pub id: String,
    pub label: String,
    pub display_label: String,
    pub entity_type: String,
    pub properties: Vec<GraphNodePropertySnapshot>,
    pub degree: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphEdgeSnapshot {
    pub source: String,
    pub target: String,
    pub edge_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct GraphNodePropertySnapshot {
    pub key: String,
    pub value: String,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize)]
pub struct GraphSubgraphResponse {
    pub focus_node_id: Option<String>,
    pub depth: usize,
    pub truncated: bool,
    pub nodes: Vec<GraphNodeSnapshot>,
    pub edges: Vec<GraphEdgeSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryRunRequest {
    pub query: String,
    #[serde(default = "default_run_mode")]
    pub run_mode: RunMode,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize)]
pub struct QueryRunResponse {
    pub run_mode: RunMode,
    pub summary: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub row_graph_hints: Vec<Option<ResultGraphHint>>,
    pub row_count: usize,
    pub duration_ms: f64,
    pub change_kind: ChangeKind,
    pub touched_labels: Vec<String>,
    pub touched_edge_types: Vec<String>,
    pub touched_properties: Vec<String>,
    pub graph_hint: Option<ResultGraphHint>,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize)]
pub struct TimelineSnapshot {
    pub entries: Vec<TimelineEntry>,
    pub active_tab_id: String,
    pub graph_nodes: usize,
    pub graph_edges: usize,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize)]
pub struct TimelineDagResponse {
    pub target_run_id: String,
    pub summary: Option<String>,
    pub nodes: Vec<QueryGraphNode>,
    pub edges: Vec<QueryGraphEdge>,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize)]
pub struct TimelineImpactResponse {
    pub target_run_id: String,
    pub threshold: u8,
    pub impacted: Vec<ImpactedRun>,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TimelineRerunRequest {
    pub run_mode: Option<RunMode>,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize)]
pub struct TimelinePinResponse {
    pub recent_index: usize,
    pub pinned: bool,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingCreateRequest {
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub tab_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub query_text: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub row_index: Option<usize>,
    #[serde(default)]
    pub graph_focus_node_id: Option<String>,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingUpdateRequest {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TabCreateRequest {
    pub title: Option<String>,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabRenameRequest {
    pub title: String,
}

#[cfg(feature = "web")]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabQueryRequest {
    pub query_text: String,
}

#[derive(Debug, Clone)]
pub struct QueryExecutionResult {
    pub run_mode: RunMode,
    pub summary: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub row_graph_hints: Vec<Option<ResultGraphHint>>,
    pub invalidation: QueryInvalidation,
    pub touched_labels: Vec<String>,
    pub touched_edge_types: Vec<String>,
    pub touched_properties: Vec<String>,
    pub graph_hint: Option<ResultGraphHint>,
}

impl QueryExecutionResult {
    #[cfg(feature = "web")]
    pub fn row_count(&self) -> usize {
        self.rows.len()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryInvalidation {
    None,
    Data,
    Schema,
}

pub async fn open_graph(db_path: &str) -> Result<Graph> {
    Graph::open(db_path)
        .await
        .with_context(|| format!("failed to open database at {}", db_path))
}

#[cfg(feature = "web")]
pub async fn build_session_open_snapshot(
    graph: Option<&Graph>,
    session: &SessionState,
    registry: &ProjectRegistry,
    session_restored: bool,
    pending_db_path: Option<&str>,
) -> Result<SessionOpenSnapshot> {
    let (total_nodes, total_edges) = if let Some(graph) = graph {
        let stats = graph.get_stats().await?;
        (stats.total_nodes, stats.total_edges)
    } else {
        (0, 0)
    };
    Ok(SessionOpenSnapshot {
        session_id: session.session_id.clone(),
        db_path: session.db_path.clone(),
        total_nodes,
        total_edges,
        opened_at: session.opened_at.clone(),
        active_tab_id: session.active_tab_id.clone(),
        tabs: session.tabs.len(),
        timeline_count: session.timeline.len(),
        projects: sorted_projects(&registry.projects),
        ui_preferences: session.ui_preferences.clone(),
        saved_queries: session.saved_queries.clone(),
        session_restored,
        project_open: graph.is_some() && !session.db_path.trim().is_empty(),
        launcher_mode: graph.is_none(),
        pending_db_path: pending_db_path.map(str::to_string),
    })
}

/// Sort projects: pinned first, then by last_opened_at descending
#[cfg(feature = "web")]
fn sorted_projects(projects: &[ProjectEntry]) -> Vec<ProjectEntry> {
    let mut sorted = projects.to_vec();
    sorted.sort_by(|a, b| {
        b.pinned.cmp(&a.pinned).then_with(|| b.last_opened_at.cmp(&a.last_opened_at))
    });
    sorted
}

#[cfg(feature = "web")]
fn ndbstudio_root_dir() -> Result<PathBuf> {
    let base = default_session_path()?;
    base.parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("invalid default session path"))
}

#[cfg(feature = "web")]
fn session_path_for_db(db_path: &str) -> Result<PathBuf> {
    let root = ndbstudio_root_dir()?;
    let sessions_dir = root.join("sessions");
    Ok(sessions_dir.join(format!("{}.json", session_file_key(db_path))))
}

#[cfg(feature = "web")]
fn recent_dbs_path() -> Result<PathBuf> {
    Ok(ndbstudio_root_dir()?.join("recent_dbs.json"))
}

#[cfg(feature = "web")]
fn load_or_create_session(db_path: &str) -> Result<(SessionState, bool)> {
    let session_path = session_path_for_db(db_path)?;
    if session_path.exists() {
        let state = load_session_state(&session_path).with_context(|| {
            format!("failed to restore session for database at {}", db_path)
        })?;
        return Ok((state, true));
    }
    let state = SessionState::new(db_path);
    save_session_state(&session_path, &state)?;
    Ok((state, false))
}

#[cfg(feature = "web")]
fn load_recent_dbs() -> Result<Vec<String>> {
    let path = recent_dbs_path()?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("failed to read recent DB list from {}", path.display()))?;
    let recent = serde_json::from_str::<Vec<String>>(&raw)
        .with_context(|| format!("failed to parse recent DB list from {}", path.display()))?;
    Ok(recent)
}

#[cfg(feature = "web")]
fn save_recent_dbs(recent_dbs: &[String]) -> Result<()> {
    let path = recent_dbs_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(recent_dbs)
        .context("failed to serialize recent DB list")?;
    std::fs::write(&path, raw)
        .with_context(|| format!("failed to write recent DB list to {}", path.display()))?;
    Ok(())
}

#[cfg(feature = "web")]
fn update_recent_dbs(db_path: &str) -> Result<Vec<String>> {
    let recent = touch_recent_dbs(load_recent_dbs()?, db_path);
    save_recent_dbs(&recent)?;
    Ok(recent)
}

#[cfg(feature = "web")]
fn project_registry_path() -> Result<PathBuf> {
    Ok(ndbstudio_root_dir()?.join("projects.json"))
}

#[cfg(feature = "web")]
fn load_project_registry() -> Result<ProjectRegistry> {
    let path = project_registry_path()?;
    if path.exists() {
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read project registry from {}", path.display()))?;
        let registry = serde_json::from_str::<ProjectRegistry>(&raw)
            .with_context(|| format!("failed to parse project registry from {}", path.display()))?;
        return Ok(registry);
    }

    // Migrate from recent_dbs.json if it exists
    let recent = load_recent_dbs().unwrap_or_default();
    let now = chrono::Utc::now().to_rfc3339();
    let projects = recent
        .iter()
        .map(|db_path| ProjectEntry {
            db_path: db_path.clone(),
            name: database_name(db_path),
            description: String::new(),
            notes: String::new(),
            tags: Vec::new(),
            pinned: false,
            created_at: now.clone(),
            last_opened_at: now.clone(),
        })
        .collect();

    let registry = ProjectRegistry {
        version: 1,
        projects,
    };
    save_project_registry(&registry)?;
    Ok(registry)
}

#[cfg(feature = "web")]
fn save_project_registry(registry: &ProjectRegistry) -> Result<()> {
    let path = project_registry_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    let raw = serde_json::to_string_pretty(registry)
        .context("failed to serialize project registry")?;
    std::fs::write(&path, raw)
        .with_context(|| format!("failed to write project registry to {}", path.display()))?;
    Ok(())
}

#[cfg(feature = "web")]
fn touch_project(registry: &mut ProjectRegistry, db_path: &str) {
    let now = chrono::Utc::now().to_rfc3339();
    if let Some(project) = registry.projects.iter_mut().find(|p| p.db_path == db_path) {
        project.last_opened_at = now;
    } else {
        registry.projects.insert(
            0,
            ProjectEntry {
                db_path: db_path.to_string(),
                name: database_name(db_path),
                description: String::new(),
                notes: String::new(),
                tags: Vec::new(),
                pinned: false,
                created_at: now.clone(),
                last_opened_at: now,
            },
        );
    }
}

#[cfg(feature = "web")]
fn sanitize_db_name(name: &str) -> String {
    name.trim()
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            ' ' => '-',
            _ => '_',
        })
        .collect::<String>()
        .to_ascii_lowercase()
}

#[cfg(feature = "web")]
fn session_file_key(db_path: &str) -> String {
    let safe_name = database_name(db_path)
        .chars()
        .map(|ch| match ch {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => ch,
            _ => '_',
        })
        .collect::<String>();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    db_path.hash(&mut hasher);
    let hash = hasher.finish();
    format!("{}-{:016x}", safe_name, hash)
}

#[cfg(feature = "web")]
fn touch_recent_dbs(mut recent: Vec<String>, db_path: &str) -> Vec<String> {
    recent.retain(|path| path != db_path);
    recent.insert(0, db_path.to_string());
    recent.truncate(10);
    recent
}

pub async fn build_schema_snapshot(graph: &Graph, db_path: &str) -> Result<SchemaSnapshot> {
    let schema = graph.get_schema().await?;
    let stats = graph.get_stats().await?;

    let mut node_types = Vec::new();
    for label in &schema.node_labels {
        let mut properties = schema
            .node_properties
            .get(label)
            .map(|set| set.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        properties.sort();
        node_types.push(SchemaTypeSnapshot {
            name: label.clone(),
            count: schema.node_counts.get(label).copied().unwrap_or(0),
            properties,
        });
    }

    let mut edge_types = Vec::new();
    for edge_type in &schema.edge_types {
        let mut properties = schema
            .edge_properties
            .get(edge_type)
            .map(|set| set.iter().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        properties.sort();
        edge_types.push(SchemaTypeSnapshot {
            name: edge_type.clone(),
            count: schema.edge_counts.get(edge_type).copied().unwrap_or(0),
            properties,
        });
    }

    let density = if stats.total_nodes > 1 {
        stats.total_edges as f64 / ((stats.total_nodes * (stats.total_nodes - 1)) as f64)
    } else {
        0.0
    };

    Ok(SchemaSnapshot {
        db_path: db_path.to_string(),
        db_name: database_name(db_path),
        total_nodes: stats.total_nodes,
        total_edges: stats.total_edges,
        avg_degree: stats.avg_degree,
        density,
        node_types,
        edge_types,
    })
}

pub async fn build_graph_snapshot(graph: &Graph) -> Result<GraphSnapshot> {
    let nodes = graph.get_all_nodes().await?;
    let edges = graph.get_all_edges().await?;
    let mut degree_map: HashMap<String, usize> = HashMap::new();
    for edge in &edges {
        *degree_map.entry(edge.source.to_string()).or_insert(0) += 1;
        *degree_map.entry(edge.target.to_string()).or_insert(0) += 1;
    }

    Ok(GraphSnapshot {
        nodes: nodes
            .into_iter()
            .map(|n| GraphNodeSnapshot {
                id: n.id.to_string(),
                display_label: preferred_node_display_label(&n),
                entity_type: n.label.clone(),
                properties: node_properties_snapshot(&n),
                label: n.label,
                degree: degree_map.get(&n.id.to_string()).copied(),
            })
            .collect(),
        edges: edges
            .into_iter()
            .map(|e| GraphEdgeSnapshot {
                source: e.source.to_string(),
                target: e.target.to_string(),
                edge_type: e.edge_type,
            })
            .collect(),
    })
}

fn preferred_node_display_label(node: &nopaldb::Node) -> String {
    for key in ["name", "title", "display_name", "iri", "code"] {
        if let Some(value) = node.properties.get(key)
            && let Some(text) = property_value_as_string_for_display(value)
            && !text.is_empty()
        {
            return text;
        }
    }
    node.label.clone()
}

fn node_properties_snapshot(node: &nopaldb::Node) -> Vec<GraphNodePropertySnapshot> {
    let mut properties = node
        .properties
        .iter()
        .map(|(key, value)| GraphNodePropertySnapshot {
            key: key.clone(),
            value: property_value_to_display(value),
        })
        .collect::<Vec<_>>();
    properties.sort_by(|left, right| left.key.cmp(&right.key));
    properties
}

fn property_value_to_display(value: &PropertyValue) -> String {
    match value {
        PropertyValue::Null => "null".to_string(),
        PropertyValue::Bool(value) => value.to_string(),
        PropertyValue::Int(value) => value.to_string(),
        PropertyValue::Float(value) => value.to_string(),
        PropertyValue::String(value) => value.clone(),
        PropertyValue::Bytes(value) => format!("<{} bytes>", value.len()),
        PropertyValue::List(items) => {
            let inner: Vec<String> = items.iter().map(property_value_to_display).collect();
            format!("[{}]", inner.join(", "))
        }
        PropertyValue::Object(fields) => {
            let inner: Vec<String> = fields
                .iter()
                .map(|(k, v)| format!("{}: {}", k, property_value_to_display(v)))
                .collect();
            format!("{{{}}}", inner.join(", "))
        }
    }
}

fn property_value_as_string_for_display(value: &PropertyValue) -> Option<String> {
    match value {
        PropertyValue::String(value) => Some(value.clone()),
        PropertyValue::Int(value) => Some(value.to_string()),
        PropertyValue::Float(value) => Some(value.to_string()),
        PropertyValue::Bool(value) => Some(value.to_string()),
        PropertyValue::Null | PropertyValue::Bytes(_) | PropertyValue::List(_) | PropertyValue::Object(_) => None,
    }
}

#[cfg(feature = "web")]
pub async fn build_graph_subgraph(
    graph: &Graph,
    focus_node_id: Option<&str>,
    depth: usize,
    limit: usize,
    label: Option<&str>,
) -> Result<GraphSubgraphResponse> {
    let snapshot = build_graph_snapshot(graph).await?;
    let label_filter = label.map(|value| value.trim().to_ascii_lowercase()).filter(|v| !v.is_empty());
    let requested_limit = limit.clamp(1, 500);
    let requested_depth = depth.clamp(1, 3);

    let visible_nodes = snapshot
        .nodes
        .iter()
        .filter(|node| {
            label_filter
                .as_ref()
                .map(|needle| node.label.to_ascii_lowercase() == *needle)
                .unwrap_or(true)
        })
        .cloned()
        .collect::<Vec<_>>();

    if visible_nodes.is_empty() {
        return Ok(GraphSubgraphResponse {
            focus_node_id: None,
            depth: requested_depth,
            truncated: false,
            nodes: Vec::new(),
            edges: Vec::new(),
        });
    }

    let focus_id = focus_node_id
        .and_then(|id| visible_nodes.iter().find(|node| node.id == id).map(|_| id.to_string()))
        .unwrap_or_else(|| visible_nodes[0].id.clone());

    let mut adjacency: HashMap<String, Vec<String>> = HashMap::new();
    for edge in &snapshot.edges {
        adjacency
            .entry(edge.source.clone())
            .or_default()
            .push(edge.target.clone());
        adjacency
            .entry(edge.target.clone())
            .or_default()
            .push(edge.source.clone());
    }

    let visible_set = visible_nodes
        .iter()
        .map(|node| node.id.clone())
        .collect::<BTreeSet<_>>();

    let mut selected = BTreeSet::new();
    let mut queue = VecDeque::new();
    queue.push_back((focus_id.clone(), 0usize));
    let _ = selected.insert(focus_id.clone());

    while let Some((node_id, current_depth)) = queue.pop_front() {
        if selected.len() >= requested_limit || current_depth >= requested_depth {
            continue;
        }
        for neighbor in adjacency.get(&node_id).into_iter().flatten() {
            if !visible_set.contains(neighbor) {
                continue;
            }
            if selected.insert(neighbor.clone()) {
                queue.push_back((neighbor.clone(), current_depth + 1));
                if selected.len() >= requested_limit {
                    break;
                }
            }
        }
    }

    // Include orphan nodes (no edges) up to the limit
    for node_id in &visible_set {
        if selected.len() >= requested_limit {
            break;
        }
        if !adjacency.contains_key(node_id) {
            selected.insert(node_id.clone());
        }
    }

    let nodes = visible_nodes
        .into_iter()
        .filter(|node| selected.contains(&node.id))
        .collect::<Vec<_>>();
    let edges = snapshot
        .edges
        .into_iter()
        .filter(|edge| selected.contains(&edge.source) && selected.contains(&edge.target))
        .collect::<Vec<_>>();

    Ok(GraphSubgraphResponse {
        focus_node_id: Some(focus_id),
        depth: requested_depth,
        truncated: visible_set.len() > nodes.len(),
        nodes,
        edges,
    })
}

pub async fn execute_query(graph: &Graph, request: &QueryRunRequest) -> Result<QueryExecutionResult> {
    let (statement, mode_label) = match request.run_mode {
        RunMode::Run => (request.query.clone(), "Run"),
        RunMode::Explain => (maybe_prefix_explain(&request.query), "Explain"),
        RunMode::Profile => (request.query.clone(), "Profile"),
    };

    let nql_result = graph
        .execute_statement(&statement)
        .await
        .with_context(|| format!("{} execution failed", mode_label))?;
    let invalidation = classify_cache_invalidation(&statement, &nql_result);
    let touched = extract_query_entities(&statement);
    let (graph_hint, row_graph_hints) = match &nql_result {
        NqlResult::Query(query_result) if request.run_mode == RunMode::Run => {
            derive_graph_hints(graph, &request.query, query_result).await
        }
        NqlResult::Query(query_result) => (None, vec![None; query_result.rows.len()]),
        _ => (None, Vec::new()),
    };
    let mut summary = nql_result.summary();
    let mut mapped = to_tabular_result(nql_result);

    if request.run_mode == RunMode::Profile {
        let explain_stmt = maybe_prefix_explain(&request.query);
        let plan_preview = graph
            .execute_statement(&explain_stmt)
            .await
            .ok()
            .and_then(explain_preview_from_result);
        summary = format!(
            "PROFILE • {} • rows={} • cols={}{}",
            summary,
            mapped.rows.len(),
            mapped.headers.len(),
            plan_preview
                .as_ref()
                .map(|p| format!(" • plan: {}", truncate_one_line(p, 80)))
                .unwrap_or_default()
        );

        let mut profile_rows = vec![
            vec!["mode".to_string(), "profile".to_string()],
            vec!["summary".to_string(), summary.clone()],
            vec!["rows".to_string(), mapped.rows.len().to_string()],
            vec!["cols".to_string(), mapped.headers.len().to_string()],
        ];
        if let Some(plan) = plan_preview {
            profile_rows.push(vec![
                "plan_preview".to_string(),
                truncate_one_line(&plan, 160),
            ]);
        }
        mapped.headers = vec!["metric".to_string(), "value".to_string()];
        mapped.rows = profile_rows;
    } else if request.run_mode == RunMode::Explain {
        if let Some(plan) = mapped.rows.first().and_then(|r| r.first()) {
            summary = format!("EXPLAIN • {}", truncate_one_line(plan, 100));
        } else {
            summary = "EXPLAIN".to_string();
        }
    }

    Ok(QueryExecutionResult {
        run_mode: request.run_mode,
        summary,
        headers: mapped.headers,
        rows: mapped.rows,
        invalidation,
        touched_labels: touched.labels,
        touched_edge_types: touched.edge_types,
        touched_properties: touched.properties,
        graph_hint,
        row_graph_hints,
    })
}

#[cfg(feature = "web")]
#[cfg_attr(not(test), allow(dead_code))]
fn graph_hint_from_query_result(result: &QueryResult) -> Option<ResultGraphHint> {
    combine_row_graph_hints(&row_graph_hints_from_query_result(result), "Auto-focused from query results")
}

#[cfg(feature = "web")]
fn row_graph_hints_from_query_result(result: &QueryResult) -> Vec<Option<ResultGraphHint>> {
    if result.columns.is_empty() || result.rows.is_empty() {
        return Vec::new();
    }

    let candidate_indexes = result
        .columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| graph_candidate_kind(column).map(|kind| (index, kind)))
        .collect::<Vec<_>>();

    if candidate_indexes.is_empty() {
        return vec![None; result.rows.len()];
    }

    result
        .rows
        .iter()
        .map(|row| row_graph_hint_from_values(&result.columns, &row.values, &candidate_indexes))
        .collect()
}

#[cfg(feature = "web")]
fn row_graph_hint_from_values(
    columns: &[String],
    row_values: &HashMap<String, PropertyValue>,
    candidate_indexes: &[(usize, GraphCandidateKind)],
) -> Option<ResultGraphHint> {
    let values = columns
        .iter()
        .map(|column| row_values.get(column))
        .collect::<Vec<_>>();

    let mut row_node_ids = Vec::new();
    let mut seen_nodes = BTreeSet::new();
    let mut row_edges = BTreeSet::new();

    for (index, kind) in candidate_indexes {
        let Some(value) = values.get(*index).and_then(|value| *value) else {
            continue;
        };
        let Some(string_value) = property_value_as_string(value) else {
            continue;
        };
        if !looks_like_uuid(&string_value) {
            continue;
        }
        if seen_nodes.insert(string_value.clone()) {
            row_node_ids.push(string_value.clone());
        }
        if matches!(kind, GraphCandidateKind::Node | GraphCandidateKind::EdgeEndpoint) {
            let _ = kind;
        }
    }

    if row_node_ids.len() >= 2 {
        for pair in row_node_ids.windows(2) {
            if pair[0] != pair[1] {
                row_edges.insert((pair[0].clone(), pair[1].clone()));
            }
        }
    }

    if row_node_ids.is_empty() {
        return None;
    }

    Some(ResultGraphHint {
        mode: ResultGraphMode::ResultFocus,
        focus_node_id: row_node_ids.first().cloned(),
        node_ids: row_node_ids,
        edges: row_edges
            .into_iter()
            .map(|(source, target)| ResultGraphEdge { source, target })
            .collect(),
        note: Some("Auto-focused from selected result row".to_string()),
    })
}

#[cfg(feature = "web")]
fn combine_row_graph_hints(
    row_hints: &[Option<ResultGraphHint>],
    note: &str,
) -> Option<ResultGraphHint> {
    let mut node_ids = BTreeSet::new();
    let mut edges = BTreeSet::new();
    let mut focus = None;

    for hint in row_hints.iter().flatten() {
        if focus.is_none() {
            focus = hint.focus_node_id.clone();
        }
        for node_id in &hint.node_ids {
            node_ids.insert(node_id.clone());
        }
        for edge in &hint.edges {
            edges.insert((edge.source.clone(), edge.target.clone()));
        }
    }

    if node_ids.is_empty() {
        return None;
    }

    Some(ResultGraphHint {
        mode: ResultGraphMode::ResultFocus,
        focus_node_id: focus.or_else(|| node_ids.iter().next().cloned()),
        node_ids: node_ids.into_iter().collect(),
        edges: edges
            .into_iter()
            .map(|(source, target)| ResultGraphEdge { source, target })
            .collect(),
        note: Some(note.to_string()),
    })
}

#[cfg(feature = "web")]
async fn derive_graph_hints(
    graph: &Graph,
    query: &str,
    query_result: &QueryResult,
) -> (Option<ResultGraphHint>, Vec<Option<ResultGraphHint>>) {
    let direct_row_hints = row_graph_hints_from_query_result(query_result);
    if let Some(hint) = combine_row_graph_hints(&direct_row_hints, "Auto-focused from query results") {
        return (Some(hint), direct_row_hints);
    }

    if let Some((hint, row_hints)) = fallback_graph_hints_from_pattern(graph, query).await {
        return (Some(hint), row_hints);
    }

    (None, vec![None; query_result.rows.len()])
}

#[cfg(feature = "web")]
#[derive(Clone, Copy)]
enum GraphCandidateKind {
    Node,
    EdgeEndpoint,
}

#[cfg(feature = "web")]
fn graph_candidate_kind(column: &str) -> Option<GraphCandidateKind> {
    let lower = column.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    if lower == "id"
        || lower.ends_with(".id")
        || !lower.contains('.')
        || lower == "source"
        || lower == "target"
    {
        return Some(GraphCandidateKind::Node);
    }
    if lower.contains("source") || lower.contains("target") {
        return Some(GraphCandidateKind::EdgeEndpoint);
    }
    None
}

#[cfg(feature = "web")]
fn property_value_as_string(value: &PropertyValue) -> Option<String> {
    match value {
        PropertyValue::String(value) => Some(value.clone()),
        _ => None,
    }
}

#[cfg(feature = "web")]
#[allow(dead_code)]
async fn fallback_graph_hint_from_pattern(graph: &Graph, query: &str) -> Option<ResultGraphHint> {
    fallback_graph_hints_from_pattern(graph, query).await.map(|(hint, _)| hint)
}

#[cfg(feature = "web")]
async fn fallback_graph_hints_from_pattern(
    graph: &Graph,
    query: &str,
) -> Option<(ResultGraphHint, Vec<Option<ResultGraphHint>>)> {
    let fallback_query = fallback_graph_hint_query(query)?;
    let result = graph.execute_statement(&fallback_query).await.ok()?;
    let NqlResult::Query(query_result) = result else {
        return None;
    };
    let row_hints = row_graph_hints_from_query_result(&query_result);
    let mut hint = combine_row_graph_hints(&row_hints, "Auto-focused from graph pattern")?;
    hint.note = Some("Auto-focused from graph pattern".to_string());
    Some((hint, row_hints))
}

#[cfg(feature = "web")]
fn fallback_graph_hint_query(query: &str) -> Option<String> {
    let parsed = parse_query(query).ok()?;
    let pattern_variables = collect_node_pattern_variables(&parsed);
    if pattern_variables.is_empty() {
        return None;
    }

    let query_lower = query.to_ascii_lowercase();
    let from_index = find_clause_boundary(&query_lower, "from")?;
    let where_index = find_clause_boundary(&query_lower[from_index..], "where").map(|idx| from_index + idx);
    let order_index =
        find_order_by_boundary(&query_lower[from_index..]).map(|idx| from_index + idx);
    let limit_index = find_clause_boundary(&query_lower[from_index..], "limit").map(|idx| from_index + idx);

    let end = query.len();
    let first_after_from = [where_index, order_index, limit_index]
        .into_iter()
        .flatten()
        .min()
        .unwrap_or(end);
    let from_slice = query[from_index..first_after_from].trim_end();

    let where_slice = where_index.map(|start| {
        let end_index = [order_index, limit_index]
            .into_iter()
            .flatten()
            .filter(|idx| *idx > start)
            .min()
            .unwrap_or(end);
        query[start..end_index].trim_end().to_string()
    });

    // ORDER BY is intentionally dropped: the fallback re-projects pattern variables
    // so ORDER BY aliases from the original query may no longer be valid.
    let limit_slice = limit_index.map(|start| query[start..].trim().to_string());

    // The fallback query projects ONLY `var.id` — no aggregations, no other columns.
    // This keeps the fallback fast and avoids false positives: columns like `degree`
    // or `pagerank` (no dot in name) would otherwise be detected as graph-candidate
    // columns by `graph_candidate_kind`, fail the UUID check, and corrupt the hint.
    // We only need node IDs to tell the graph which nodes to highlight.
    let id_projections: Vec<String> = pattern_variables.iter()
        .map(|v| format!("{}.id", v))
        .collect();
    let find_clause = format!("find {}", id_projections.join(", "));

    let mut parts = vec![find_clause, from_slice.to_string()];
    if let Some(where_clause) = where_slice
        && !where_clause.is_empty()
    {
        parts.push(where_clause);
    }
    if let Some(limit_clause) = limit_slice
        && !limit_clause.is_empty()
    {
        parts.push(limit_clause);
    }

    Some(parts.join("\n"))
}

/// Extract the raw projections text from the FIND clause (everything between "find" and "from").
#[cfg(feature = "web")]
#[allow(dead_code)]
fn extract_find_projections(query: &str, from_index: usize) -> Option<String> {
    let find_clause = query[..from_index].trim();
    let lower = find_clause.to_ascii_lowercase();
    let after_find = lower.find("find")? + 4;
    let projections = find_clause[after_find..].trim();
    if projections.is_empty() {
        None
    } else {
        Some(projections.to_string())
    }
}

#[cfg(feature = "web")]
fn collect_node_pattern_variables(query: &nopaldb::NQLQuery) -> Vec<String> {
    let mut variables = Vec::new();
    for projection in &query.find.projections {
        if let Projection::Expression { .. } = projection {
            for pattern in &query.from.patterns {
                for element in &pattern.elements {
                    if let PatternElement::Node(node) = element
                        && let Some(variable) = &node.variable
                        && !variables.contains(variable)
                    {
                        variables.push(variable.clone());
                    }
                }
            }
            break;
        }
    }
    variables
}

#[cfg(feature = "web")]
fn find_clause_boundary(query_lower: &str, keyword: &str) -> Option<usize> {
    let bytes = query_lower.as_bytes();
    let keyword_bytes = keyword.as_bytes();
    let mut index = 0usize;

    while index + keyword_bytes.len() <= bytes.len() {
        if &bytes[index..index + keyword_bytes.len()] == keyword_bytes {
            let before_ok = index == 0 || bytes[index - 1].is_ascii_whitespace();
            let after_ok = index + keyword_bytes.len() == bytes.len()
                || bytes[index + keyword_bytes.len()].is_ascii_whitespace();
            if before_ok && after_ok {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

#[cfg(feature = "web")]
fn find_order_by_boundary(query_lower: &str) -> Option<usize> {
    let mut offset = 0usize;
    while let Some(found) = query_lower[offset..].find("order by") {
        let index = offset + found;
        let before_ok = index == 0 || query_lower.as_bytes()[index - 1].is_ascii_whitespace();
        let after_index = index + "order by".len();
        let after_ok = after_index == query_lower.len()
            || query_lower.as_bytes()[after_index].is_ascii_whitespace();
        if before_ok && after_ok {
            return Some(index);
        }
        offset = index + 1;
    }
    None
}

#[cfg(feature = "web")]
fn looks_like_uuid(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }
    value.chars().enumerate().all(|(index, ch)| match index {
        8 | 13 | 18 | 23 => ch == '-',
        _ => ch.is_ascii_hexdigit(),
    })
}

pub fn map_invalidation_to_change_kind(invalidation: QueryInvalidation) -> ChangeKind {
    match invalidation {
        QueryInvalidation::None => ChangeKind::Read,
        QueryInvalidation::Data => ChangeKind::DataWrite,
        QueryInvalidation::Schema => ChangeKind::SchemaWrite,
    }
}

#[cfg(feature = "web")]
pub fn classify_change_kind_from_query(query: &str) -> ChangeKind {
    let normalized = query.trim().to_ascii_lowercase();
    if normalized.starts_with("create index")
        || normalized.starts_with("drop index")
        || normalized.starts_with("create fulltext index")
        || normalized.starts_with("drop fulltext index")
    {
        ChangeKind::SchemaWrite
    } else if normalized.starts_with("add ")
        || normalized.starts_with("delete ")
        || normalized.starts_with("update ")
    {
        ChangeKind::DataWrite
    } else {
        ChangeKind::Read
    }
}

pub fn database_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .unwrap_or_else(|| path.to_string())
}

struct QueryEntities {
    labels: Vec<String>,
    edge_types: Vec<String>,
    properties: Vec<String>,
}

fn default_run_mode() -> RunMode {
    RunMode::Run
}

fn maybe_prefix_explain(query: &str) -> String {
    let trimmed = query.trim();
    if trimmed.to_ascii_lowercase().starts_with("explain ") {
        trimmed.to_string()
    } else {
        format!("explain {}", trimmed)
    }
}

fn explain_preview_from_result(result: NqlResult) -> Option<String> {
    match result {
        NqlResult::Explain(plan) => plan
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_string())
            .or(Some(plan)),
        _ => None,
    }
}

fn classify_cache_invalidation(statement: &str, result: &NqlResult) -> QueryInvalidation {
    let normalized = statement.trim().to_ascii_lowercase();

    if normalized.starts_with("create index")
        || normalized.starts_with("drop index")
        || normalized.starts_with("create fulltext index")
        || normalized.starts_with("drop fulltext index")
    {
        return QueryInvalidation::Schema;
    }

    match result {
        NqlResult::Write(write) => {
            let changes = write.nodes_created
                + write.edges_created
                + write.nodes_deleted
                + write.edges_deleted
                + write.nodes_updated
                + write.properties_changed;
            if changes == 0 {
                QueryInvalidation::None
            } else {
                QueryInvalidation::Data
            }
        }
        _ => QueryInvalidation::None,
    }
}

fn extract_query_entities(query: &str) -> QueryEntities {
    let chars = query.chars().collect::<Vec<_>>();
    let mut labels = std::collections::BTreeSet::new();
    let mut edge_types = std::collections::BTreeSet::new();
    let mut properties = std::collections::BTreeSet::new();

    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == ':' {
            let is_edge = i > 0 && chars[i - 1] == '[';
            let mut j = i + 1;
            while j < chars.len()
                && (chars[j].is_ascii_alphanumeric() || chars[j] == '_' || chars[j] == '-')
            {
                j += 1;
            }
            if j > i + 1 {
                let token: String = chars[i + 1..j].iter().collect();
                if is_edge {
                    edge_types.insert(token);
                } else {
                    labels.insert(token);
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }

    for token in query
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '.'))
        .filter(|t| !t.is_empty() && t.contains('.'))
    {
        let mut parts = token.split('.');
        let _var = parts.next();
        if let Some(prop) = parts.next()
            && !prop.is_empty()
        {
            properties.insert(prop.to_string());
        }
    }

    for token in query
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
    {
        if query.contains(&format!("{{{}:", token)) {
            properties.insert(token.to_string());
        }
    }

    QueryEntities {
        labels: labels.into_iter().collect(),
        edge_types: edge_types.into_iter().collect(),
        properties: properties.into_iter().collect(),
    }
}

#[cfg(feature = "web")]
fn format_error_chain(err: &anyhow::Error) -> String {
    let mut lines = vec![err.to_string()];
    for cause in err.chain().skip(1) {
        lines.push(format!("caused by: {}", cause));
    }
    lines.join("\n")
}

fn truncate_one_line(input: &str, max_len: usize) -> String {
    let trimmed = input.replace('\n', " ").trim().to_string();
    if trimmed.len() <= max_len {
        trimmed
    } else {
        format!("{}...", &trimmed[..max_len.saturating_sub(3)])
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "web")]
    use super::{
        fallback_graph_hint_query, graph_hint_from_query_result, row_graph_hints_from_query_result,
        sanitize_db_name, session_file_key, sorted_projects, touch_project, touch_recent_dbs,
        ProjectEntry, ProjectRegistry,
    };
    use super::{database_name, default_run_mode, maybe_prefix_explain};
    use crate::session::RunMode;
    #[cfg(feature = "web")]
    use nopaldb::query::nql::{QueryResult, Row};
    use nopaldb::PropertyValue;
    #[cfg(feature = "web")]
    use std::collections::HashMap;

    #[test]
    fn database_name_returns_filename() {
        assert_eq!(database_name("/tmp/my_graph.db"), "my_graph.db");
    }

    #[test]
    fn default_run_mode_is_run() {
        assert_eq!(default_run_mode(), RunMode::Run);
    }

    #[test]
    fn maybe_prefix_explain_preserves_existing_prefix() {
        assert_eq!(
            maybe_prefix_explain("explain find n from (n)"),
            "explain find n from (n)"
        );
    }

    #[test]
    fn maybe_prefix_explain_adds_prefix() {
        assert_eq!(
            maybe_prefix_explain("find n from (n)"),
            "explain find n from (n)"
        );
    }

    #[cfg(feature = "web")]
    #[test]
    fn session_file_key_is_stable_and_safe() {
        let key = session_file_key("/tmp/test dbs/florentine-families.db");
        assert!(key.starts_with("florentine-families_db-"));
        assert!(!key.contains('/'));
        assert!(!key.contains(' '));
    }

    #[cfg(feature = "web")]
    #[test]
    fn touch_recent_dbs_deduplicates_and_truncates() {
        let existing = (0..10)
            .map(|idx| format!("/tmp/db-{}.db", idx))
            .collect::<Vec<_>>();
        let updated = touch_recent_dbs(existing, "/tmp/db-4.db");
        assert_eq!(updated.first().map(String::as_str), Some("/tmp/db-4.db"));
        assert_eq!(updated.len(), 10);
        assert_eq!(
            updated.iter().filter(|path| path.as_str() == "/tmp/db-4.db").count(),
            1
        );
    }

    #[cfg(feature = "web")]
    #[test]
    fn graph_hint_detects_node_ids_from_query_result() {
        let mut row = HashMap::new();
        row.insert(
            "n".to_string(),
            PropertyValue::String("105f9844-f5ed-4223-bf1c-9cafbe676fc6".to_string()),
        );
        let result = QueryResult {
            columns: vec!["n".to_string()],
            rows: vec![Row { values: row }],
        };

        let hint = graph_hint_from_query_result(&result).expect("graph hint");
        assert_eq!(hint.node_ids.len(), 1);
        assert_eq!(hint.focus_node_id.as_deref(), Some("105f9844-f5ed-4223-bf1c-9cafbe676fc6"));
    }

    #[cfg(feature = "web")]
    #[test]
    fn row_graph_hints_preserve_per_row_focus() {
        let mut first = HashMap::new();
        first.insert(
            "a".to_string(),
            PropertyValue::String("105f9844-f5ed-4223-bf1c-9cafbe676fc6".to_string()),
        );
        first.insert(
            "b".to_string(),
            PropertyValue::String("205f9844-f5ed-4223-bf1c-9cafbe676fc6".to_string()),
        );
        let mut second = HashMap::new();
        second.insert(
            "a".to_string(),
            PropertyValue::String("305f9844-f5ed-4223-bf1c-9cafbe676fc6".to_string()),
        );
        second.insert(
            "b".to_string(),
            PropertyValue::String("405f9844-f5ed-4223-bf1c-9cafbe676fc6".to_string()),
        );
        let result = QueryResult {
            columns: vec!["a".to_string(), "b".to_string()],
            rows: vec![Row { values: first }, Row { values: second }],
        };

        let row_hints = row_graph_hints_from_query_result(&result);
        assert_eq!(row_hints.len(), 2);
        assert_eq!(
            row_hints[0].as_ref().and_then(|hint| hint.focus_node_id.as_deref()),
            Some("105f9844-f5ed-4223-bf1c-9cafbe676fc6")
        );
        assert_eq!(
            row_hints[1].as_ref().and_then(|hint| hint.focus_node_id.as_deref()),
            Some("305f9844-f5ed-4223-bf1c-9cafbe676fc6")
        );
    }

    #[cfg(feature = "web")]
    #[test]
    fn fallback_graph_hint_query_reprojects_pattern_variables_and_drops_order_by() {
        let query = r#"
            find x.name as bridge,
                 a.name as albizzi_family,
                 m.name as medici_family
            from (a:Family)-[:MARRIAGE]->(x:Family)-[:MARRIAGE]->(m:Family)
            where a.faction = "Albizzi" and m.faction = "Medici"
            order by bridge
            limit 10
        "#;

        let rewritten = fallback_graph_hint_query(query).expect("fallback query");
        assert!(rewritten.contains("find a.id, x.id, m.id"));
        assert!(rewritten.contains("from (a:Family)-[:MARRIAGE]->(x:Family)-[:MARRIAGE]->(m:Family)"));
        assert!(rewritten.contains("where a.faction = \"Albizzi\" and m.faction = \"Medici\""));
        assert!(rewritten.contains("limit 10"));
        assert!(!rewritten.to_ascii_lowercase().contains("order by"));
    }

    #[cfg(feature = "web")]
    #[test]
    fn touch_project_creates_new_entry_for_unknown_path() {
        let mut registry = ProjectRegistry::default();
        touch_project(&mut registry, "/tmp/new.db");
        assert_eq!(registry.projects.len(), 1);
        assert_eq!(registry.projects[0].name, "new.db");
    }

    #[cfg(feature = "web")]
    #[test]
    fn touch_project_updates_last_opened_for_existing_path() {
        let mut registry = ProjectRegistry::default();
        touch_project(&mut registry, "/tmp/existing.db");
        let first_opened = registry.projects[0].last_opened_at.clone();
        // Touch again — should update timestamp
        std::thread::sleep(std::time::Duration::from_millis(10));
        touch_project(&mut registry, "/tmp/existing.db");
        assert_eq!(registry.projects.len(), 1);
        assert!(registry.projects[0].last_opened_at >= first_opened);
    }

    #[cfg(feature = "web")]
    #[test]
    fn sorted_projects_pins_first_then_mru() {
        let now = "2026-01-01T00:00:00Z".to_string();
        let later = "2026-01-02T00:00:00Z".to_string();
        let projects = vec![
            ProjectEntry {
                db_path: "/a".into(), name: "A".into(), description: String::new(),
                notes: String::new(), tags: vec![], pinned: false,
                created_at: now.clone(), last_opened_at: now.clone(),
            },
            ProjectEntry {
                db_path: "/b".into(), name: "B".into(), description: String::new(),
                notes: String::new(), tags: vec![], pinned: true,
                created_at: now.clone(), last_opened_at: now.clone(),
            },
            ProjectEntry {
                db_path: "/c".into(), name: "C".into(), description: String::new(),
                notes: String::new(), tags: vec![], pinned: false,
                created_at: now.clone(), last_opened_at: later.clone(),
            },
        ];
        let sorted = sorted_projects(&projects);
        assert_eq!(sorted[0].name, "B"); // pinned
        assert_eq!(sorted[1].name, "C"); // more recent
        assert_eq!(sorted[2].name, "A"); // oldest
    }

    #[cfg(feature = "web")]
    #[test]
    fn sanitize_db_name_handles_spaces_and_special_chars() {
        assert_eq!(sanitize_db_name("My Test DB!"), "my-test-db_");
        assert_eq!(sanitize_db_name("simple"), "simple");
    }
}
