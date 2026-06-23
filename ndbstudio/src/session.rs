use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};

pub const SESSION_V2_FLAG_ENV: &str = "NDBSTUDIO_SESSION_V2";
const MAX_TIMELINE_ENTRIES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionState {
    pub version: u32,
    pub session_id: String,
    pub db_path: String,
    pub opened_at: String,
    pub active_tab_id: String,
    pub tabs: Vec<QueryTab>,
    pub timeline: Vec<TimelineEntry>,
    pub query_graph: QueryGraph,
    pub saved_queries: Vec<SavedQuery>,
    #[serde(default)]
    pub findings: Vec<FindingEntry>,
    pub active_parameters: BTreeMap<String, String>,
    pub last_result_ref: Option<ResultRef>,
    pub transaction_state: TransactionState,
    #[serde(default)]
    pub ui_preferences: Option<UiPreferences>,
}

impl SessionState {
    pub fn new(db_path: &str) -> Self {
        let first_tab = QueryTab {
            id: next_id("tab"),
            title: "Query 1".to_string(),
            query_text: String::new(),
            last_run_mode: None,
            last_result_ref: None,
            last_result: None,
            last_executed_at: None,
        };

        Self {
            version: 1,
            session_id: next_id("sess"),
            db_path: db_path.to_string(),
            opened_at: now_iso(),
            active_tab_id: first_tab.id.clone(),
            tabs: vec![first_tab],
            timeline: Vec::new(),
            query_graph: QueryGraph::default(),
            saved_queries: Vec::new(),
            findings: Vec::new(),
            active_parameters: BTreeMap::new(),
            last_result_ref: None,
            transaction_state: TransactionState::Closed,
            ui_preferences: None,
        }
    }

    pub fn active_tab(&self) -> Option<&QueryTab> {
        self.tabs.iter().find(|t| t.id == self.active_tab_id)
    }

    pub fn set_active_query_text(&mut self, query_text: &str) {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == self.active_tab_id) {
            tab.query_text = query_text.to_string();
        }
    }

    pub fn set_query_text_for_tab(&mut self, tab_id: &str, query_text: &str) -> bool {
        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.query_text = query_text.to_string();
            return true;
        }
        false
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_success(
        &mut self,
        run_mode: RunMode,
        cache_status: Option<CacheStatus>,
        change_kind: ChangeKind,
        touched_labels: Vec<String>,
        touched_edge_types: Vec<String>,
        touched_properties: Vec<String>,
        query: &str,
        summary: &str,
        row_count: usize,
        duration_ms: f64,
    ) {
        let query_hash = query_hash(query);
        let now = now_iso();
        let tab_id = self.active_tab_id.clone();
        let dependency_edges = self.derive_dependency_edges(
            &query_hash,
            &tab_id,
            &touched_labels,
            &touched_edge_types,
            &touched_properties,
        );
        let entry = TimelineEntry {
            id: next_id("run"),
            query: query.to_string(),
            normalized_query: Some(normalize_query(query)),
            run_mode,
            started_at: now.clone(),
            duration_ms: Some(duration_ms),
            status: RunStatus::Success,
            row_count: Some(row_count),
            summary: Some(summary.to_string()),
            error: None,
            tab_id: Some(tab_id.clone()),
            query_hash: Some(query_hash.clone()),
            params: self.active_parameters.clone(),
            pinned: false,
            cache_status,
            change_kind,
            touched_labels,
            touched_edge_types,
            touched_properties,
            depends_on: dependency_edges
                .iter()
                .map(|edge| edge.from_run_id.clone())
                .collect(),
            dependencies: dependency_edges
                .iter()
                .map(|edge| RunDependency {
                    run_id: edge.from_run_id.clone(),
                    reason: edge.reason,
                })
                .collect(),
        };

        let result_ref = ResultRef {
            query_hash,
            row_count,
            duration_ms: Some(duration_ms),
            captured_at: now.clone(),
        };

        self.timeline.push(entry);
        trim_timeline(&mut self.timeline);
        self.rebuild_query_graph();
        self.last_result_ref = Some(result_ref.clone());

        if let Some(tab) = self.tabs.iter_mut().find(|t| t.id == tab_id) {
            tab.last_result_ref = Some(result_ref);
            tab.last_executed_at = Some(now);
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_failure(
        &mut self,
        run_mode: RunMode,
        cache_status: Option<CacheStatus>,
        change_kind: ChangeKind,
        touched_labels: Vec<String>,
        touched_edge_types: Vec<String>,
        touched_properties: Vec<String>,
        query: &str,
        error: &str,
        duration_ms: Option<f64>,
    ) {
        let q_hash = query_hash(query);
        let tab_id = self.active_tab_id.clone();
        let dependency_edges = self.derive_dependency_edges(
            &q_hash,
            &tab_id,
            &touched_labels,
            &touched_edge_types,
            &touched_properties,
        );
        let entry = TimelineEntry {
            id: next_id("run"),
            query: query.to_string(),
            normalized_query: Some(normalize_query(query)),
            run_mode,
            started_at: now_iso(),
            duration_ms,
            status: RunStatus::Failure,
            row_count: None,
            summary: None,
            error: Some(error.to_string()),
            tab_id: Some(tab_id),
            query_hash: Some(q_hash),
            params: self.active_parameters.clone(),
            pinned: false,
            cache_status,
            change_kind,
            touched_labels,
            touched_edge_types,
            touched_properties,
            depends_on: dependency_edges
                .iter()
                .map(|edge| edge.from_run_id.clone())
                .collect(),
            dependencies: dependency_edges
                .iter()
                .map(|edge| RunDependency {
                    run_id: edge.from_run_id.clone(),
                    reason: edge.reason,
                })
                .collect(),
        };

        self.timeline.push(entry);
        trim_timeline(&mut self.timeline);
        self.rebuild_query_graph();
    }

    pub fn create_tab(&mut self, title: Option<&str>) -> String {
        let index = self.tabs.len() + 1;
        let tab = QueryTab {
            id: next_id("tab"),
            title: title
                .map(|t| t.trim().to_string())
                .filter(|t| !t.is_empty())
                .unwrap_or_else(|| format!("Query {}", index)),
            query_text: String::new(),
            last_run_mode: None,
            last_result_ref: None,
            last_result: None,
            last_executed_at: None,
        };
        let id = tab.id.clone();
        self.tabs.push(tab);
        self.active_tab_id = id.clone();
        id
    }

    pub fn close_active_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }

        let idx = self
            .tabs
            .iter()
            .position(|t| t.id == self.active_tab_id)
            .unwrap_or(0);
        self.tabs.remove(idx);
        let next_idx = idx.saturating_sub(1).min(self.tabs.len().saturating_sub(1));
        self.active_tab_id = self.tabs[next_idx].id.clone();
        true
    }

    pub fn activate_next_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        let idx = self
            .tabs
            .iter()
            .position(|t| t.id == self.active_tab_id)
            .unwrap_or(0);
        let next = (idx + 1) % self.tabs.len();
        self.active_tab_id = self.tabs[next].id.clone();
    }

    pub fn activate_prev_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        let idx = self
            .tabs
            .iter()
            .position(|t| t.id == self.active_tab_id)
            .unwrap_or(0);
        let prev = if idx == 0 {
            self.tabs.len() - 1
        } else {
            idx - 1
        };
        self.active_tab_id = self.tabs[prev].id.clone();
    }

    pub fn activate_tab_by_index(&mut self, index: usize) -> bool {
        let Some(tab) = self.tabs.get(index) else {
            return false;
        };
        self.active_tab_id = tab.id.clone();
        true
    }

    pub fn activate_tab_by_id(&mut self, tab_id: &str) -> bool {
        if self.tabs.iter().any(|tab| tab.id == tab_id) {
            self.active_tab_id = tab_id.to_string();
            return true;
        }
        false
    }

    pub fn rename_tab(&mut self, tab_id: &str, title: &str) -> bool {
        let title = title.trim();
        if title.is_empty() {
            return false;
        }
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == tab_id) {
            tab.title = title.to_string();
            return true;
        }
        false
    }

    pub fn close_tab_by_id(&mut self, tab_id: &str) -> bool {
        if self.tabs.len() <= 1 {
            return false;
        }
        let Some(idx) = self.tabs.iter().position(|tab| tab.id == tab_id) else {
            return false;
        };
        self.tabs.remove(idx);
        if self.active_tab_id == tab_id {
            let next_idx = idx.saturating_sub(1).min(self.tabs.len().saturating_sub(1));
            self.active_tab_id = self.tabs[next_idx].id.clone();
        }
        true
    }

    pub fn set_active_tab_result(&mut self, snapshot: QueryTabResultSnapshot) -> bool {
        let active_tab_id = self.active_tab_id.clone();
        if let Some(tab) = self.tabs.iter_mut().find(|tab| tab.id == active_tab_id) {
            tab.last_run_mode = Some(snapshot.run_mode);
            tab.last_result = Some(snapshot);
            return true;
        }
        false
    }

    pub fn save_query(&mut self, name: &str, query: &str) -> bool {
        let trimmed_name = name.trim();
        let trimmed_query = query.trim();
        if trimmed_name.is_empty() || trimmed_query.is_empty() {
            return false;
        }

        if let Some(existing) = self
            .saved_queries
            .iter_mut()
            .find(|q| q.name.eq_ignore_ascii_case(trimmed_name))
        {
            existing.query = trimmed_query.to_string();
            return true;
        }

        self.saved_queries.push(SavedQuery {
            id: next_id("saved"),
            name: trimmed_name.to_string(),
            query: trimmed_query.to_string(),
            tags: Vec::new(),
            created_at: now_iso(),
        });
        true
    }

    pub fn find_saved_query(&self, name: &str) -> Option<&SavedQuery> {
        self.saved_queries
            .iter()
            .find(|q| q.name.eq_ignore_ascii_case(name.trim()))
    }

    pub fn add_finding(&mut self, draft: FindingDraft) -> Option<FindingEntry> {
        let title = draft.title.trim();
        let body = draft.body.trim();
        if title.is_empty() && body.is_empty() {
            return None;
        }

        let finding = FindingEntry {
            id: next_id("finding"),
            title: if title.is_empty() {
                "Untitled finding".to_string()
            } else {
                title.to_string()
            },
            body: body.to_string(),
            created_at: now_iso(),
            updated_at: now_iso(),
            tab_id: draft.tab_id,
            run_id: draft.run_id,
            query_text: draft.query_text.unwrap_or_default(),
            summary: draft.summary.unwrap_or_default(),
            row_index: draft.row_index,
            graph_focus_node_id: draft.graph_focus_node_id,
        };
        self.findings.insert(0, finding.clone());
        Some(finding)
    }

    pub fn update_finding(
        &mut self,
        finding_id: &str,
        title: Option<&str>,
        body: Option<&str>,
    ) -> Option<FindingEntry> {
        let finding = self
            .findings
            .iter_mut()
            .find(|item| item.id == finding_id)?;
        if let Some(value) = title {
            let trimmed = value.trim();
            if !trimmed.is_empty() {
                finding.title = trimmed.to_string();
            }
        }
        if let Some(value) = body {
            finding.body = value.trim().to_string();
        }
        finding.updated_at = now_iso();
        Some(finding.clone())
    }

    pub fn delete_finding(&mut self, finding_id: &str) -> bool {
        let before = self.findings.len();
        self.findings.retain(|finding| finding.id != finding_id);
        before != self.findings.len()
    }

    pub fn delete_saved_query(&mut self, query_id: &str) -> bool {
        let before = self.saved_queries.len();
        self.saved_queries.retain(|q| q.id != query_id);
        self.saved_queries.len() < before
    }

    pub fn recent_timeline(&self, limit: usize) -> Vec<&TimelineEntry> {
        self.timeline.iter().rev().take(limit).collect()
    }

    pub fn toggle_recent_timeline_pin(&mut self, recent_index: usize) -> bool {
        let len = self.timeline.len();
        if recent_index >= len {
            return false;
        }
        let actual_index = len - 1 - recent_index;
        if let Some(entry) = self.timeline.get_mut(actual_index) {
            entry.pinned = !entry.pinned;
            return true;
        }
        false
    }

    pub fn dependent_queries_for_recent(
        &self,
        recent_index: usize,
        limit: usize,
    ) -> Vec<(String, RunMode)> {
        let recent = self.recent_timeline(limit);
        let Some(target) = recent.get(recent_index) else {
            return Vec::new();
        };
        let target_id = &target.id;
        self.timeline
            .iter()
            .filter(|entry| entry.depends_on.iter().any(|dep| dep == target_id))
            .map(|entry| (entry.query.clone(), entry.run_mode))
            .collect()
    }

    pub fn impacted_dependent_queries_scored_for_recent(
        &self,
        recent_index: usize,
        limit: usize,
    ) -> Vec<ImpactedRun> {
        let recent = self.recent_timeline(limit);
        let Some(target) = recent.get(recent_index) else {
            return Vec::new();
        };

        let mut impacted = Vec::new();
        let mut queue = vec![target.id.clone()];
        let mut visited = std::collections::BTreeSet::new();
        let _ = visited.insert(target.id.clone());

        while let Some(current) = queue.pop() {
            for entry in &self.timeline {
                if !entry.depends_on.iter().any(|dep| dep == &current) {
                    continue;
                }
                if !visited.insert(entry.id.clone()) {
                    continue;
                }
                if let Some(score) = self.semantic_impact_score(target, entry) {
                    impacted.push(ImpactedRun {
                        run_id: entry.id.clone(),
                        query: entry.query.clone(),
                        run_mode: entry.run_mode,
                        impact_score: score,
                    });
                    queue.push(entry.id.clone());
                }
            }
        }

        impacted.sort_by(|a, b| b.impact_score.cmp(&a.impact_score));
        impacted
    }

    pub fn lineage_summary_for_recent(&self, recent_index: usize, limit: usize) -> Option<String> {
        let recent = self.recent_timeline(limit);
        let entry = recent.get(recent_index)?;
        let deps = entry.depends_on.len();
        let dependents = self
            .timeline
            .iter()
            .filter(|e| e.depends_on.iter().any(|dep| dep == &entry.id))
            .count();
        Some(format!(
            "Lineage #{} deps={} dependents={} graph_nodes={} graph_edges={}",
            recent_index + 1,
            deps,
            dependents,
            self.query_graph.nodes.len(),
            self.query_graph.edges.len()
        ))
    }

    fn derive_dependency_edges(
        &self,
        query_hash: &str,
        tab_id: &str,
        touched_labels: &[String],
        touched_edge_types: &[String],
        touched_properties: &[String],
    ) -> Vec<QueryGraphEdge> {
        let mut out = Vec::new();
        let mut seen = std::collections::BTreeSet::new();

        if let Some(prev_tab) = self
            .timeline
            .iter()
            .rev()
            .find(|e| e.tab_id.as_deref() == Some(tab_id))
            && seen.insert(prev_tab.id.clone())
        {
            out.push(QueryGraphEdge {
                from_run_id: prev_tab.id.clone(),
                to_run_id: String::new(),
                reason: DependencyReason::TabSequence,
            });
        }

        if let Some(prev_same_query) = self
            .timeline
            .iter()
            .rev()
            .find(|e| e.query_hash.as_deref() == Some(query_hash))
            && seen.insert(prev_same_query.id.clone())
        {
            out.push(QueryGraphEdge {
                from_run_id: prev_same_query.id.clone(),
                to_run_id: String::new(),
                reason: DependencyReason::SameQueryHash,
            });
        }

        if let Some(prev_write) = self.timeline.iter().rev().find(|e| {
            matches!(
                e.change_kind,
                ChangeKind::DataWrite | ChangeKind::SchemaWrite
            )
        }) && seen.insert(prev_write.id.clone())
        {
            out.push(QueryGraphEdge {
                from_run_id: prev_write.id.clone(),
                to_run_id: String::new(),
                reason: DependencyReason::AfterWrite,
            });
        }

        if let Some(shared) = self.timeline.iter().rev().find(|e| {
            overlaps(touched_labels, &e.touched_labels)
                || overlaps(touched_edge_types, &e.touched_edge_types)
        }) && seen.insert(shared.id.clone())
        {
            out.push(QueryGraphEdge {
                from_run_id: shared.id.clone(),
                to_run_id: String::new(),
                reason: DependencyReason::SharedEntity,
            });
        }
        if let Some(shared_prop) = self
            .timeline
            .iter()
            .rev()
            .find(|e| overlaps(touched_properties, &e.touched_properties))
            && seen.insert(shared_prop.id.clone())
        {
            out.push(QueryGraphEdge {
                from_run_id: shared_prop.id.clone(),
                to_run_id: String::new(),
                reason: DependencyReason::SharedProperty,
            });
        }

        out
    }

    fn rebuild_query_graph(&mut self) {
        let nodes = self
            .timeline
            .iter()
            .map(|entry| QueryGraphNode {
                run_id: entry.id.clone(),
                query_hash: entry.query_hash.clone(),
                run_mode: entry.run_mode,
                status: entry.status.clone(),
                started_at: entry.started_at.clone(),
                tab_id: entry.tab_id.clone(),
            })
            .collect::<Vec<_>>();

        let mut edges = Vec::new();
        for entry in &self.timeline {
            for dep in &entry.dependencies {
                edges.push(QueryGraphEdge {
                    from_run_id: dep.run_id.clone(),
                    to_run_id: entry.id.clone(),
                    reason: dep.reason,
                });
            }
        }

        self.query_graph = QueryGraph { nodes, edges };
    }

    fn semantic_impact_score(
        &self,
        source: &TimelineEntry,
        candidate: &TimelineEntry,
    ) -> Option<u8> {
        if matches!(source.change_kind, ChangeKind::SchemaWrite) {
            return Some(100);
        }
        if matches!(source.change_kind, ChangeKind::DataWrite) {
            let mut score = 0u8;
            if overlaps(&source.touched_labels, &candidate.touched_labels) {
                score = score.saturating_add(45);
            }
            if overlaps(&source.touched_edge_types, &candidate.touched_edge_types) {
                score = score.saturating_add(35);
            }
            if overlaps(&source.touched_properties, &candidate.touched_properties) {
                score = score.saturating_add(25);
            }
            if score > 0 {
                return Some(score.min(100));
            }
            return None;
        }
        if source
            .depends_on
            .iter()
            .any(|dep| candidate.depends_on.iter().any(|cdep| cdep == dep))
        {
            Some(20)
        } else {
            None
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryTab {
    pub id: String,
    pub title: String,
    pub query_text: String,
    #[serde(default)]
    pub last_run_mode: Option<RunMode>,
    pub last_result_ref: Option<ResultRef>,
    #[serde(default)]
    pub last_result: Option<QueryTabResultSnapshot>,
    pub last_executed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryTabResultSnapshot {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
    pub summary: String,
    pub row_count: usize,
    pub duration_ms: Option<f64>,
    pub run_mode: RunMode,
    pub error: Option<String>,
    #[serde(default)]
    pub graph_hint: Option<ResultGraphHint>,
    #[serde(default)]
    pub row_graph_hints: Vec<Option<ResultGraphHint>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultGraphHint {
    pub mode: ResultGraphMode,
    pub node_ids: Vec<String>,
    #[serde(default)]
    pub edges: Vec<ResultGraphEdge>,
    pub focus_node_id: Option<String>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResultGraphMode {
    ResultFocus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultGraphEdge {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimelineEntry {
    pub id: String,
    pub query: String,
    pub normalized_query: Option<String>,
    pub run_mode: RunMode,
    pub started_at: String,
    pub duration_ms: Option<f64>,
    pub status: RunStatus,
    pub row_count: Option<usize>,
    pub summary: Option<String>,
    pub error: Option<String>,
    pub tab_id: Option<String>,
    pub query_hash: Option<String>,
    pub params: BTreeMap<String, String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub cache_status: Option<CacheStatus>,
    #[serde(default)]
    pub change_kind: ChangeKind,
    #[serde(default)]
    pub touched_labels: Vec<String>,
    #[serde(default)]
    pub touched_edge_types: Vec<String>,
    #[serde(default)]
    pub touched_properties: Vec<String>,
    #[serde(default)]
    pub depends_on: Vec<String>,
    #[serde(default)]
    pub dependencies: Vec<RunDependency>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunMode {
    Run,
    Explain,
    Profile,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    Success,
    Failure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheStatus {
    Hit,
    Miss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    #[default]
    Read,
    DataWrite,
    SchemaWrite,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct QueryGraph {
    pub nodes: Vec<QueryGraphNode>,
    pub edges: Vec<QueryGraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryGraphNode {
    pub run_id: String,
    pub query_hash: Option<String>,
    pub run_mode: RunMode,
    pub status: RunStatus,
    pub started_at: String,
    pub tab_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryGraphEdge {
    pub from_run_id: String,
    pub to_run_id: String,
    pub reason: DependencyReason,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyReason {
    TabSequence,
    SameQueryHash,
    AfterWrite,
    SharedEntity,
    SharedProperty,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunDependency {
    pub run_id: String,
    pub reason: DependencyReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactedRun {
    pub run_id: String,
    pub query: String,
    pub run_mode: RunMode,
    pub impact_score: u8,
}

fn overlaps(left: &[String], right: &[String]) -> bool {
    if left.is_empty() || right.is_empty() {
        return false;
    }
    let set = left
        .iter()
        .map(|v| v.to_ascii_lowercase())
        .collect::<std::collections::BTreeSet<_>>();
    right.iter().any(|v| set.contains(&v.to_ascii_lowercase()))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedQuery {
    pub id: String,
    pub name: String,
    pub query: String,
    pub tags: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FindingEntry {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default)]
    pub tab_id: Option<String>,
    #[serde(default)]
    pub run_id: Option<String>,
    #[serde(default)]
    pub query_text: String,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub row_index: Option<usize>,
    #[serde(default)]
    pub graph_focus_node_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FindingDraft {
    pub title: String,
    pub body: String,
    pub tab_id: Option<String>,
    pub run_id: Option<String>,
    pub query_text: Option<String>,
    pub summary: Option<String>,
    pub row_index: Option<usize>,
    pub graph_focus_node_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiPreferences {
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_workspace_tab")]
    pub workspace_tab: String,
    #[serde(default = "default_run_mode_str")]
    pub run_mode: String,
    #[serde(default = "default_graph_layout")]
    pub graph_layout: String,
    #[serde(default = "default_graph_depth")]
    pub graph_depth: u32,
    #[serde(default = "default_graph_limit")]
    pub graph_limit: u32,
    #[serde(default)]
    pub graph_type_filter: String,
    #[serde(default)]
    pub sidebar_collapsed: bool,
}

impl Default for UiPreferences {
    fn default() -> Self {
        Self {
            theme: default_theme(),
            workspace_tab: default_workspace_tab(),
            run_mode: default_run_mode_str(),
            graph_layout: default_graph_layout(),
            graph_depth: default_graph_depth(),
            graph_limit: default_graph_limit(),
            graph_type_filter: String::new(),
            sidebar_collapsed: false,
        }
    }
}

fn default_theme() -> String {
    "light".to_string()
}
fn default_workspace_tab() -> String {
    "results".to_string()
}
fn default_run_mode_str() -> String {
    "run".to_string()
}
fn default_graph_layout() -> String {
    "radial".to_string()
}
fn default_graph_depth() -> u32 {
    1
}
fn default_graph_limit() -> u32 {
    50
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultRef {
    pub query_hash: String,
    pub row_count: usize,
    pub duration_ms: Option<f64>,
    pub captured_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum TransactionState {
    #[default]
    Closed,
    Open {
        started_at: String,
    },
}

pub fn session_v2_enabled_from_env() -> bool {
    match std::env::var(SESSION_V2_FLAG_ENV) {
        Ok(value) => parse_truthy_flag(&value),
        Err(_) => false,
    }
}

pub fn default_session_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable is not set")?;
    Ok(PathBuf::from(home).join(".ndstudio").join("session.json"))
}

pub fn load_session_state(path: &Path) -> Result<SessionState> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read session state from {}", path.display()))?;
    serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse session state from {}", path.display()))
}

pub fn save_session_state(path: &Path, state: &SessionState) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create session directory {}", parent.display()))?;
    }

    let raw =
        serde_json::to_string_pretty(state).context("failed to serialize session state to JSON")?;
    std::fs::write(path, raw)
        .with_context(|| format!("failed to write session state to {}", path.display()))?;
    Ok(())
}

pub fn session_summary(state: &SessionState) -> String {
    let active_params = state.active_parameters.len();
    format!(
        "session={} tabs={} timeline={} params={}",
        state.session_id,
        state.tabs.len(),
        state.timeline.len(),
        active_params
    )
}

fn trim_timeline(timeline: &mut Vec<TimelineEntry>) {
    if timeline.len() <= MAX_TIMELINE_ENTRIES {
        return;
    }
    let overflow = timeline.len() - MAX_TIMELINE_ENTRIES;
    timeline.drain(0..overflow);
}

fn next_id(prefix: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{}-{}", prefix, nanos)
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn normalize_query(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn query_hash(query: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalize_query(query).hash(&mut hasher);
    format!("{:x}", hasher.finish())
}

fn parse_truthy_flag(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on" | "enabled"
    )
}

#[cfg(test)]
mod tests {
    use super::{
        CacheStatus, ChangeKind, FindingDraft, QueryTabResultSnapshot, RunMode, SessionState,
        parse_truthy_flag,
    };

    #[test]
    fn session_state_has_default_tab() {
        let state = SessionState::new("test_dbs/sample.db");
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.active_tab_id, state.tabs[0].id);
    }

    #[test]
    fn session_state_records_timeline() {
        let mut state = SessionState::new("test_dbs/sample.db");
        state.set_active_query_text("find * from (n)");
        state.record_success(
            RunMode::Run,
            Some(CacheStatus::Miss),
            ChangeKind::Read,
            vec![],
            vec![],
            vec![],
            "find * from (n)",
            "2 rows",
            2,
            1.2,
        );
        state.record_failure(
            RunMode::Explain,
            Some(CacheStatus::Miss),
            ChangeKind::Read,
            vec![],
            vec![],
            vec![],
            "find bad",
            "syntax error",
            Some(0.3),
        );
        assert_eq!(state.timeline.len(), 2);
    }

    #[test]
    fn flag_parser_understands_truthy_values() {
        assert!(parse_truthy_flag("true"));
        assert!(parse_truthy_flag("1"));
        assert!(parse_truthy_flag("enabled"));
        assert!(!parse_truthy_flag("0"));
        assert!(!parse_truthy_flag("no"));
    }

    #[test]
    fn tabs_can_be_created_and_switched() {
        let mut state = SessionState::new("test_dbs/sample.db");
        let first = state.active_tab_id.clone();
        state.create_tab(Some("Tab 2"));
        assert_ne!(state.active_tab_id, first);
        state.activate_prev_tab();
        assert_eq!(state.active_tab_id, first);
    }

    #[test]
    fn saving_query_replaces_existing_name() {
        let mut state = SessionState::new("test_dbs/sample.db");
        assert!(state.save_query("Top", "find * from (n)"));
        assert!(state.save_query("Top", "find count(*) from (n)"));
        assert_eq!(state.saved_queries.len(), 1);
    }

    #[test]
    fn can_activate_tab_by_index() {
        let mut state = SessionState::new("test_dbs/sample.db");
        let first = state.active_tab_id.clone();
        state.create_tab(Some("Two"));
        assert!(state.activate_tab_by_index(0));
        assert_eq!(state.active_tab_id, first);
        assert!(!state.activate_tab_by_index(99));
    }

    #[test]
    fn can_activate_rename_and_close_tab_by_id() {
        let mut state = SessionState::new("test_dbs/sample.db");
        let first = state.active_tab_id.clone();
        let second = state.create_tab(Some("Two"));
        assert!(state.rename_tab(&second, "Two renamed"));
        assert!(state.activate_tab_by_id(&second));
        assert_eq!(
            state.active_tab().map(|tab| tab.title.as_str()),
            Some("Two renamed")
        );
        assert!(state.close_tab_by_id(&second));
        assert_eq!(state.tabs.len(), 1);
        assert_eq!(state.active_tab_id, first);
    }

    #[test]
    fn active_tab_can_store_last_result_snapshot() {
        let mut state = SessionState::new("test_dbs/sample.db");
        assert!(state.set_active_tab_result(QueryTabResultSnapshot {
            headers: vec!["n".into()],
            rows: vec![vec!["node-1".into()]],
            summary: "1 row returned".into(),
            row_count: 1,
            duration_ms: Some(1.1),
            run_mode: RunMode::Run,
            error: None,
            graph_hint: None,
            row_graph_hints: vec![None],
        }));
        let tab = state.active_tab().expect("active tab");
        assert_eq!(tab.last_run_mode, Some(RunMode::Run));
        assert_eq!(
            tab.last_result
                .as_ref()
                .and_then(|result| result.headers.first()),
            Some(&"n".to_string())
        );
    }

    #[test]
    fn can_create_update_and_delete_findings() {
        let mut state = SessionState::new("test_dbs/sample.db");
        let finding = state
            .add_finding(FindingDraft {
                title: "Bridge family".into(),
                body: "Looks like an intermediary family".into(),
                tab_id: Some(state.active_tab_id.clone()),
                run_id: Some("run_1".into()),
                query_text: Some("find x from (x)".into()),
                summary: Some("1 row returned".into()),
                row_index: Some(0),
                graph_focus_node_id: Some("node_1".into()),
            })
            .expect("finding created");
        assert_eq!(state.findings.len(), 1);
        assert_eq!(state.findings[0].title, "Bridge family");

        let updated = state
            .update_finding(
                &finding.id,
                Some("Bridge family updated"),
                Some("Refined interpretation"),
            )
            .expect("finding updated");
        assert_eq!(updated.title, "Bridge family updated");
        assert_eq!(updated.body, "Refined interpretation");

        assert!(state.delete_finding(&finding.id));
        assert!(state.findings.is_empty());
    }

    #[test]
    fn findings_survive_session_roundtrip() {
        let mut state = SessionState::new("test_dbs/sample.db");
        state.add_finding(FindingDraft {
            title: "Observation".into(),
            body: "Important detail".into(),
            tab_id: Some(state.active_tab_id.clone()),
            run_id: None,
            query_text: Some("find n from (n)".into()),
            summary: Some("sample".into()),
            row_index: None,
            graph_focus_node_id: None,
        });

        let raw = serde_json::to_string(&state).expect("serialize session");
        let restored: SessionState = serde_json::from_str(&raw).expect("deserialize session");
        assert_eq!(restored.findings.len(), 1);
        assert_eq!(restored.findings[0].title, "Observation");
    }

    #[test]
    fn can_toggle_pin_on_recent_timeline_entry() {
        let mut state = SessionState::new("test_dbs/sample.db");
        state.record_success(
            RunMode::Profile,
            Some(CacheStatus::Hit),
            ChangeKind::Read,
            vec![],
            vec![],
            vec![],
            "find 1",
            "ok",
            1,
            1.0,
        );
        assert!(!state.timeline[0].pinned);
        assert!(state.toggle_recent_timeline_pin(0));
        assert!(state.timeline[0].pinned);
    }

    #[test]
    fn query_graph_tracks_dependencies() {
        let mut state = SessionState::new("test_dbs/sample.db");
        state.record_success(
            RunMode::Run,
            Some(CacheStatus::Miss),
            ChangeKind::Read,
            vec!["Person".to_string()],
            vec![],
            vec!["name".to_string()],
            "find a from (a:Person)",
            "ok",
            1,
            1.0,
        );
        state.record_success(
            RunMode::Run,
            Some(CacheStatus::Miss),
            ChangeKind::Read,
            vec!["Person".to_string()],
            vec![],
            vec!["name".to_string()],
            "find a from (a:Person)",
            "ok",
            1,
            1.0,
        );

        assert_eq!(state.query_graph.nodes.len(), 2);
        assert!(!state.query_graph.edges.is_empty());
        let deps = state.dependent_queries_for_recent(1, 20);
        assert!(!deps.is_empty());
    }

    #[test]
    fn impacted_dependents_prioritize_semantic_overlap() {
        let mut state = SessionState::new("test_dbs/sample.db");
        state.record_success(
            RunMode::Run,
            Some(CacheStatus::Miss),
            ChangeKind::DataWrite,
            vec!["Character".to_string()],
            vec!["ALLY_WITH".to_string()],
            vec!["name".to_string()],
            "update (c:Character) set c.name = 'x'",
            "ok",
            1,
            1.0,
        );
        state.record_success(
            RunMode::Run,
            Some(CacheStatus::Miss),
            ChangeKind::Read,
            vec!["Character".to_string()],
            vec![],
            vec!["name".to_string()],
            "find c.name from (c:Character)",
            "ok",
            1,
            1.0,
        );
        state.record_success(
            RunMode::Run,
            Some(CacheStatus::Miss),
            ChangeKind::Read,
            vec!["House".to_string()],
            vec![],
            vec!["region".to_string()],
            "find h.region from (h:House)",
            "ok",
            1,
            1.0,
        );

        let impacted = state.impacted_dependent_queries_scored_for_recent(2, 20);
        assert!(!impacted.is_empty());
        assert!(impacted[0].impact_score >= 45);
    }
}
