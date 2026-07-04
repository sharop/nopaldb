use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
    time::Duration,
    time::Instant,
};

use anyhow::{Context, Result};
use chrono::Local;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use nopaldb::{Graph, PropertyValue};
use serde::{Deserialize, Serialize};
use tokio::runtime::{Builder, Runtime};

use crate::session::{
    default_session_path, load_session_state, save_session_state, session_summary,
    session_v2_enabled_from_env, CacheStatus, ChangeKind, RunMode, SessionState,
};
use crate::ui::{
    editor::QueryEditor,
    graph::GraphView,
    history::HistoryView,
    results::{ResultsMode, ResultsView},
    schema::SchemaView,
};
use crate::workbench::{
    self, QueryExecutionResult, QueryInvalidation, QueryRunRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    Command,
    Visual,
    History,
    Session,
    Help,
    Palette,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusedPane {
    Editor,
    Results,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SidePanel {
    #[default]
    None,
    Schema,
    Graph,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionPane {
    Timeline,
    Snippets,
    Tabs,
}

#[derive(Debug, Clone)]
struct SessionBrowser {
    active: SessionPane,
    timeline_idx: usize,
    snippet_idx: usize,
    tab_idx: usize,
    filter_text: String,
    filter_editing: bool,
}

#[derive(Debug, Clone)]
struct PaletteState {
    query: String,
    selected: usize,
    return_mode: Mode,
}

impl Default for PaletteState {
    fn default() -> Self {
        Self {
            query: String::new(),
            selected: 0,
            return_mode: Mode::Normal,
        }
    }
}

#[derive(Debug, Clone)]
enum PaletteAction {
    Execute(RunMode),
    RunCommand(String),
    RerunTimeline(usize),
    LoadSnippet(String),
    ActivateTab(usize),
}

#[derive(Debug, Clone)]
struct PaletteEntry {
    title: String,
    detail: String,
    action: PaletteAction,
}

impl Default for SessionBrowser {
    fn default() -> Self {
        Self {
            active: SessionPane::Timeline,
            timeline_idx: 0,
            snippet_idx: 0,
            tab_idx: 0,
            filter_text: String::new(),
            filter_editing: false,
        }
    }
}

pub struct App {
    mode: Mode,
    focused_pane: FocusedPane,

    // UI Components
    pub editor: QueryEditor,
    pub results: ResultsView,
    pub schema: SchemaView,
    pub graph_view: GraphView,
    pub history: HistoryView,
    side_panel: SidePanel,
    side_panel_width: u16,

    // Database state
    graph: Graph,
    runtime: Runtime,
    db_path: String,
    db_info: String,
    pub status_message: String,
    graph_last_refresh: Option<String>,
    last_run_mode: Option<RunMode>,
    last_run_summary: Option<String>,
    last_cache_event: Option<CacheEvent>,
    cache: ResultCache,
    db_revision: u64,
    schema_revision: u64,
    session_v2_enabled: bool,
    session_state: SessionState,
    timeline_visible: bool,
    session_browser: SessionBrowser,
    palette: PaletteState,
    help_scroll: usize,
    help_return_mode: Mode,

    // Command buffer for ':' commands
    command_buffer: String,
    pending_query: Option<PendingQuery>,
    rerun_queue: VecDeque<QueuedRun>,
    quit_requested: bool,
}

impl App {
    pub fn new(db_path: &str) -> Result<Self> {
        let graph = Self::open_graph(db_path)?;
        Self::from_graph(db_path, graph)
    }

    pub fn open_graph(db_path: &str) -> Result<Graph> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to create async runtime")?;

        runtime
            .block_on(workbench::open_graph(db_path))
    }

    pub fn from_graph(db_path: &str, graph: Graph) -> Result<Self> {
        let runtime = Builder::new_current_thread()
            .enable_all()
            .build()
            .context("failed to create async runtime")?;

        let db_name = database_name(db_path);
        let session_v2_enabled = session_v2_enabled_from_env();
        let mut status_message = format!("Opened: {}", db_path);
        if session_v2_enabled {
            status_message.push_str(" • session-v2");
        }

        let mut app = Self {
            mode: Mode::Normal,
            focused_pane: FocusedPane::Editor,
            editor: QueryEditor::new(),
            results: ResultsView::new(),
            schema: SchemaView::new(),
            graph_view: GraphView::new(),
            history: HistoryView::new(),
            side_panel: SidePanel::None,
            side_panel_width: 32,
            graph,
            runtime,
            db_path: db_path.to_string(),
            db_info: format!("{} • 0 nodes • 0 edges", db_name),
            status_message,
            graph_last_refresh: None,
            last_run_mode: None,
            last_run_summary: None,
            last_cache_event: None,
            cache: ResultCache::new(128),
            db_revision: 0,
            schema_revision: 0,
            session_v2_enabled,
            session_state: SessionState::new(db_path),
            timeline_visible: false,
            session_browser: SessionBrowser::default(),
            palette: PaletteState::default(),
            help_scroll: 0,
            help_return_mode: Mode::Normal,
            command_buffer: String::new(),
            pending_query: None,
            rerun_queue: VecDeque::new(),
            quit_requested: false,
        };

        if app.session_v2_enabled {
            let _ = app.load_session_data();
        }
        app.load_ui_prefs()?;
        app.refresh_db_info()?;
        Ok(app)
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn focused_pane(&self) -> FocusedPane {
        self.focused_pane
    }

    pub fn command_buffer(&self) -> &str {
        &self.command_buffer
    }

    pub fn db_info_text(&self) -> &str {
        &self.db_info
    }

    pub fn is_schema_panel_visible(&self) -> bool {
        self.side_panel == SidePanel::Schema
    }

    pub fn is_graph_panel_visible(&self) -> bool {
        self.side_panel == SidePanel::Graph
    }

    pub fn side_panel(&self) -> SidePanel {
        self.side_panel
    }

    pub fn schema_panel_width(&self) -> u16 {
        self.side_panel_width
    }

    pub fn quit_requested(&self) -> bool {
        self.quit_requested
    }

    pub fn graph_focus_badge(&self) -> Option<String> {
        self.graph_view.focus_summary()
    }

    pub fn graph_last_refresh_text(&self) -> Option<&str> {
        self.graph_last_refresh.as_deref()
    }

    pub fn results_quick_view(&self) -> Option<String> {
        let mode = self.last_run_mode?;
        let summary = self.last_run_summary.as_deref().unwrap_or_default();
        let out = if summary.is_empty() {
            run_mode_short(mode).to_string()
        } else {
            format!("{}: {}", run_mode_short(mode), truncate_one_line(summary, 96))
        };
        Some(out)
    }

    pub fn cache_badge(&self) -> Option<&'static str> {
        self.last_cache_event.map(|e| match e {
            CacheEvent::Hit => "cache:hit",
            CacheEvent::Miss => "cache:miss",
        })
    }

    pub fn cache_hit_rate_summary(&self) -> Option<String> {
        let session = self.session_cache_hit_rate()?;
        let tab = self.active_tab_cache_hit_rate();
        Some(match tab {
            Some(tab_rate) => format!("cache {:.0}% tab {:.0}%", session * 100.0, tab_rate * 100.0),
            None => format!("cache {:.0}%", session * 100.0),
        })
    }

    pub fn session_browser_cache_health_text(&self) -> String {
        let recent = self.recent_cache_hit_rate(20, false).unwrap_or(0.0) * 100.0;
        let tab_recent = self.recent_cache_hit_rate(20, true).unwrap_or(0.0) * 100.0;
        format!("cache recent20 {:.0}% • tab {:.0}%", recent, tab_recent)
    }

    pub fn session_v2_enabled(&self) -> bool {
        self.session_v2_enabled
    }

    pub fn timeline_visible(&self) -> bool {
        self.session_v2_enabled && self.timeline_visible
    }

    pub fn session_browser_filter_text(&self) -> &str {
        &self.session_browser.filter_text
    }

    pub fn session_browser_filter_editing(&self) -> bool {
        self.session_browser.filter_editing
    }

    pub fn help_scroll(&self) -> usize {
        self.help_scroll
    }

    pub fn palette_query(&self) -> &str {
        &self.palette.query
    }

    pub fn palette_rows(&self, limit: usize) -> Vec<String> {
        let entries = self.palette_entries();
        entries
            .iter()
            .enumerate()
            .skip(self.palette.selected.saturating_sub(limit / 2))
            .take(limit)
            .map(|(idx, e)| {
                let marker = if idx == self.palette.selected { ">" } else { " " };
                format!("{} {}  [{}]", marker, e.title, e.detail)
            })
            .collect()
    }

    pub fn active_tab_title(&self) -> &str {
        self.session_state
            .active_tab()
            .map(|t| t.title.as_str())
            .unwrap_or("Query")
    }

    pub fn tab_position_text(&self) -> String {
        let total = self.session_state.tabs.len();
        let idx = self
            .session_state
            .tabs
            .iter()
            .position(|t| t.id == self.session_state.active_tab_id)
            .map(|i| i + 1)
            .unwrap_or(1);
        format!("{}/{}", idx, total.max(1))
    }

    pub fn timeline_rows(&self, limit: usize) -> Vec<String> {
        self.session_state
            .recent_timeline(limit)
            .iter()
            .enumerate()
            .map(|(idx, entry)| {
                let status = match entry.status {
                    crate::session::RunStatus::Success => "OK",
                    crate::session::RunStatus::Failure => "ERR",
                };
                let mode = run_mode_short(entry.run_mode);
                let ms = entry
                    .duration_ms
                    .map(|v| format!("{:.1}ms", v))
                    .unwrap_or_else(|| "-".to_string());
                let cache = match entry.cache_status {
                    Some(CacheStatus::Hit) => "HIT",
                    Some(CacheStatus::Miss) => "MISS",
                    None => "-",
                };
                let lineage = format!("d{}", entry.depends_on.len());
                let query_preview = if entry.query.len() > 60 {
                    format!("{}...", &entry.query[..60])
                } else {
                    entry.query.clone()
                };
                format!(
                    "{:>2}. [{} {} {} {} {}] {}",
                    idx + 1,
                    mode,
                    status,
                    cache,
                    lineage,
                    ms,
                    query_preview
                )
            })
            .collect()
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('k') {
            self.open_palette();
            return Ok(());
        }

        match self.mode {
            Mode::Normal => self.handle_normal_mode(key),
            Mode::Insert => self.handle_insert_mode(key),
            Mode::Command => self.handle_command_mode(key),
            Mode::Visual => self.handle_visual_mode(key),
            Mode::History => self.handle_history_mode(key),
            Mode::Session => self.handle_session_mode(key),
            Mode::Help => self.handle_help_mode(key),
            Mode::Palette => self.handle_palette_mode(key),
        }
    }

    fn handle_normal_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('?') => {
                self.open_help_modal();
            }
            KeyCode::Char('i') => {
                self.mode = Mode::Insert;
                self.status_message = "-- INSERT --".to_string();
            }
            KeyCode::Char(':') => {
                self.mode = Mode::Command;
                self.command_buffer.clear();
                self.status_message = ":".to_string();
            }
            KeyCode::Char('v') => {
                self.mode = Mode::Visual;
                self.status_message = "-- VISUAL --".to_string();
            }
            KeyCode::Char('b') if self.session_v2_enabled => {
                self.mode = Mode::Session;
                self.status_message =
                    "Session Browser • /:filter(mode:run|explain|profile) • Tab:pane • j/k:move • Enter:run • l:load • p:pin • g:dag • d:detail".to_string();
            }
            KeyCode::Char('y') if self.session_v2_enabled => {
                self.timeline_visible = !self.timeline_visible;
                self.status_message = if self.timeline_visible {
                    "Timeline shown".to_string()
                } else {
                    "Timeline hidden".to_string()
                };
            }
            KeyCode::Char('R') if self.session_v2_enabled => {
                self.status_message = self
                    .rerun_last_timeline_query()
                    .unwrap_or_else(|err| format!("Failed to rerun query: {}", err));
            }
            KeyCode::Char(']') if self.session_v2_enabled => {
                self.status_message = self
                    .activate_next_tab()
                    .unwrap_or_else(|err| format!("Failed to switch tab: {}", err));
            }
            KeyCode::Char('[') if self.session_v2_enabled => {
                self.status_message = self
                    .activate_prev_tab()
                    .unwrap_or_else(|err| format!("Failed to switch tab: {}", err));
            }
            KeyCode::Char('t')
                if self.session_v2_enabled && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.status_message = self
                    .create_new_tab(None)
                    .unwrap_or_else(|err| format!("Failed to create tab: {}", err));
            }
            KeyCode::Char('w')
                if self.session_v2_enabled && key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.status_message = self
                    .close_active_tab()
                    .unwrap_or_else(|err| format!("Failed to close tab: {}", err));
            }
            KeyCode::Char('s') => {
                self.status_message = self
                    .toggle_side_panel(SidePanel::Schema)
                    .unwrap_or_else(|err| format!("Failed to toggle schema panel: {}", err));
            }
            KeyCode::Char('x') => {
                self.status_message = self
                    .toggle_side_panel(SidePanel::Graph)
                    .unwrap_or_else(|err| format!("Failed to toggle graph panel: {}", err));
            }
            KeyCode::Char('f') if self.side_panel == SidePanel::Graph => {
                self.status_message = self
                    .focus_graph_from_results_row()
                    .unwrap_or_else(|err| format!("Failed to focus graph from results: {}", err));
            }
            KeyCode::Char('o') if self.side_panel == SidePanel::Graph => {
                if self.graph_view.focus_selected_neighbor() {
                    self.status_message = "Graph focus updated".to_string();
                } else {
                    self.status_message = "No neighbor selected".to_string();
                }
            }
            KeyCode::Char('r') if self.side_panel == SidePanel::Graph => {
                self.status_message = self
                    .refresh_graph_view()
                    .map(|_| "Graph view refreshed".to_string())
                    .unwrap_or_else(|err| format!("Failed to refresh graph view: {}", err));
            }

            KeyCode::Char('j') => match self.focused_pane {
                FocusedPane::Editor => self.editor.move_down(),
                FocusedPane::Results => self.results.scroll_down(),
            },
            KeyCode::Char('k') => match self.focused_pane {
                FocusedPane::Editor => self.editor.move_up(),
                FocusedPane::Results => self.results.scroll_up(),
            },
            KeyCode::Char('z')
                if self.focused_pane == FocusedPane::Results
                    && self.results.mode() == ResultsMode::Plan =>
            {
                if let Some(msg) = self.results.toggle_plan_section() {
                    self.status_message = msg;
                }
            }
            KeyCode::Down => match self.focused_pane {
                FocusedPane::Editor => self.editor.move_down(),
                FocusedPane::Results => self.results.scroll_down(),
            },
            KeyCode::Up => match self.focused_pane {
                FocusedPane::Editor => self.editor.move_up(),
                FocusedPane::Results => self.results.scroll_up(),
            },
            KeyCode::Char('h') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.status_message = self
                    .adjust_schema_panel_width(-4)
                    .unwrap_or_else(|err| format!("Failed to resize schema panel: {}", err));
            }
            KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.status_message = self
                    .adjust_schema_panel_width(4)
                    .unwrap_or_else(|err| format!("Failed to resize schema panel: {}", err));
            }
            KeyCode::Char('h') => {
                self.focused_pane = match self.focused_pane {
                    FocusedPane::Results => FocusedPane::Editor,
                    _ => self.focused_pane,
                };
            }
            KeyCode::Char('l') => {
                self.focused_pane = match self.focused_pane {
                    FocusedPane::Editor => FocusedPane::Results,
                    _ => self.focused_pane,
                };
            }
            KeyCode::Char('g') if key.modifiers.contains(KeyModifiers::NONE) => {
                self.results.jump_to_top();
            }
            KeyCode::Char('G') => {
                self.results.jump_to_bottom();
            }
            KeyCode::PageDown => match self.focused_pane {
                FocusedPane::Editor => {
                    for _ in 0..10 {
                        self.editor.move_down();
                    }
                }
                FocusedPane::Results => self.results.scroll_by(20),
            },
            KeyCode::PageUp => match self.focused_pane {
                FocusedPane::Editor => {
                    for _ in 0..10 {
                        self.editor.move_up();
                    }
                }
                FocusedPane::Results => self.results.scroll_back_by(20),
            },
            KeyCode::Home if self.focused_pane == FocusedPane::Results => {
                self.results.jump_to_top();
            }
            KeyCode::End if self.focused_pane == FocusedPane::Results => {
                self.results.jump_to_bottom();
            }
            KeyCode::Char('d')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.focused_pane == FocusedPane::Editor =>
            {
                if self.editor.delete_current_line() {
                    self.status_message = "Editor: current line deleted".to_string();
                } else {
                    self.status_message = "Editor: nothing to delete".to_string();
                }
            }
            KeyCode::Char('u')
                if key.modifiers.contains(KeyModifiers::CONTROL)
                    && self.focused_pane == FocusedPane::Editor =>
            {
                self.editor.clear();
                self.status_message = "Editor: cleared".to_string();
            }
            KeyCode::Char('+') | KeyCode::Char('=') if self.side_panel == SidePanel::Graph => {
                if self.graph_view.increase_depth() {
                    self.status_message = format!("Graph depth: {}", self.graph_view.depth());
                } else {
                    self.status_message = "Graph depth max: 3".to_string();
                }
            }
            KeyCode::Char('-') if self.side_panel == SidePanel::Graph => {
                if self.graph_view.decrease_depth() {
                    self.status_message = format!("Graph depth: {}", self.graph_view.depth());
                } else {
                    self.status_message = "Graph depth min: 1".to_string();
                }
            }
            KeyCode::Char('J') if self.side_panel != SidePanel::None => {
                match self.side_panel {
                    SidePanel::Schema => self.schema.scroll_down(),
                    SidePanel::Graph => self.graph_view.scroll_down(),
                    SidePanel::None => {}
                }
            }
            KeyCode::Char('K') if self.side_panel != SidePanel::None => {
                match self.side_panel {
                    SidePanel::Schema => self.schema.scroll_up(),
                    SidePanel::Graph => self.graph_view.scroll_up(),
                    SidePanel::None => {}
                }
            }
            KeyCode::Enter
                if self.side_panel == SidePanel::Graph
                    && key.modifiers.contains(KeyModifiers::SHIFT) =>
            {
                if self.graph_view.focus_selected_neighbor() {
                    self.status_message = "Graph focus updated".to_string();
                } else {
                    self.status_message = "No neighbor selected".to_string();
                }
            }

            KeyCode::Char('1') => self.focused_pane = FocusedPane::Editor,
            KeyCode::Char('2') => self.focused_pane = FocusedPane::Results,

            KeyCode::Tab => {
                self.focused_pane = match self.focused_pane {
                    FocusedPane::Editor => FocusedPane::Results,
                    FocusedPane::Results => FocusedPane::Editor,
                };
            }

            KeyCode::Enter => {
                self.execute_query()?;
            }

            KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(query) = self.history.previous() {
                    self.editor.set_content(query);
                }
            }
            KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(query) = self.history.next() {
                    self.editor.set_content(query);
                }
            }
            KeyCode::Char('t') => {
                let mode = self.results.cycle_mode();
                self.status_message = format!("Results mode: {:?}", mode);
            }

            _ => {}
        }
        Ok(())
    }

    fn handle_insert_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status_message = "Ready".to_string();
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.mode = Mode::Normal;
                self.execute_query()?;
            }
            KeyCode::Enter => {
                self.editor.insert_newline();
            }
            KeyCode::Backspace => {
                self.editor.delete_char();
            }
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.editor.delete_current_line() {
                    self.status_message = "Editor: current line deleted".to_string();
                } else {
                    self.status_message = "Editor: nothing to delete".to_string();
                }
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.editor.clear();
                self.status_message = "Editor: cleared".to_string();
            }
            KeyCode::Char(c) => {
                self.editor.insert_char(c);
            }
            KeyCode::Left => self.editor.move_left(),
            KeyCode::Right => self.editor.move_right(),
            KeyCode::Up => self.editor.move_up(),
            KeyCode::Down => self.editor.move_down(),
            _ => {}
        }
        Ok(())
    }

    fn handle_command_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.command_buffer.clear();
                self.status_message = "Ready".to_string();
            }
            KeyCode::Enter => {
                self.execute_command()?;
                if self.mode == Mode::Command {
                    self.mode = Mode::Normal;
                }
            }
            KeyCode::Tab => {
                if self.autocomplete_command()? {
                    self.status_message = format!(":{}", self.command_buffer);
                }
            }
            KeyCode::Backspace => {
                self.command_buffer.pop();
                self.status_message = format!(":{}", self.command_buffer);
            }
            KeyCode::Char(c) => {
                self.command_buffer.push(c);
                self.status_message = format!(":{}", self.command_buffer);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_visual_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('?') => {
                self.open_help_modal();
            }
            KeyCode::Esc => {
                self.mode = Mode::Normal;
                self.status_message = "Ready".to_string();
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_history_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('?') => {
                self.open_help_modal();
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
                self.status_message = "Ready".to_string();
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_session_mode(&mut self, key: KeyEvent) -> Result<()> {
        if self.session_browser.filter_editing {
            match key.code {
                KeyCode::Esc => {
                    self.session_browser.filter_editing = false;
                    self.status_message = "Session filter edit canceled".to_string();
                }
                KeyCode::Enter => {
                    self.session_browser.filter_editing = false;
                    self.status_message = format!(
                        "Session filter applied: {}",
                        self.session_browser.filter_text
                    );
                }
                KeyCode::Backspace => {
                    self.session_browser.filter_text.pop();
                    self.reset_session_browser_selection();
                    self.status_message =
                        format!("Filter: {}", self.session_browser.filter_text);
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.session_browser.filter_text.clear();
                    self.reset_session_browser_selection();
                    self.status_message = "Filter cleared".to_string();
                }
                KeyCode::Char(c) => {
                    self.session_browser.filter_text.push(c);
                    self.reset_session_browser_selection();
                    self.status_message =
                        format!("Filter: {}", self.session_browser.filter_text);
                }
                _ => {}
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Char('?') => {
                self.open_help_modal();
            }
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = Mode::Normal;
                self.status_message = "Ready".to_string();
            }
            KeyCode::Char('/') => {
                self.session_browser.filter_editing = true;
                self.status_message = format!("Filter: {}", self.session_browser.filter_text);
            }
            KeyCode::Tab => {
                self.session_browser.active = match self.session_browser.active {
                    SessionPane::Timeline => SessionPane::Snippets,
                    SessionPane::Snippets => SessionPane::Tabs,
                    SessionPane::Tabs => SessionPane::Timeline,
                };
            }
            KeyCode::Char('j') | KeyCode::Down => self.session_browser_select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.session_browser_select_prev(),
            KeyCode::Char('p') if self.session_browser.active == SessionPane::Timeline => {
                self.status_message = self
                    .toggle_selected_timeline_pin()
                    .unwrap_or_else(|err| format!("Failed to pin timeline item: {}", err));
            }
            KeyCode::Enter | KeyCode::Char('r') => {
                self.status_message = self
                    .run_selected_session_item()
                    .unwrap_or_else(|err| format!("Failed to run selected item: {}", err));
            }
            KeyCode::Char('l') => {
                self.status_message = self
                    .load_selected_session_item()
                    .unwrap_or_else(|err| format!("Failed to load selected item: {}", err));
            }
            KeyCode::Char('g') if self.session_browser.active == SessionPane::Timeline => {
                self.status_message = self
                    .show_selected_timeline_dag()
                    .unwrap_or_else(|err| format!("Failed to show selected DAG: {}", err));
            }
            KeyCode::Char('d') if self.session_browser.active == SessionPane::Timeline => {
                self.status_message = self
                    .show_selected_timeline_lineage()
                    .unwrap_or_else(|err| format!("Failed to show selected lineage: {}", err));
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_help_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('?') => {
                self.close_help_modal();
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let max = self.help_lines().len().saturating_sub(1);
                self.help_scroll = (self.help_scroll + 1).min(max);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
            }
            KeyCode::PageDown => {
                let max = self.help_lines().len().saturating_sub(1);
                self.help_scroll = (self.help_scroll + 8).min(max);
            }
            KeyCode::PageUp => {
                self.help_scroll = self.help_scroll.saturating_sub(8);
            }
            KeyCode::Home => {
                self.help_scroll = 0;
            }
            KeyCode::End => {
                self.help_scroll = self.help_lines().len().saturating_sub(1);
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_palette_mode(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.close_palette();
            }
            KeyCode::Backspace => {
                self.palette.query.pop();
                self.palette.selected = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                let max = self.palette_entries().len().saturating_sub(1);
                self.palette.selected = (self.palette.selected + 1).min(max);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.palette.selected = self.palette.selected.saturating_sub(1);
            }
            KeyCode::PageDown => {
                let max = self.palette_entries().len().saturating_sub(1);
                self.palette.selected = (self.palette.selected + 8).min(max);
            }
            KeyCode::PageUp => {
                self.palette.selected = self.palette.selected.saturating_sub(8);
            }
            KeyCode::Home => {
                self.palette.selected = 0;
            }
            KeyCode::End => {
                self.palette.selected = self.palette_entries().len().saturating_sub(1);
            }
            KeyCode::Enter => {
                self.apply_palette_selection()?;
            }
            KeyCode::Char(c) => {
                self.palette.query.push(c);
                self.palette.selected = 0;
            }
            _ => {}
        }
        Ok(())
    }

    fn execute_query(&mut self) -> Result<()> {
        self.execute_query_with_mode(RunMode::Run)
    }

    fn execute_query_with_mode(&mut self, run_mode: RunMode) -> Result<()> {
        if self.pending_query.is_some() {
            self.status_message = "Query already in progress...".to_string();
            return Ok(());
        }

        let query = self.editor.content().trim().to_string();
        let normalized_query = normalize_query_for_cache(&query);

        if query.is_empty() {
            self.status_message = "Empty query".to_string();
            return Ok(());
        }

        let cache_key = build_query_hash(
            &query,
            run_mode,
            self.db_revision,
            self.schema_revision,
            &self.session_state.active_parameters,
        );
        if let Some(cached) = self.cache.get(&cache_key, &normalized_query) {
            self.results
                .set_data(cached.headers.clone(), cached.rows.clone());
            self.results.set_execution_time(Duration::from_millis(0));
            self.last_cache_event = Some(CacheEvent::Hit);
            self.last_run_mode = Some(run_mode);
            self.last_run_summary = Some(format!("{} • cache hit", cached.summary));
            if self.session_v2_enabled {
                let touched = extract_query_entities(&query);
                self.session_state.record_success(
                    run_mode,
                    Some(CacheStatus::Hit),
                    ChangeKind::Read,
                    touched.labels,
                    touched.edge_types,
                    touched.properties,
                    &query,
                    &format!("{} • cache hit", cached.summary),
                    cached.row_count,
                    0.0,
                );
                self.persist_session_data();
            }
            self.history.add_query(query, true);
            self.status_message = format!("✓ {} • cache hit", run_mode_label(run_mode));
            return Ok(());
        }

        self.results.set_data(
            vec!["status".to_string()],
            vec![vec!["Query en progreso...".to_string()]],
        );
        self.results.clear_execution_time();
        self.last_cache_event = Some(CacheEvent::Miss);
        if self.session_v2_enabled {
            self.session_state.set_active_query_text(&query);
        }
        self.status_message = format!("{} en progreso...", run_mode_label(run_mode));
        self.start_query_job(query, run_mode, cache_key);

        Ok(())
    }

    fn execute_command(&mut self) -> Result<()> {
        let cmd = self.command_buffer.trim().to_string();

        match cmd.as_str() {
            "q" | "quit" => {
                self.persist_session_data();
                self.quit_requested = true;
                self.status_message = "Bye!".to_string();
            }
            "timeline" => {
                if self.session_v2_enabled {
                    self.timeline_visible = !self.timeline_visible;
                    self.status_message = if self.timeline_visible {
                        "Timeline shown".to_string()
                    } else {
                        "Timeline hidden".to_string()
                    };
                } else {
                    self.status_message =
                        "Session v2 disabled. Set NDBSTUDIO_SESSION_V2=1".to_string();
                }
            }
            "browser" => {
                if self.session_v2_enabled {
                    self.mode = Mode::Session;
                    self.status_message =
                        "Session Browser • /:filter(mode:run|explain|profile) • Tab:pane • j/k:move • Enter:run • l:load • p:pin • g:dag • d:detail".to_string();
                } else {
                    self.status_message =
                        "Session v2 disabled. Set NDBSTUDIO_SESSION_V2=1".to_string();
                }
            }
            "browser filter" => {
                self.status_message = format!("Session filter: {}", self.session_browser.filter_text);
            }
            "browser filter clear" => {
                self.status_message = self
                    .set_session_browser_filter("")
                    .unwrap_or_else(|err| format!("Failed to clear browser filter: {}", err));
            }
            _ if cmd.starts_with("browser filter ") => {
                let text = cmd.trim_start_matches("browser filter ").trim();
                self.status_message = self
                    .set_session_browser_filter(text)
                    .unwrap_or_else(|err| format!("Failed to set browser filter: {}", err));
            }
            _ if cmd.starts_with("browser mode ") => {
                let raw = cmd.trim_start_matches("browser mode ").trim();
                let mode = match raw.to_ascii_lowercase().as_str() {
                    "run" => "mode:run",
                    "explain" => "mode:explain",
                    "profile" => "mode:profile",
                    "all" | "*" => "",
                    _ => {
                        self.status_message =
                            "Unknown browser mode. Use: run | explain | profile | all".to_string();
                        self.command_buffer.clear();
                        return Ok(());
                    }
                };
                self.status_message = self
                    .set_session_browser_filter(mode)
                    .unwrap_or_else(|err| format!("Failed to set browser mode filter: {}", err));
            }
            "timeline rerun last" => {
                self.status_message = self
                    .rerun_last_timeline_query()
                    .unwrap_or_else(|err| format!("Failed to rerun query: {}", err));
            }
            _ if cmd.starts_with("timeline rerun dependents ") => {
                let idx_raw = cmd.trim_start_matches("timeline rerun dependents ").trim();
                self.status_message = self
                    .rerun_timeline_dependents(idx_raw)
                    .unwrap_or_else(|err| format!("Failed to rerun dependents: {}", err));
            }
            _ if cmd.starts_with("timeline rerun impacted ") => {
                let raw = cmd.trim_start_matches("timeline rerun impacted ").trim();
                self.status_message = self
                    .rerun_timeline_impacted(raw)
                    .unwrap_or_else(|err| format!("Failed to rerun impacted: {}", err));
            }
            _ if cmd.starts_with("timeline rerun ") => {
                let idx_raw = cmd.trim_start_matches("timeline rerun ").trim();
                self.status_message = self
                    .rerun_timeline_query(idx_raw)
                    .unwrap_or_else(|err| format!("Failed to rerun query: {}", err));
            }
            _ if cmd.starts_with("timeline lineage ") => {
                let idx_raw = cmd.trim_start_matches("timeline lineage ").trim();
                self.status_message = self
                    .show_timeline_lineage(idx_raw)
                    .unwrap_or_else(|err| format!("Failed to show lineage: {}", err));
            }
            _ if cmd.starts_with("timeline dag ") => {
                let idx_raw = cmd.trim_start_matches("timeline dag ").trim();
                self.status_message = self
                    .show_timeline_dag(idx_raw)
                    .unwrap_or_else(|err| format!("Failed to show timeline DAG: {}", err));
            }
            _ if cmd.starts_with("timeline impact ") => {
                let raw = cmd.trim_start_matches("timeline impact ").trim();
                self.status_message = self
                    .show_timeline_impact(raw)
                    .unwrap_or_else(|err| format!("Failed to show timeline impact: {}", err));
            }
            _ if cmd.starts_with("timeline pin ") => {
                let idx_raw = cmd.trim_start_matches("timeline pin ").trim();
                self.status_message = self
                    .toggle_timeline_pin(idx_raw)
                    .unwrap_or_else(|err| format!("Failed to pin query: {}", err));
            }
            _ if cmd.starts_with("tab new") => {
                let title = cmd.trim_start_matches("tab new").trim();
                let title_opt = if title.is_empty() { None } else { Some(title) };
                self.status_message = self
                    .create_new_tab(title_opt)
                    .unwrap_or_else(|err| format!("Failed to create tab: {}", err));
            }
            "tab next" => {
                self.status_message = self
                    .activate_next_tab()
                    .unwrap_or_else(|err| format!("Failed to switch tab: {}", err));
            }
            "tab prev" => {
                self.status_message = self
                    .activate_prev_tab()
                    .unwrap_or_else(|err| format!("Failed to switch tab: {}", err));
            }
            "tab close" => {
                self.status_message = self
                    .close_active_tab()
                    .unwrap_or_else(|err| format!("Failed to close tab: {}", err));
            }
            "tabs" => {
                self.status_message = self
                    .show_tabs()
                    .unwrap_or_else(|err| format!("Failed to list tabs: {}", err));
            }
            _ if cmd.starts_with("save ") => {
                let name = cmd.trim_start_matches("save ").trim();
                self.status_message = self
                    .save_snippet(name)
                    .unwrap_or_else(|err| format!("Failed to save snippet: {}", err));
            }
            "snippets" => {
                self.status_message = self
                    .show_snippets()
                    .unwrap_or_else(|err| format!("Failed to list snippets: {}", err));
            }
            _ if cmd.starts_with("snippet run ") => {
                let name = cmd.trim_start_matches("snippet run ").trim();
                self.status_message = self
                    .load_snippet_to_editor(name)
                    .unwrap_or_else(|err| format!("Failed to load snippet: {}", err));
            }
            "session" => {
                if self.session_v2_enabled {
                    self.status_message = format!("Session v2: {}", session_summary(&self.session_state));
                } else {
                    self.status_message =
                        "Session v2 disabled. Set NDBSTUDIO_SESSION_V2=1".to_string();
                }
            }
            "cache stats" => {
                self.status_message = self.cache_stats_message();
            }
            _ if cmd.starts_with("cache stats ") => {
                let scope = cmd.trim_start_matches("cache stats ").trim();
                self.status_message = self
                    .cache_stats_scope_message(scope)
                    .unwrap_or_else(|err| format!("Failed to read cache stats scope: {}", err));
            }
            _ if cmd.starts_with("cache recent") => {
                let raw = cmd.trim_start_matches("cache recent").trim();
                self.status_message = self
                    .cache_recent_message(raw)
                    .unwrap_or_else(|err| format!("Failed to read recent cache stats: {}", err));
            }
            "cache clear" => {
                let removed = self.cache.len();
                self.cache.clear();
                self.last_cache_event = None;
                self.status_message = format!("Cache cleared ({} entries)", removed);
            }
            "params" => {
                self.status_message = self.list_params_message();
            }
            _ if cmd.starts_with("param set ") => {
                let raw = cmd.trim_start_matches("param set ").trim();
                self.status_message = self
                    .set_param_command(raw)
                    .unwrap_or_else(|err| format!("Failed to set param: {}", err));
            }
            _ if cmd.starts_with("param unset ") => {
                let key = cmd.trim_start_matches("param unset ").trim();
                self.status_message = self
                    .unset_param_command(key)
                    .unwrap_or_else(|err| format!("Failed to unset param: {}", err));
            }
            "params clear" => {
                self.status_message = self
                    .clear_params_command()
                    .unwrap_or_else(|err| format!("Failed to clear params: {}", err));
            }
            "run" => {
                self.status_message = "Run en progreso...".to_string();
                self.execute_query_with_mode(RunMode::Run)?;
            }
            "explain" => {
                self.status_message = "Explain en progreso...".to_string();
                self.execute_query_with_mode(RunMode::Explain)?;
            }
            "profile" => {
                self.status_message = "Profile en progreso...".to_string();
                self.execute_query_with_mode(RunMode::Profile)?;
            }
            "schema" => {
                self.status_message = self
                    .toggle_side_panel(SidePanel::Schema)
                    .unwrap_or_else(|err| format!("Failed to toggle schema panel: {}", err));
            }
            _ if cmd.starts_with("graph label ") => {
                let raw = cmd.trim_start_matches("graph label ").trim();
                let label = if raw == "*" || raw.eq_ignore_ascii_case("all") {
                    None
                } else {
                    Some(raw.to_string())
                };
                self.status_message = self
                    .set_graph_label_filter(label)
                    .unwrap_or_else(|err| format!("Failed to apply graph label filter: {}", err));
            }
            "graph labels" => {
                self.status_message = self
                    .show_graph_labels()
                    .unwrap_or_else(|err| format!("Failed to show graph labels: {}", err));
            }
            _ if cmd.starts_with("graph focus name ") => {
                let raw = cmd.trim_start_matches("graph focus name ").trim();
                let name = parse_command_value(raw);
                self.status_message = self
                    .focus_graph_by_name(&name)
                    .unwrap_or_else(|err| format!("Failed to focus graph node by name: {}", err));
            }
            _ if cmd.starts_with("graph focus ") => {
                let node_id = cmd.trim_start_matches("graph focus ").trim();
                self.status_message = self
                    .focus_graph_node(node_id)
                    .unwrap_or_else(|err| format!("Failed to focus graph node: {}", err));
            }
            "graph refresh" => {
                self.status_message = self
                    .refresh_graph_view()
                    .map(|_| "Graph view refreshed".to_string())
                    .unwrap_or_else(|err| format!("Failed to refresh graph view: {}", err));
            }
            "graph" => {
                self.status_message = self
                    .toggle_side_panel(SidePanel::Graph)
                    .unwrap_or_else(|err| format!("Failed to toggle graph panel: {}", err));
            }
            "history" => {
                self.mode = Mode::History;
            }
            "clear" | "editor clear" => {
                self.editor.clear();
                self.status_message = "Editor: cleared".to_string();
            }
            "editor delete-line" | "editor delline" => {
                if self.editor.delete_current_line() {
                    self.status_message = "Editor: current line deleted".to_string();
                } else {
                    self.status_message = "Editor: nothing to delete".to_string();
                }
            }
            _ if cmd.starts_with("results ") => {
                let mode = cmd.trim_start_matches("results ").trim();
                if self.results.set_mode_from_name(mode) {
                    self.status_message = format!("Results mode: {}", mode);
                } else {
                    self.status_message =
                        "Unknown results mode. Use: table | json | graph | plan".to_string();
                }
            }
            "help" => {
                self.open_help_modal();
            }
            _ if cmd.starts_with("export ") => {
                let format = cmd.strip_prefix("export ").unwrap_or_default();
                self.export_results(format)?;
            }
            _ => {
                self.status_message = format!("Unknown command: {}", cmd);
            }
        }

        self.command_buffer.clear();
        Ok(())
    }

    fn export_results(&mut self, format: &str) -> Result<()> {
        match format {
            "csv" | "json" | "arrow" => {
                self.status_message = format!("Exported to {}", format);
                // TODO: implement file exports
            }
            _ => {
                self.status_message = format!("Unknown format: {}", format);
            }
        }
        Ok(())
    }

    fn refresh_schema_view(&mut self) -> Result<()> {
        let snapshot = self
            .runtime
            .block_on(workbench::build_schema_snapshot(&self.graph, &self.db_path))?;

        self.db_info = format!(
            "{} • {} nodes • {} edges",
            snapshot.db_name,
            format_count(snapshot.total_nodes),
            format_count(snapshot.total_edges)
        );

        let mut items = Vec::new();
        items.push(format!("Nodes ({} types)", snapshot.node_types.len()));

        for node_type in &snapshot.node_types {
            items.push(format!(
                "  {} ({})",
                node_type.name,
                format_count(node_type.count)
            ));
            for prop in &node_type.properties {
                items.push(format!("     • {}", prop));
            }
        }

        items.push(String::new());
        items.push(format!("Edges ({} types)", snapshot.edge_types.len()));

        for edge_type in &snapshot.edge_types {
            items.push(format!(
                "  {} ({})",
                edge_type.name,
                format_count(edge_type.count)
            ));
            for prop in &edge_type.properties {
                items.push(format!("     • {}", prop));
            }
        }

        items.push(String::new());
        items.push("Statistics".to_string());
        items.push(format!(
            "  Total nodes:  {}",
            format_count(snapshot.total_nodes)
        ));
        items.push(format!(
            "  Total edges:  {}",
            format_count(snapshot.total_edges)
        ));
        items.push(format!("  Avg degree:   {:.2}", snapshot.avg_degree));
        items.push(format!("  Density:      {:.6}", snapshot.density));

        self.schema.set_items(items);
        Ok(())
    }

    fn start_query_job(&mut self, query: String, run_mode: RunMode, cache_key: String) {
        let (tx, rx) = mpsc::channel();
        let graph = self.graph.clone();
        let query_for_thread = query.clone();
        let cache_key_for_thread = cache_key.clone();

        thread::spawn(move || {
            let result = (|| -> Result<QueryJobResult> {
                let runtime = Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .context("failed to create async runtime for query execution")?;
                let result = runtime.block_on(workbench::execute_query(
                    &graph,
                    &QueryRunRequest {
                        query: query_for_thread.clone(),
                        run_mode,
                    },
                ))?;

                Ok(QueryJobResult {
                    cache_key: cache_key_for_thread,
                    result,
                })
            })();

            let _ = tx.send(result);
        });

        self.pending_query = Some(PendingQuery {
            query,
            run_mode,
            started_at: Instant::now(),
            receiver: rx,
        });
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_success(
        &mut self,
        run_mode: RunMode,
        change_kind: ChangeKind,
        touched_labels: Vec<String>,
        touched_edge_types: Vec<String>,
        touched_properties: Vec<String>,
        query: String,
        summary: String,
        elapsed: std::time::Duration,
    ) {
        self.history.add_query(query.clone(), true);
        self.results.set_execution_time(elapsed);
        if self.session_v2_enabled {
            self.session_state.record_success(
                run_mode,
                Some(CacheStatus::Miss),
                change_kind,
                touched_labels,
                touched_edge_types,
                touched_properties,
                &query,
                &summary,
                self.results.row_count(),
                elapsed.as_secs_f64() * 1000.0,
            );
            self.persist_session_data();
        }
        self.status_message = format!(
            "✓ {} • {} • {:.1}ms{}",
            run_mode_label(run_mode),
            summary,
            elapsed.as_secs_f64() * 1000.0,
            if self.last_cache_event == Some(CacheEvent::Miss) {
                " • cache miss"
            } else {
                ""
            }
        );
        self.last_run_mode = Some(run_mode);
        self.last_run_summary = Some(summary);
    }

    #[allow(clippy::too_many_arguments)]
    fn handle_failure(
        &mut self,
        run_mode: RunMode,
        change_kind: ChangeKind,
        touched_labels: Vec<String>,
        touched_edge_types: Vec<String>,
        touched_properties: Vec<String>,
        query: String,
        err: anyhow::Error,
        elapsed: Option<std::time::Duration>,
    ) {
        let details = format_error_chain(&err);
        let rows = details
            .lines()
            .map(|line| vec![line.to_string()])
            .collect::<Vec<_>>();
        self.results.set_data(vec!["error".to_string()], rows);
        if let Some(elapsed) = elapsed {
            self.results.set_execution_time(elapsed);
        } else {
            self.results.clear_execution_time();
        }
        self.history.add_query(query.clone(), false);
        if self.session_v2_enabled {
            self.session_state.record_failure(
                run_mode,
                Some(CacheStatus::Miss),
                change_kind,
                touched_labels,
                touched_edge_types,
                touched_properties,
                &query,
                &details,
                elapsed.map(|d| d.as_secs_f64() * 1000.0),
            );
            self.persist_session_data();
        }
        let headline = err.to_string();
        self.status_message = format!("✗ {}: {}", run_mode_label(run_mode), headline);
        self.last_run_mode = Some(run_mode);
        self.last_run_summary = Some(details);
    }

    fn load_schema_browser(&mut self) -> Result<String> {
        self.refresh_schema_view()
            .context("failed to refresh schema view")?;
        Ok("Schema browser".to_string())
    }

    fn toggle_side_panel(&mut self, panel: SidePanel) -> Result<String> {
        if self.side_panel == panel {
            self.side_panel = SidePanel::None;
            self.save_ui_prefs()?;
            return Ok("Side panel hidden".to_string());
        }

        let message = match panel {
            SidePanel::Schema => {
                self.load_schema_browser()?;
                "Schema panel shown"
            }
            SidePanel::Graph => {
                self.refresh_graph_view()?;
                "Graph panel shown"
            }
            SidePanel::None => "Side panel hidden",
        };

        self.side_panel = panel;
        self.save_ui_prefs()?;
        Ok(message.to_string())
    }

    fn adjust_schema_panel_width(&mut self, delta: i16) -> Result<String> {
        if self.side_panel == SidePanel::None {
            return Ok("Side panel is hidden (press 's' or 'x' to show)".to_string());
        }

        let min_width = 20i16;
        let max_width = 45i16;
        let current = self.side_panel_width as i16;
        let next = (current + delta).clamp(min_width, max_width) as u16;
        self.side_panel_width = next;
        self.save_ui_prefs()?;
        Ok(format!("Side panel width: {}%", self.side_panel_width))
    }

    fn sync_schema_after_write(&mut self) -> Result<()> {
        self.runtime.block_on(self.graph.rebuild_schema())?;
        self.refresh_db_info()?;
        match self.side_panel {
            SidePanel::Schema => self.refresh_schema_view()?,
            SidePanel::Graph => self.refresh_graph_view()?,
            SidePanel::None => {}
        }
        Ok(())
    }

    fn refresh_db_info(&mut self) -> Result<()> {
        let stats = self.runtime.block_on(self.graph.get_stats())?;
        self.db_info = format!(
            "{} • {} nodes • {} edges",
            database_name(&self.db_path),
            format_count(stats.total_nodes),
            format_count(stats.total_edges)
        );
        Ok(())
    }

    pub fn tick(&mut self) -> Result<()> {
        self.poll_pending_query()?;
        self.maybe_run_next_queued()?;
        Ok(())
    }

    fn poll_pending_query(&mut self) -> Result<()> {
        let Some(pending) = self.pending_query.as_ref() else {
            return Ok(());
        };

        match pending.receiver.try_recv() {
            Ok(Ok(job)) => {
                let pending = self
                    .pending_query
                    .take()
                    .expect("pending query should exist when receiving result");
                let row_count = job.result.rows.len();
                self.results
                    .set_data(job.result.headers.clone(), job.result.rows.clone());

                match job.result.invalidation {
                    QueryInvalidation::None => {
                        self.cache.put(
                            job.cache_key.clone(),
                            CachedQueryResult {
                                summary: job.result.summary.clone(),
                                normalized_query: normalize_query_for_cache(&pending.query),
                                headers: job.result.headers.clone(),
                                rows: job.result.rows.clone(),
                                row_count,
                            },
                        );
                        self.refresh_db_info()
                            .context("failed to refresh database stats")?;
                        if self.side_panel == SidePanel::Graph {
                            let _ = self.refresh_graph_view();
                        }
                    }
                    QueryInvalidation::Data => {
                        self.db_revision = self.db_revision.saturating_add(1);
                        self.sync_schema_after_write()
                            .context("schema synchronization failed after write")?;
                    }
                    QueryInvalidation::Schema => {
                        self.db_revision = self.db_revision.saturating_add(1);
                        self.schema_revision = self.schema_revision.saturating_add(1);
                        self.sync_schema_after_write()
                            .context("schema synchronization failed after schema write")?;
                    }
                }

                self.handle_success(
                    job.result.run_mode,
                    workbench::map_invalidation_to_change_kind(job.result.invalidation),
                    job.result.touched_labels,
                    job.result.touched_edge_types,
                    job.result.touched_properties,
                    pending.query,
                    job.result.summary,
                    pending.started_at.elapsed(),
                );
            }
            Ok(Err(err)) => {
                let pending = self
                    .pending_query
                    .take()
                    .expect("pending query should exist when receiving error");
                let touched = extract_query_entities(&pending.query);
                self.handle_failure(
                    pending.run_mode,
                    classify_change_kind_from_query(&pending.query),
                    touched.labels,
                    touched.edge_types,
                    touched.properties,
                    pending.query,
                    err,
                    Some(pending.started_at.elapsed()),
                );
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                let pending = self
                    .pending_query
                    .take()
                    .expect("pending query should exist when channel disconnects");
                let touched = extract_query_entities(&pending.query);
                self.handle_failure(
                    pending.run_mode,
                    classify_change_kind_from_query(&pending.query),
                    touched.labels,
                    touched.edge_types,
                    touched.properties,
                    pending.query,
                    anyhow::anyhow!("query worker disconnected unexpectedly"),
                    Some(pending.started_at.elapsed()),
                );
            }
        }

        Ok(())
    }

    fn load_ui_prefs(&mut self) -> Result<()> {
        let path = ui_prefs_path().context("failed to resolve UI preferences path")?;
        if !path.exists() {
            return Ok(());
        }

        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("failed to read UI preferences from {}", path.display()))?;
        let prefs: UiPrefs = serde_json::from_str(&raw)
            .with_context(|| format!("failed to parse UI preferences from {}", path.display()))?;

        self.side_panel = prefs.side_panel;
        self.side_panel_width = prefs.side_panel_width.clamp(20, 45);

        match self.side_panel {
            SidePanel::Schema => self.refresh_schema_view()?,
            SidePanel::Graph => self.refresh_graph_view()?,
            SidePanel::None => {}
        }

        Ok(())
    }

    fn save_ui_prefs(&self) -> Result<()> {
        let path = ui_prefs_path().context("failed to resolve UI preferences path")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create UI preferences directory {}",
                    parent.display()
                )
            })?;
        }

        let prefs = UiPrefs {
            side_panel: self.side_panel,
            side_panel_width: self.side_panel_width,
        };

        let raw = serde_json::to_string_pretty(&prefs)
            .context("failed to serialize UI preferences")?;
        std::fs::write(&path, raw)
            .with_context(|| format!("failed to write UI preferences to {}", path.display()))?;
        Ok(())
    }

    fn load_session_data(&mut self) -> Result<()> {
        let path = default_session_path().context("failed to resolve session path")?;
        if !path.exists() {
            return Ok(());
        }

        let loaded = load_session_state(&path)?;
        if loaded.db_path != self.db_path {
            return Ok(());
        }

        if let Some(tab) = loaded.active_tab() {
            self.editor.set_content(tab.query_text.clone());
        }
        self.session_state = loaded;
        Ok(())
    }

    fn persist_session_data(&mut self) {
        if !self.session_v2_enabled {
            return;
        }

        self.session_state.set_active_query_text(&self.editor.content());
        let path = match default_session_path() {
            Ok(p) => p,
            Err(_) => return,
        };
        let _ = save_session_state(&path, &self.session_state);
    }

    fn create_new_tab(&mut self, title: Option<&str>) -> Result<String> {
        self.ensure_session_v2()?;
        self.session_state.set_active_query_text(&self.editor.content());
        self.session_state.create_tab(title);
        if let Some(tab) = self.session_state.active_tab() {
            let tab_title = tab.title.clone();
            let tab_query = tab.query_text.clone();
            self.editor.set_content(tab_query);
            self.persist_session_data();
            return Ok(format!("Tab created: {}", tab_title));
        }
        Ok("Tab created".to_string())
    }

    fn activate_next_tab(&mut self) -> Result<String> {
        self.ensure_session_v2()?;
        self.session_state.set_active_query_text(&self.editor.content());
        self.session_state.activate_next_tab();
        if let Some(tab) = self.session_state.active_tab() {
            let tab_title = tab.title.clone();
            let tab_query = tab.query_text.clone();
            self.editor.set_content(tab_query);
            self.persist_session_data();
            return Ok(format!("Tab: {}", tab_title));
        }
        Ok("Tab switched".to_string())
    }

    fn activate_prev_tab(&mut self) -> Result<String> {
        self.ensure_session_v2()?;
        self.session_state.set_active_query_text(&self.editor.content());
        self.session_state.activate_prev_tab();
        if let Some(tab) = self.session_state.active_tab() {
            let tab_title = tab.title.clone();
            let tab_query = tab.query_text.clone();
            self.editor.set_content(tab_query);
            self.persist_session_data();
            return Ok(format!("Tab: {}", tab_title));
        }
        Ok("Tab switched".to_string())
    }

    fn close_active_tab(&mut self) -> Result<String> {
        self.ensure_session_v2()?;
        self.session_state.set_active_query_text(&self.editor.content());
        if !self.session_state.close_active_tab() {
            return Ok("Cannot close last tab".to_string());
        }
        if let Some(tab) = self.session_state.active_tab() {
            let tab_title = tab.title.clone();
            let tab_query = tab.query_text.clone();
            self.editor.set_content(tab_query);
            self.persist_session_data();
            return Ok(format!("Closed tab. Active: {}", tab_title));
        }
        Ok("Tab closed".to_string())
    }

    fn show_tabs(&mut self) -> Result<String> {
        self.ensure_session_v2()?;
        let rows = self
            .session_state
            .tabs
            .iter()
            .enumerate()
            .map(|(idx, tab)| {
                let marker = if tab.id == self.session_state.active_tab_id {
                    "*"
                } else {
                    " "
                };
                vec![
                    marker.to_string(),
                    (idx + 1).to_string(),
                    tab.title.clone(),
                    tab.last_executed_at
                        .clone()
                        .unwrap_or_else(|| "-".to_string()),
                ]
            })
            .collect::<Vec<_>>();
        self.results.set_data(
            vec![
                "active".to_string(),
                "idx".to_string(),
                "title".to_string(),
                "last_run".to_string(),
            ],
            rows,
        );
        Ok(format!("{} tabs", self.session_state.tabs.len()))
    }

    fn save_snippet(&mut self, name: &str) -> Result<String> {
        self.ensure_session_v2()?;
        if !self.session_state.save_query(name, &self.editor.content()) {
            return Ok("Usage: :save <name> (editor must not be empty)".to_string());
        }
        self.persist_session_data();
        Ok(format!("Snippet saved: {}", name.trim()))
    }

    fn show_snippets(&mut self) -> Result<String> {
        self.ensure_session_v2()?;
        let rows = self
            .session_state
            .saved_queries
            .iter()
            .map(|q| {
                let preview = if q.query.len() > 70 {
                    format!("{}...", &q.query[..70])
                } else {
                    q.query.clone()
                };
                vec![q.name.clone(), preview]
            })
            .collect::<Vec<_>>();
        self.results
            .set_data(vec!["name".to_string(), "query".to_string()], rows);
        Ok(format!("{} snippets", self.session_state.saved_queries.len()))
    }

    fn load_snippet_to_editor(&mut self, name: &str) -> Result<String> {
        self.ensure_session_v2()?;
        let Some(saved) = self.session_state.find_saved_query(name) else {
            return Ok(format!("Snippet not found: {}", name));
        };
        self.editor.set_content(saved.query.clone());
        Ok(format!("Snippet loaded: {}", saved.name))
    }

    fn rerun_last_timeline_query(&mut self) -> Result<String> {
        self.ensure_session_v2()?;
        let Some(last) = self.session_state.timeline.last() else {
            return Ok("Timeline is empty".to_string());
        };
        self.editor.set_content(last.query.clone());
        let run_mode = last.run_mode;
        self.execute_query_with_mode(run_mode)?;
        Ok(format!(
            "Rerunning last timeline query [{}]",
            run_mode_label(run_mode)
        ))
    }

    fn rerun_timeline_query(&mut self, index_raw: &str) -> Result<String> {
        self.ensure_session_v2()?;
        let trimmed = index_raw.trim();
        let (idx_part, mode_override) = if let Some((left, right)) = trimmed.split_once("--as") {
            (
                left.trim(),
                Some(
                    parse_run_mode(right.trim())
                        .ok_or_else(|| anyhow::anyhow!("invalid --as mode (run|explain|profile)"))?,
                ),
            )
        } else {
            (trimmed, None)
        };

        let idx = idx_part
            .trim()
            .parse::<usize>()
            .context("timeline index must be a number")?;
        if idx == 0 {
            return Ok("Timeline index starts at 1".to_string());
        }
        let recent = self.session_state.recent_timeline(100);
        let Some(entry) = recent.get(idx - 1) else {
            return Ok(format!("Timeline entry {} not found", idx));
        };
        let query = entry.query.clone();
        let run_mode = mode_override.unwrap_or(entry.run_mode);
        self.editor.set_content(query);
        self.execute_query_with_mode(run_mode)?;
        Ok(format!(
            "Rerunning timeline entry {} [{}]",
            idx,
            run_mode_label(run_mode)
        ))
    }

    fn rerun_timeline_dependents(&mut self, index_raw: &str) -> Result<String> {
        self.ensure_session_v2()?;
        let idx = index_raw
            .trim()
            .parse::<usize>()
            .context("timeline index must be a number")?;
        if idx == 0 {
            return Ok("Timeline index starts at 1".to_string());
        }

        let dependent_runs_scored = self
            .session_state
            .impacted_dependent_queries_scored_for_recent(idx - 1, 300);
        let mut dependent_runs = dependent_runs_scored
            .into_iter()
            .map(|run| (run.query, run.run_mode))
            .collect::<Vec<_>>();
        if dependent_runs.is_empty() {
            dependent_runs = self
                .session_state
                .dependent_queries_for_recent(idx - 1, 300)
                .into_iter()
                .collect::<Vec<_>>();
        }
        if dependent_runs.is_empty() {
            return Ok(format!("No dependent runs for timeline entry {}", idx));
        }

        for (query, run_mode) in dependent_runs {
            self.rerun_queue.push_back(QueuedRun { query, run_mode });
        }
        self.maybe_run_next_queued()?;
        Ok(format!(
            "Queued dependent reruns from timeline entry {} ({} queued)",
            idx,
            self.rerun_queue.len()
        ))
    }

    fn rerun_timeline_impacted(&mut self, raw: &str) -> Result<String> {
        self.ensure_session_v2()?;
        let (idx_raw, threshold) = parse_index_with_threshold(raw)?;
        let idx = idx_raw
            .trim()
            .parse::<usize>()
            .context("timeline index must be a number")?;
        if idx == 0 {
            return Ok("Timeline index starts at 1".to_string());
        }

        let impacted = self
            .session_state
            .impacted_dependent_queries_scored_for_recent(idx - 1, 300);
        let filtered = impacted
            .into_iter()
            .filter(|run| run.impact_score >= threshold)
            .collect::<Vec<_>>();
        if filtered.is_empty() {
            return Ok(format!(
                "No impacted dependents above threshold {} for entry {}",
                threshold, idx
            ));
        }
        for run in filtered {
            self.rerun_queue.push_back(QueuedRun {
                query: run.query,
                run_mode: run.run_mode,
            });
        }
        self.maybe_run_next_queued()?;
        Ok(format!(
            "Queued impacted reruns from entry {} (threshold={} queued={})",
            idx,
            threshold,
            self.rerun_queue.len()
        ))
    }

    fn show_timeline_lineage(&mut self, index_raw: &str) -> Result<String> {
        self.ensure_session_v2()?;
        let idx = index_raw
            .trim()
            .parse::<usize>()
            .context("timeline index must be a number")?;
        if idx == 0 {
            return Ok("Timeline index starts at 1".to_string());
        }
        Ok(self
            .session_state
            .lineage_summary_for_recent(idx - 1, 300)
            .unwrap_or_else(|| format!("Timeline entry {} not found", idx)))
    }

    fn show_timeline_dag(&mut self, index_raw: &str) -> Result<String> {
        self.ensure_session_v2()?;
        let idx = index_raw
            .trim()
            .parse::<usize>()
            .context("timeline index must be a number")?;
        if idx == 0 {
            return Ok("Timeline index starts at 1".to_string());
        }
        let recent = self.session_state.recent_timeline(300);
        let Some(entry) = recent.get(idx - 1) else {
            return Ok(format!("Timeline entry {} not found", idx));
        };
        let target_id = entry.id.clone();
        let mut rows = Vec::new();
        for edge in &self.session_state.query_graph.edges {
            if edge.from_run_id == target_id || edge.to_run_id == target_id {
                rows.push(vec![
                    edge.from_run_id.clone(),
                    edge.to_run_id.clone(),
                    format!("{:?}", edge.reason),
                ]);
            }
        }
        if rows.is_empty() {
            rows.push(vec![
                target_id.clone(),
                target_id,
                "self".to_string(),
            ]);
        }
        self.results
            .set_data(vec!["source".to_string(), "target".to_string(), "type".to_string()], rows);
        let _ = self.results.set_mode_from_name("graph");
        Ok(format!("Timeline DAG around entry {}", idx))
    }

    fn show_timeline_impact(&mut self, raw: &str) -> Result<String> {
        self.ensure_session_v2()?;
        let (idx_raw, threshold) = parse_index_with_threshold(raw)?;
        let idx = idx_raw
            .trim()
            .parse::<usize>()
            .context("timeline index must be a number")?;
        if idx == 0 {
            return Ok("Timeline index starts at 1".to_string());
        }

        let impacted = self
            .session_state
            .impacted_dependent_queries_scored_for_recent(idx - 1, 300)
            .into_iter()
            .filter(|run| run.impact_score >= threshold)
            .collect::<Vec<_>>();

        let rows = if impacted.is_empty() {
            vec![vec![
                "-".to_string(),
                "0".to_string(),
                "-".to_string(),
                "(no impacted runs)".to_string(),
            ]]
        } else {
            impacted
                .iter()
                .map(|run| {
                    vec![
                        run.run_id.clone(),
                        run.impact_score.to_string(),
                        run_mode_label(run.run_mode).to_string(),
                        truncate_one_line(&run.query, 80),
                    ]
                })
                .collect::<Vec<_>>()
        };

        self.results.set_data(
            vec![
                "run_id".to_string(),
                "impact_score".to_string(),
                "mode".to_string(),
                "query".to_string(),
            ],
            rows,
        );
        let _ = self.results.set_mode_from_name("table");
        Ok(format!(
            "Timeline impact view for entry {} (threshold={})",
            idx, threshold
        ))
    }

    fn maybe_run_next_queued(&mut self) -> Result<()> {
        if self.pending_query.is_some() {
            return Ok(());
        }
        let Some(next) = self.rerun_queue.pop_front() else {
            return Ok(());
        };
        self.editor.set_content(next.query);
        self.execute_query_with_mode(next.run_mode)
    }

    fn toggle_timeline_pin(&mut self, index_raw: &str) -> Result<String> {
        self.ensure_session_v2()?;
        let idx = index_raw
            .trim()
            .parse::<usize>()
            .context("timeline index must be a number")?;
        if idx == 0 {
            return Ok("Timeline index starts at 1".to_string());
        }
        if !self.session_state.toggle_recent_timeline_pin(idx - 1) {
            return Ok(format!("Timeline entry {} not found", idx));
        }
        self.persist_session_data();
        Ok(format!("Toggled pin for timeline entry {}", idx))
    }

    fn toggle_selected_timeline_pin(&mut self) -> Result<String> {
        self.ensure_session_v2()?;
        let Some(recent_idx) = self
            .filtered_timeline_indices(200)
            .get(self.session_browser.timeline_idx)
            .copied()
        else {
            return Ok("No timeline item selected".to_string());
        };
        if !self.session_state.toggle_recent_timeline_pin(recent_idx) {
            return Ok("No timeline item selected".to_string());
        };
        self.persist_session_data();
        Ok("Toggled pin for selected timeline item".to_string())
    }

    fn ensure_session_v2(&self) -> Result<()> {
        if self.session_v2_enabled {
            Ok(())
        } else {
            anyhow::bail!("Session v2 disabled. Set NDBSTUDIO_SESSION_V2=1")
        }
    }

    fn cache_stats_message(&self) -> String {
        format!(
            "Cache {} / {} • hits={} misses={} db_rev={} schema_rev={} params={}",
            self.cache.len(),
            self.cache.capacity(),
            self.cache.hits(),
            self.cache.misses(),
            self.db_revision,
            self.schema_revision,
            self.session_state.active_parameters.len()
        )
    }

    fn session_cache_hit_rate(&self) -> Option<f64> {
        compute_hit_rate(
            self.session_state
                .timeline
                .iter()
                .map(|entry| entry.cache_status),
        )
    }

    fn active_tab_cache_hit_rate(&self) -> Option<f64> {
        let active_tab = self.session_state.active_tab_id.clone();
        compute_hit_rate(
            self.session_state
                .timeline
                .iter()
                .filter(|entry| entry.tab_id.as_deref() == Some(active_tab.as_str()))
                .map(|entry| entry.cache_status),
        )
    }

    fn cache_stats_scope_message(&self, scope: &str) -> Result<String> {
        match scope.trim().to_ascii_lowercase().as_str() {
            "session" => {
                let rate = self.session_cache_hit_rate().unwrap_or(0.0) * 100.0;
                Ok(format!("Cache hit-rate session: {:.1}%", rate))
            }
            "tab" => {
                let title = self.active_tab_title();
                let rate = self.active_tab_cache_hit_rate().unwrap_or(0.0) * 100.0;
                Ok(format!("Cache hit-rate tab [{}]: {:.1}%", title, rate))
            }
            _ => anyhow::bail!("usage: :cache stats [session|tab]"),
        }
    }

    fn recent_cache_hit_rate(&self, limit: usize, tab_only: bool) -> Option<f64> {
        let active_tab = self.session_state.active_tab_id.clone();
        let iter = self
            .session_state
            .timeline
            .iter()
            .rev()
            .filter(|entry| !tab_only || entry.tab_id.as_deref() == Some(active_tab.as_str()))
            .take(limit)
            .map(|entry| entry.cache_status);
        compute_hit_rate(iter)
    }

    fn cache_recent_message(&self, raw: &str) -> Result<String> {
        let trimmed = raw.trim();
        let (scope, n) = if trimmed.is_empty() {
            ("session", 20usize)
        } else {
            let mut parts = trimmed.split_whitespace();
            let first = parts.next().unwrap_or("session");
            let second = parts.next();
            let n = second
                .map(|v| v.parse::<usize>())
                .transpose()
                .context("N must be numeric")?
                .unwrap_or(20)
                .clamp(1, 500);
            (first, n)
        };

        match scope.to_ascii_lowercase().as_str() {
            "session" => {
                let rate = self.recent_cache_hit_rate(n, false).unwrap_or(0.0) * 100.0;
                Ok(format!("Cache hit-rate recent {} (session): {:.1}%", n, rate))
            }
            "tab" => {
                let rate = self.recent_cache_hit_rate(n, true).unwrap_or(0.0) * 100.0;
                Ok(format!(
                    "Cache hit-rate recent {} (tab [{}]): {:.1}%",
                    n,
                    self.active_tab_title(),
                    rate
                ))
            }
            _ => anyhow::bail!("usage: :cache recent [session|tab] [N]"),
        }
    }

    fn list_params_message(&self) -> String {
        if self.session_state.active_parameters.is_empty() {
            return "No active params".to_string();
        }
        let pairs = self
            .session_state
            .active_parameters
            .iter()
            .map(|(k, v)| format!("${}={}", k, v))
            .collect::<Vec<_>>()
            .join(", ");
        format!("Params: {}", pairs)
    }

    fn set_param_command(&mut self, raw: &str) -> Result<String> {
        let (name_raw, value_raw) = raw
            .split_once(' ')
            .ok_or_else(|| anyhow::anyhow!("usage: :param set <name> <value>"))?;
        let name = normalize_param_name(name_raw);
        if name.is_empty() {
            anyhow::bail!("param name cannot be empty");
        }
        let value = parse_command_value(value_raw);
        self.session_state
            .active_parameters
            .insert(name.clone(), value.clone());
        self.cache.clear();
        self.last_cache_event = None;
        self.persist_session_data();
        Ok(format!("Param set: ${}={}", name, value))
    }

    fn unset_param_command(&mut self, key: &str) -> Result<String> {
        let name = normalize_param_name(key);
        if name.is_empty() {
            anyhow::bail!("usage: :param unset <name>");
        }
        if self.session_state.active_parameters.remove(&name).is_some() {
            self.cache.clear();
            self.last_cache_event = None;
            self.persist_session_data();
            Ok(format!("Param removed: ${}", name))
        } else {
            Ok(format!("Param not found: ${}", name))
        }
    }

    fn clear_params_command(&mut self) -> Result<String> {
        if self.session_state.active_parameters.is_empty() {
            return Ok("No params to clear".to_string());
        }
        self.session_state.active_parameters.clear();
        self.cache.clear();
        self.last_cache_event = None;
        self.persist_session_data();
        Ok("All params cleared".to_string())
    }

    fn open_help_modal(&mut self) {
        self.help_return_mode = self.mode;
        self.help_scroll = 0;
        self.mode = Mode::Help;
        self.status_message = "Help opened".to_string();
    }

    fn close_help_modal(&mut self) {
        self.mode = self.help_return_mode;
        if matches!(self.mode, Mode::Help) {
            self.mode = Mode::Normal;
        }
        self.status_message = "Ready".to_string();
    }

    fn open_palette(&mut self) {
        if self.mode == Mode::Palette {
            return;
        }
        self.palette.return_mode = self.mode;
        self.palette.query.clear();
        self.palette.selected = 0;
        self.mode = Mode::Palette;
        self.status_message = "Command Palette".to_string();
    }

    fn close_palette(&mut self) {
        self.mode = self.palette.return_mode;
        if matches!(self.mode, Mode::Palette) {
            self.mode = Mode::Normal;
        }
        self.status_message = "Ready".to_string();
    }

    fn palette_entries(&self) -> Vec<PaletteEntry> {
        let mut entries = Vec::new();
        let query = self.palette.query.trim().to_lowercase();

        let base_cmds = vec![
            ("Run Query", "run current editor query", PaletteAction::Execute(RunMode::Run)),
            (
                "Explain Query",
                "explain current editor query",
                PaletteAction::Execute(RunMode::Explain),
            ),
            (
                "Profile Query",
                "profile current editor query",
                PaletteAction::Execute(RunMode::Profile),
            ),
            ("Help", ":help", PaletteAction::RunCommand("help".to_string())),
            ("Toggle Schema Panel", ":schema", PaletteAction::RunCommand("schema".to_string())),
            ("Toggle Graph Panel", ":graph", PaletteAction::RunCommand("graph".to_string())),
            (
                "Refresh Graph",
                ":graph refresh",
                PaletteAction::RunCommand("graph refresh".to_string()),
            ),
            (
                "Results Table",
                ":results table",
                PaletteAction::RunCommand("results table".to_string()),
            ),
            (
                "Results JSON",
                ":results json",
                PaletteAction::RunCommand("results json".to_string()),
            ),
            (
                "Results Graph",
                ":results graph",
                PaletteAction::RunCommand("results graph".to_string()),
            ),
            (
                "Results Plan",
                ":results plan",
                PaletteAction::RunCommand("results plan".to_string()),
            ),
            (
                "Open Session Browser",
                ":browser",
                PaletteAction::RunCommand("browser".to_string()),
            ),
            (
                "Toggle Timeline Strip",
                ":timeline",
                PaletteAction::RunCommand("timeline".to_string()),
            ),
            (
                "Cache Stats",
                ":cache stats",
                PaletteAction::RunCommand("cache stats".to_string()),
            ),
            (
                "Cache Stats Session",
                ":cache stats session",
                PaletteAction::RunCommand("cache stats session".to_string()),
            ),
            (
                "Cache Stats Tab",
                ":cache stats tab",
                PaletteAction::RunCommand("cache stats tab".to_string()),
            ),
            (
                "Cache Recent Session",
                ":cache recent session 20",
                PaletteAction::RunCommand("cache recent session 20".to_string()),
            ),
            (
                "Cache Recent Tab",
                ":cache recent tab 20",
                PaletteAction::RunCommand("cache recent tab 20".to_string()),
            ),
            (
                "Cache Clear",
                ":cache clear",
                PaletteAction::RunCommand("cache clear".to_string()),
            ),
            (
                "List Params",
                ":params",
                PaletteAction::RunCommand("params".to_string()),
            ),
            (
                "Clear Params",
                ":params clear",
                PaletteAction::RunCommand("params clear".to_string()),
            ),
        ];

        for (title, detail, action) in base_cmds {
            entries.push(PaletteEntry {
                title: title.to_string(),
                detail: detail.to_string(),
                action,
            });
        }

        if self.session_v2_enabled {
            for (recent_idx, entry) in self.session_state.recent_timeline(40).iter().enumerate() {
                let title = format!("Run: {}", truncate_one_line(&entry.query, 56));
                let detail = format!(
                    "timeline #{} [{}]{}",
                    recent_idx + 1,
                    run_mode_label(entry.run_mode),
                    if entry.pinned { " [pinned]" } else { "" }
                );
                entries.push(PaletteEntry {
                    title,
                    detail,
                    action: PaletteAction::RerunTimeline(recent_idx),
                });
            }

            for (idx, s) in self.session_state.saved_queries.iter().enumerate().take(80) {
                entries.push(PaletteEntry {
                    title: format!("Snippet: {}", s.name),
                    detail: truncate_one_line(&s.query, 52),
                    action: PaletteAction::LoadSnippet(s.name.clone()),
                });
                let _ = idx;
            }

            for (idx, t) in self.session_state.tabs.iter().enumerate().take(40) {
                let active = if t.id == self.session_state.active_tab_id {
                    " [active]"
                } else {
                    ""
                };
                entries.push(PaletteEntry {
                    title: format!("Tab: {}{}", t.title, active),
                    detail: truncate_one_line(&t.query_text, 52),
                    action: PaletteAction::ActivateTab(idx),
                });
            }
        }

        if query.is_empty() {
            return entries;
        }

        entries
            .into_iter()
            .filter(|e| {
                let hay = format!("{} {}", e.title.to_lowercase(), e.detail.to_lowercase());
                hay.contains(&query)
            })
            .collect()
    }

    fn apply_palette_selection(&mut self) -> Result<()> {
        let entries = self.palette_entries();
        let Some(entry) = entries.get(self.palette.selected).cloned() else {
            return Ok(());
        };

        self.mode = self.palette.return_mode;
        if matches!(self.mode, Mode::Palette) {
            self.mode = Mode::Normal;
        }

        match entry.action {
            PaletteAction::Execute(run_mode) => {
                self.execute_query_with_mode(run_mode)?;
            }
            PaletteAction::RunCommand(cmd) => {
                self.command_buffer = cmd;
                self.execute_command()?;
                if self.mode == Mode::Command {
                    self.mode = Mode::Normal;
                }
            }
            PaletteAction::RerunTimeline(recent_idx) => {
                let recent = self.session_state.recent_timeline(200);
                if let Some(e) = recent.get(recent_idx) {
                    let query = e.query.clone();
                    self.editor.set_content(query);
                    self.execute_query_with_mode(e.run_mode)?;
                }
            }
            PaletteAction::LoadSnippet(name) => {
                self.load_snippet_to_editor(&name)?;
            }
            PaletteAction::ActivateTab(idx) => {
                if self.session_state.activate_tab_by_index(idx)
                    && let Some(tab) = self.session_state.active_tab()
                {
                    let query = tab.query_text.clone();
                    self.editor.set_content(query);
                    self.persist_session_data();
                }
            }
        }

        Ok(())
    }

    pub fn help_lines(&self) -> Vec<String> {
        let mut out = vec![
            "## Versions".to_string(),
            help_version_line(),
            String::new(),
            "## Core".to_string(),
            "  ?          open/close help modal".to_string(),
            "  Ctrl+k     open command palette".to_string(),
            "  i          insert mode".to_string(),
            "  :          command mode".to_string(),
            "  Enter      run query".to_string(),
            "  :run       run editor query".to_string(),
            "  :explain   explain editor query".to_string(),
            "  :profile   run + profile summary".to_string(),
            "  Tab        switch focus editor/results".to_string(),
            "  1 / 2      focus editor/results".to_string(),
            "  t          cycle results mode".to_string(),
            "  g / G      results top/bottom".to_string(),
            String::new(),
            "## Panels".to_string(),
            "  s          toggle schema panel".to_string(),
            "  x          toggle graph panel".to_string(),
            "  Ctrl+h/l   resize side panel".to_string(),
            "  Shift+J/K  scroll side panel".to_string(),
            String::new(),
            "## Graph".to_string(),
            "  r          refresh graph snapshot".to_string(),
            "  f          focus graph from results row".to_string(),
            "  o          focus selected graph neighbor".to_string(),
            "  + / -      graph depth".to_string(),
            "  :graph labels | :graph label <L> | :graph label *".to_string(),
            "  :graph focus <id> | :graph focus name \"...\"".to_string(),
            String::new(),
            "## Editor".to_string(),
            "  Ctrl+d     delete current line".to_string(),
            "  Ctrl+u     clear editor".to_string(),
            "  :editor clear | :editor delete-line".to_string(),
            String::new(),
            "## Results".to_string(),
            "  :results table | json | graph | plan".to_string(),
            "  up/down, PgUp/PgDn, Home/End scroll".to_string(),
            "  (Plan mode) j/k:operator nav • z:collapse section".to_string(),
            String::new(),
            "## Session v2 (set NDBSTUDIO_SESSION_V2=1)".to_string(),
            "  y          toggle timeline strip".to_string(),
            "  R          rerun last timeline query".to_string(),
            "  Timeline rows include cache badge: HIT/MISS".to_string(),
            "  [ / ]      prev/next tab".to_string(),
            "  Ctrl+t/w   new/close tab".to_string(),
            "  b          open session browser".to_string(),
            "  :session | :timeline | :timeline rerun <n> | :timeline pin <n>".to_string(),
            "  :timeline rerun <n> --as <run|explain|profile>".to_string(),
            "  :timeline lineage <n> | :timeline rerun dependents <n>".to_string(),
            "  :timeline dag <n> (render lineage neighborhood in Graph mode)".to_string(),
            "  :timeline impact <n> [--threshold N]".to_string(),
            "  :timeline rerun impacted <n> [--threshold N]".to_string(),
            "  :tab new [title] | :tab next | :tab prev | :tab close | :tabs".to_string(),
            "  :save <name> | :snippets | :snippet run <name>".to_string(),
            "  :browser filter <text> | :browser filter clear | :browser mode <...>".to_string(),
            "  :cache stats [session|tab] | :cache recent [session|tab] [N]".to_string(),
            "  :cache clear".to_string(),
            "  :params | :params clear | :param set <k> <v> | :param unset <k>".to_string(),
            String::new(),
            "## Session Browser".to_string(),
            "  /          quick filter (live)".to_string(),
            "  Tab        cycle pane (timeline/snippets/tabs)".to_string(),
            "  j/k        move selection".to_string(),
            "  Enter/r    run selected or activate tab".to_string(),
            "  l          load selected to editor".to_string(),
            "  p          pin/unpin selected timeline entry".to_string(),
            String::new(),
            "## Commands".to_string(),
            "  :help      open this modal".to_string(),
            "  :history   open history view".to_string(),
            "  :q         quit".to_string(),
        ];
        if !self.session_v2_enabled {
            out.push(String::new());
            out.push("  Note: Session v2 is disabled in this run.".to_string());
        }
        out
    }

    fn session_browser_select_next(&mut self) {
        match self.session_browser.active {
            SessionPane::Timeline => {
                let max = self.filtered_timeline_indices(200).len().saturating_sub(1);
                self.session_browser.timeline_idx = (self.session_browser.timeline_idx + 1).min(max);
            }
            SessionPane::Snippets => {
                let max = self.filtered_snippet_indices(200).len().saturating_sub(1);
                self.session_browser.snippet_idx = (self.session_browser.snippet_idx + 1).min(max);
            }
            SessionPane::Tabs => {
                let max = self.filtered_tab_indices(200).len().saturating_sub(1);
                self.session_browser.tab_idx = (self.session_browser.tab_idx + 1).min(max);
            }
        }
    }

    fn session_browser_select_prev(&mut self) {
        match self.session_browser.active {
            SessionPane::Timeline => {
                self.session_browser.timeline_idx = self.session_browser.timeline_idx.saturating_sub(1);
            }
            SessionPane::Snippets => {
                self.session_browser.snippet_idx = self.session_browser.snippet_idx.saturating_sub(1);
            }
            SessionPane::Tabs => {
                self.session_browser.tab_idx = self.session_browser.tab_idx.saturating_sub(1);
            }
        }
    }

    fn run_selected_session_item(&mut self) -> Result<String> {
        if self.session_browser.active == SessionPane::Tabs {
            return self.activate_selected_browser_tab();
        }

        let Some((query, run_mode)) = self.selected_session_item() else {
            return Ok("No item selected".to_string());
        };
        self.editor.set_content(query);
        self.mode = Mode::Normal;
        self.execute_query_with_mode(run_mode)?;
        Ok(format!("Running selected item [{}]", run_mode_label(run_mode)))
    }

    fn load_selected_session_item(&mut self) -> Result<String> {
        if self.session_browser.active == SessionPane::Tabs {
            return self.activate_selected_browser_tab();
        }

        let Some((query, _)) = self.selected_session_item() else {
            return Ok("No item selected".to_string());
        };
        self.editor.set_content(query);
        self.mode = Mode::Normal;
        Ok("Loaded selected item into editor".to_string())
    }

    fn activate_selected_browser_tab(&mut self) -> Result<String> {
        self.ensure_session_v2()?;
        let Some(tab_idx) = self
            .filtered_tab_indices(200)
            .get(self.session_browser.tab_idx)
            .copied()
        else {
            return Ok("Tab not found".to_string());
        };
        if !self.session_state.activate_tab_by_index(tab_idx) {
            return Ok("Tab not found".to_string());
        }
        if let Some(tab) = self.session_state.active_tab() {
            let title = tab.title.clone();
            self.editor.set_content(tab.query_text.clone());
            self.mode = Mode::Normal;
            self.persist_session_data();
            return Ok(format!("Activated tab: {}", title));
        }
        Ok("Tab activated".to_string())
    }

    fn selected_session_item(&self) -> Option<(String, RunMode)> {
        match self.session_browser.active {
            SessionPane::Timeline => {
                let recent_idx = self
                    .filtered_timeline_indices(200)
                    .get(self.session_browser.timeline_idx)
                    .copied()?;
                self.session_state
                    .recent_timeline(200)
                    .get(recent_idx)
                    .map(|e| (e.query.clone(), e.run_mode))
            }
            SessionPane::Snippets => {
                let snippet_idx = self
                    .filtered_snippet_indices(200)
                    .get(self.session_browser.snippet_idx)
                    .copied()?;
                self.session_state
                    .saved_queries
                    .get(snippet_idx)
                    .map(|s| (s.query.clone(), RunMode::Run))
            }
            SessionPane::Tabs => None,
        }
    }

    fn selected_timeline_recent_index(&self) -> Option<usize> {
        self.filtered_timeline_indices(300)
            .get(self.session_browser.timeline_idx)
            .copied()
    }

    fn show_selected_timeline_dag(&mut self) -> Result<String> {
        let Some(recent_idx) = self.selected_timeline_recent_index() else {
            return Ok("No timeline item selected".to_string());
        };
        self.show_timeline_dag(&(recent_idx + 1).to_string())
    }

    fn show_selected_timeline_lineage(&mut self) -> Result<String> {
        let Some(recent_idx) = self.selected_timeline_recent_index() else {
            return Ok("No timeline item selected".to_string());
        };
        self.show_timeline_impact(&(recent_idx + 1).to_string())
    }

    pub fn session_browser_timeline_rows(&self, limit: usize) -> Vec<String> {
        let filtered = self.filtered_timeline_indices(limit);
        self.session_state
            .recent_timeline(limit)
            .iter()
            .enumerate()
            .filter_map(|(recent_idx, entry)| {
                if !filtered.contains(&recent_idx) {
                    return None;
                }
                let visible_idx = filtered
                    .iter()
                    .position(|idx| *idx == recent_idx)
                    .unwrap_or(usize::MAX);
                let marker = if visible_idx == self.session_browser.timeline_idx {
                    ">"
                } else {
                    " "
                };
                let pin = if entry.pinned { "*" } else { " " };
                let ms = entry
                    .duration_ms
                    .map(|d| format!("{:.1}ms", d))
                    .unwrap_or_else(|| "-".to_string());
                let status = match entry.status {
                    crate::session::RunStatus::Success => "OK",
                    crate::session::RunStatus::Failure => "ERR",
                };
                let mode = run_mode_short(entry.run_mode);
                let cache = match entry.cache_status {
                    Some(CacheStatus::Hit) => "HIT",
                    Some(CacheStatus::Miss) => "MISS",
                    None => "-",
                };
                let lineage = format!("d{}", entry.depends_on.len());
                let preview = if entry.query.len() > 52 {
                    format!("{}...", &entry.query[..52])
                } else {
                    entry.query.clone()
                };
                let quick = entry
                    .summary
                    .as_ref()
                    .or(entry.error.as_ref())
                    .map(|s| truncate_one_line(s, 44))
                    .unwrap_or_default();
                if quick.is_empty() {
                    Some(format!(
                        "{}{} [{} {} {} {} {}] {}",
                        marker, pin, mode, status, cache, lineage, ms, preview
                    ))
                } else {
                    Some(format!(
                        "{}{} [{} {} {} {} {}] {} • {}",
                        marker, pin, mode, status, cache, lineage, ms, preview, quick
                    ))
                }
            })
            .collect()
    }

    pub fn session_browser_snippet_rows(&self, limit: usize) -> Vec<String> {
        let filtered = self.filtered_snippet_indices(limit);
        self.session_state
            .saved_queries
            .iter()
            .take(limit)
            .enumerate()
            .filter_map(|(idx, s)| {
                if !filtered.contains(&idx) {
                    return None;
                }
                let visible_idx = filtered
                    .iter()
                    .position(|v| *v == idx)
                    .unwrap_or(usize::MAX);
                let marker = if visible_idx == self.session_browser.snippet_idx {
                    ">"
                } else {
                    " "
                };
                let preview = if s.query.len() > 40 {
                    format!("{}...", &s.query[..40])
                } else {
                    s.query.clone()
                };
                Some(format!("{} {}: {}", marker, s.name, preview))
            })
            .collect()
    }

    pub fn session_browser_active_pane(&self) -> &'static str {
        match self.session_browser.active {
            SessionPane::Timeline => "timeline",
            SessionPane::Snippets => "snippets",
            SessionPane::Tabs => "tabs",
        }
    }

    pub fn session_browser_tab_rows(&self, limit: usize) -> Vec<String> {
        let filtered = self.filtered_tab_indices(limit);
        self.session_state
            .tabs
            .iter()
            .take(limit)
            .enumerate()
            .filter_map(|(idx, tab)| {
                if !filtered.contains(&idx) {
                    return None;
                }
                let visible_idx = filtered
                    .iter()
                    .position(|v| *v == idx)
                    .unwrap_or(usize::MAX);
                let marker = if visible_idx == self.session_browser.tab_idx {
                    ">"
                } else {
                    " "
                };
                let active = if tab.id == self.session_state.active_tab_id {
                    "*"
                } else {
                    " "
                };
                let preview = if tab.query_text.len() > 34 {
                    format!("{}...", &tab.query_text[..34])
                } else {
                    tab.query_text.clone()
                };
                Some(format!("{}{} {}: {}", marker, active, tab.title, preview))
            })
            .collect()
    }

    fn filtered_timeline_indices(&self, limit: usize) -> Vec<usize> {
        let (mode_filter, text_filter) = parse_session_filter(&self.session_browser.filter_text);
        self.session_state
            .recent_timeline(limit)
            .iter()
            .enumerate()
            .filter_map(|(recent_idx, entry)| {
                if let Some(mode_filter) = mode_filter
                    && entry.run_mode != mode_filter
                {
                    return None;
                }
                if text_filter.is_empty() || entry.query.to_lowercase().contains(&text_filter) {
                    Some(recent_idx)
                } else {
                    None
                }
            })
            .collect()
    }

    fn filtered_snippet_indices(&self, limit: usize) -> Vec<usize> {
        let filter = self.session_browser.filter_text.to_lowercase();
        self.session_state
            .saved_queries
            .iter()
            .take(limit)
            .enumerate()
            .filter_map(|(idx, s)| {
                let hay = format!("{} {}", s.name, s.query).to_lowercase();
                if filter.is_empty() || hay.contains(&filter) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    fn filtered_tab_indices(&self, limit: usize) -> Vec<usize> {
        let filter = self.session_browser.filter_text.to_lowercase();
        self.session_state
            .tabs
            .iter()
            .take(limit)
            .enumerate()
            .filter_map(|(idx, t)| {
                let hay = format!("{} {}", t.title, t.query_text).to_lowercase();
                if filter.is_empty() || hay.contains(&filter) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    fn reset_session_browser_selection(&mut self) {
        self.session_browser.timeline_idx = 0;
        self.session_browser.snippet_idx = 0;
        self.session_browser.tab_idx = 0;
    }

    fn set_session_browser_filter(&mut self, text: &str) -> Result<String> {
        self.ensure_session_v2()?;
        self.session_browser.filter_text = text.trim().to_string();
        self.reset_session_browser_selection();
        Ok(if self.session_browser.filter_text.is_empty() {
            "Session filter cleared".to_string()
        } else {
            format!("Session filter: {}", self.session_browser.filter_text)
        })
    }

    fn refresh_graph_view(&mut self) -> Result<()> {
        let snapshot = self.runtime.block_on(workbench::build_graph_snapshot(&self.graph))?;
        let node_data = snapshot
            .nodes
            .iter()
            .map(|n| (n.id.to_string(), n.label.clone()))
            .collect::<Vec<_>>();
        let edge_data = snapshot
            .edges
            .iter()
            .map(|e| (e.source.to_string(), e.target.to_string(), e.edge_type.clone()))
            .collect::<Vec<_>>();
        self.graph_view.set_snapshot(node_data, edge_data);
        self.graph_last_refresh = Some(Local::now().format("%H:%M:%S").to_string());
        Ok(())
    }

    fn set_graph_label_filter(&mut self, label: Option<String>) -> Result<String> {
        self.graph_view.set_label_filter(label.clone());
        if self.side_panel != SidePanel::Graph {
            self.side_panel = SidePanel::Graph;
        }
        self.refresh_graph_view()?;
        self.save_ui_prefs()?;

        match label {
            Some(v) => Ok(format!("Graph label filter: {}", v)),
            None => Ok("Graph label filter: *".to_string()),
        }
    }

    fn focus_graph_node(&mut self, node_id: &str) -> Result<String> {
        if self.side_panel != SidePanel::Graph {
            self.side_panel = SidePanel::Graph;
            self.refresh_graph_view()?;
        }

        if self.graph_view.focus_by_id(node_id) {
            Ok(format!("Graph focus: {}", short_node_id(node_id)))
        } else {
            Ok(format!(
                "Node not visible in current graph view: {} (try :graph label *)",
                short_node_id(node_id)
            ))
        }
    }

    fn focus_graph_by_name(&mut self, name: &str) -> Result<String> {
        let target = name.trim();
        if target.is_empty() {
            return Ok("Usage: :graph focus name \"Jon Snow\"".to_string());
        }

        let target_lower = target.to_lowercase();
        let nodes = self.runtime.block_on(self.graph.get_all_nodes())?;

        let mut exact = Vec::new();
        let mut partial = Vec::new();

        for node in nodes {
            for key in ["name", "title"] {
                if let Some(PropertyValue::String(value)) = node.properties.get(key) {
                    let value_lower = value.to_lowercase();
                    if value_lower == target_lower {
                        exact.push((node.id.to_string(), value.clone(), key));
                    } else if value_lower.contains(&target_lower) {
                        partial.push((node.id.to_string(), value.clone(), key));
                    }
                }
            }
        }

        let (selected, match_kind, total_matches) = if let Some(first) = exact.first().cloned() {
            (first, "exact", exact.len())
        } else if let Some(first) = partial.first().cloned() {
            (first, "partial", partial.len())
        } else {
            return Ok(format!(
                "No node found by name/title: {} (hint: :graph label * then :graph refresh)",
                target
            ));
        };

        let focus_status = self.focus_graph_node(&selected.0)?;
        let suffix = if total_matches > 1 {
            format!(" ({} matches, focused first)", total_matches)
        } else {
            String::new()
        };

        Ok(format!(
            "{} • {} match on {}=\"{}\"{}",
            focus_status, match_kind, selected.2, selected.1, suffix
        ))
    }

    fn focus_graph_from_results_row(&mut self) -> Result<String> {
        let Some(row) = self.results.current_row().cloned() else {
            return Ok("Results are empty".to_string());
        };

        let maybe_id = row
            .iter()
            .find_map(|cell| extract_node_id(cell))
            .or_else(|| self.resolve_node_id_from_row_values(&row).ok().flatten());
        let Some(node_id) = maybe_id else {
            return Ok(
                "No node id found in current result row. Include <var>.id or a name/title column."
                    .to_string(),
            );
        };

        self.focus_graph_node(&node_id)
    }

    fn resolve_node_id_from_row_values(&mut self, row: &[String]) -> Result<Option<String>> {
        let candidates = row
            .iter()
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
            .map(|v| v.to_lowercase())
            .collect::<Vec<_>>();

        if candidates.is_empty() {
            return Ok(None);
        }

        let nodes = self.runtime.block_on(self.graph.get_all_nodes())?;
        for node in nodes {
            for key in ["name", "title"] {
                if let Some(PropertyValue::String(value)) = node.properties.get(key)
                    && candidates.iter().any(|candidate| candidate == &value.to_lowercase())
                {
                    return Ok(Some(node.id.to_string()));
                }
            }
        }

        Ok(None)
    }

    fn show_graph_labels(&mut self) -> Result<String> {
        if self.graph_view.available_labels().is_empty() {
            self.refresh_graph_view()?;
        }

        let labels = self.graph_view.available_labels();
        if labels.is_empty() {
            self.results
                .set_data(vec!["label".to_string()], vec![vec!["(no labels)".to_string()]]);
            return Ok("No labels found in graph".to_string());
        }

        let rows = labels.iter().map(|l| vec![l.clone()]).collect::<Vec<_>>();
        self.results.set_data(vec!["label".to_string()], rows);
        Ok(format!("{} labels", labels.len()))
    }

    fn autocomplete_command(&mut self) -> Result<bool> {
        let prefix = "graph label ";
        if !self.command_buffer.starts_with(prefix) {
            return Ok(false);
        }

        if self.graph_view.available_labels().is_empty() {
            self.refresh_graph_view()?;
        }

        let current = self.command_buffer.trim_start_matches(prefix).trim();
        if let Some(suggestion) = self.graph_view.suggest_label(current) {
            self.command_buffer = format!("{}{}", prefix, suggestion);
            return Ok(true);
        }

        Ok(false)
    }
}

fn database_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .unwrap_or_else(|| path.to_string())
}

fn format_count(value: usize) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + (digits.len() / 3));

    for (idx, ch) in digits.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }

    out.chars().rev().collect()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct UiPrefs {
    #[serde(default)]
    side_panel: SidePanel,
    #[serde(default = "default_side_panel_width")]
    side_panel_width: u16,
}

fn ui_prefs_path() -> Result<PathBuf> {
    let home = std::env::var("HOME").context("HOME environment variable is not set")?;
    Ok(PathBuf::from(home).join(".ndstudio").join("ui_state.json"))
}

fn default_side_panel_width() -> u16 {
    32
}

fn format_error_chain(err: &anyhow::Error) -> String {
    err.chain()
        .enumerate()
        .map(|(idx, cause)| {
            if idx == 0 {
                cause.to_string()
            } else {
                format!("caused by {}: {}", idx, cause)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn extract_node_id(value: &str) -> Option<String> {
    if looks_like_uuid(value) {
        return Some(value.to_string());
    }

    for token in value
        .split(|c: char| !(c.is_ascii_hexdigit() || c == '-'))
        .filter(|t| !t.is_empty())
    {
        if looks_like_uuid(token) {
            return Some(token.to_string());
        }
    }

    None
}

fn looks_like_uuid(value: &str) -> bool {
    if value.len() != 36 {
        return false;
    }

    value.chars().enumerate().all(|(idx, ch)| match idx {
        8 | 13 | 18 | 23 => ch == '-',
        _ => ch.is_ascii_hexdigit(),
    })
}

fn short_node_id(node_id: &str) -> String {
    node_id.chars().take(8).collect()
}

fn truncate_one_line(input: &str, max_chars: usize) -> String {
    let one_line = input.replace('\n', " ");
    if one_line.chars().count() <= max_chars {
        return one_line;
    }
    one_line.chars().take(max_chars).collect::<String>() + "..."
}

fn parse_command_value(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.len() >= 2 {
        if let Some(stripped) = strip_wrapping_quotes(trimmed, '"') {
            return stripped.to_string();
        }
        if let Some(stripped) = strip_wrapping_quotes(trimmed, '\'') {
            return stripped.to_string();
        }
    }
    trimmed.to_string()
}

fn strip_wrapping_quotes(input: &str, quote: char) -> Option<&str> {
    if input.starts_with(quote) && input.ends_with(quote) && input.len() >= 2 {
        Some(&input[1..input.len() - 1])
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CacheEvent {
    Hit,
    Miss,
}

#[derive(Debug, Clone)]
struct CachedQueryResult {
    summary: String,
    normalized_query: String,
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    row_count: usize,
}

#[derive(Debug, Default)]
struct CacheMetrics {
    hits: usize,
    misses: usize,
}

#[derive(Debug)]
struct ResultCache {
    capacity: usize,
    order: VecDeque<String>,
    entries: HashMap<String, CachedQueryResult>,
    metrics: CacheMetrics,
}

impl ResultCache {
    fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            order: VecDeque::new(),
            entries: HashMap::new(),
            metrics: CacheMetrics::default(),
        }
    }

    fn get(&mut self, key: &str, normalized_query: &str) -> Option<CachedQueryResult> {
        if let Some(value) = self.entries.get(key).cloned() {
            if value.normalized_query != normalized_query {
                self.metrics.misses = self.metrics.misses.saturating_add(1);
                return None;
            }
            self.metrics.hits = self.metrics.hits.saturating_add(1);
            self.touch(key);
            return Some(value);
        }
        self.metrics.misses = self.metrics.misses.saturating_add(1);
        None
    }

    fn put(&mut self, key: String, value: CachedQueryResult) {
        if self.entries.contains_key(&key) {
            self.entries.insert(key.clone(), value);
            self.touch(&key);
            return;
        }
        if self.entries.len() >= self.capacity {
            self.evict_oldest();
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, value);
    }

    fn clear(&mut self) {
        self.order.clear();
        self.entries.clear();
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn capacity(&self) -> usize {
        self.capacity
    }

    fn hits(&self) -> usize {
        self.metrics.hits
    }

    fn misses(&self) -> usize {
        self.metrics.misses
    }

    fn touch(&mut self, key: &str) {
        if let Some(pos) = self.order.iter().position(|k| k == key) {
            let _ = self.order.remove(pos);
        }
        self.order.push_back(key.to_string());
    }

    fn evict_oldest(&mut self) {
        if let Some(oldest) = self.order.pop_front() {
            let _ = self.entries.remove(&oldest);
        }
    }
}

fn run_mode_label(mode: RunMode) -> &'static str {
    match mode {
        RunMode::Run => "Run",
        RunMode::Explain => "Explain",
        RunMode::Profile => "Profile",
    }
}

fn run_mode_short(mode: RunMode) -> &'static str {
    match mode {
        RunMode::Run => "RUN",
        RunMode::Explain => "EXP",
        RunMode::Profile => "PRO",
    }
}

fn parse_run_mode(raw: &str) -> Option<RunMode> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "run" => Some(RunMode::Run),
        "explain" | "exp" => Some(RunMode::Explain),
        "profile" | "prof" | "pro" => Some(RunMode::Profile),
        _ => None,
    }
}

fn parse_session_filter(raw: &str) -> (Option<RunMode>, String) {
    let trimmed = raw.trim();
    if let Some(rest) = trimmed.strip_prefix("mode:") {
        let mode_raw = rest.trim();
        let mode = parse_run_mode(mode_raw);
        return (mode, String::new());
    }
    (None, trimmed.to_lowercase())
}

fn normalize_query_for_cache(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn help_version_line() -> String {
    format!(
        "  NDStudio {} • NopalDB {}",
        env!("CARGO_PKG_VERSION"),
        nopaldb::VERSION
    )
}

fn build_query_hash(
    query: &str,
    run_mode: RunMode,
    db_revision: u64,
    schema_revision: u64,
    params: &std::collections::BTreeMap<String, String>,
) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    normalize_query_for_cache(query).hash(&mut hasher);
    run_mode.hash(&mut hasher);
    db_revision.hash(&mut hasher);
    schema_revision.hash(&mut hasher);
    for (k, v) in params {
        k.hash(&mut hasher);
        v.hash(&mut hasher);
    }
    format!("{:x}", hasher.finish())
}

fn normalize_param_name(raw: &str) -> String {
    raw.trim().trim_start_matches('$').to_string()
}

struct QueryEntities {
    labels: Vec<String>,
    edge_types: Vec<String>,
    properties: Vec<String>,
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

fn compute_hit_rate<I>(statuses: I) -> Option<f64>
where
    I: IntoIterator<Item = Option<CacheStatus>>,
{
    let mut hits = 0usize;
    let mut total = 0usize;
    for status in statuses.into_iter().flatten() {
        total += 1;
        if status == CacheStatus::Hit {
            hits += 1;
        }
    }
    if total == 0 {
        None
    } else {
        Some(hits as f64 / total as f64)
    }
}

fn classify_change_kind_from_query(query: &str) -> ChangeKind {
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

fn parse_index_with_threshold(raw: &str) -> Result<(String, u8)> {
    let trimmed = raw.trim();
    if let Some((left, right)) = trimmed.split_once("--threshold") {
        let idx_raw = left.trim().to_string();
        let threshold = right
            .trim()
            .parse::<u8>()
            .context("threshold must be 0..100")?;
        return Ok((idx_raw, threshold.min(100)));
    }
    Ok((trimmed.to_string(), 35))
}

struct PendingQuery {
    query: String,
    run_mode: RunMode,
    started_at: Instant,
    receiver: Receiver<Result<QueryJobResult>>,
}

struct QueuedRun {
    query: String,
    run_mode: RunMode,
}

struct QueryJobResult {
    cache_key: String,
    result: QueryExecutionResult,
}

#[cfg(test)]
mod tests {
    use crate::session::{CacheStatus, RunMode};
    use std::collections::BTreeMap;

    use super::{
        build_query_hash, compute_hit_rate, database_name, format_count, help_version_line,
        normalize_query_for_cache, parse_command_value, parse_run_mode, parse_session_filter, ResultCache,
    };

    #[test]
    fn format_count_handles_small_number() {
        assert_eq!(format_count(1), "1");
    }

    #[test]
    fn format_count_handles_thousands() {
        assert_eq!(format_count(1000), "1,000");
    }

    #[test]
    fn format_count_handles_millions() {
        assert_eq!(format_count(1_234_567), "1,234,567");
    }

    #[test]
    fn database_name_returns_filename() {
        assert_eq!(database_name("/tmp/my_graph.db"), "my_graph.db");
    }

    #[test]
    fn database_name_returns_input_when_no_filename() {
        assert_eq!(database_name("plain-db-name"), "plain-db-name");
    }

    #[test]
    fn parse_command_value_supports_double_quotes() {
        assert_eq!(parse_command_value("\"Jon Snow\""), "Jon Snow");
    }

    #[test]
    fn parse_command_value_supports_single_quotes() {
        assert_eq!(parse_command_value("'Jon Snow'"), "Jon Snow");
    }

    #[test]
    fn parse_command_value_supports_unquoted() {
        assert_eq!(parse_command_value("Jon Snow"), "Jon Snow");
    }

    #[test]
    fn parse_run_mode_supports_aliases() {
        assert_eq!(parse_run_mode("run"), Some(RunMode::Run));
        assert_eq!(parse_run_mode("exp"), Some(RunMode::Explain));
        assert_eq!(parse_run_mode("profile"), Some(RunMode::Profile));
    }

    #[test]
    fn parse_session_filter_extracts_mode_prefix() {
        let (mode, text) = parse_session_filter("mode:profile");
        assert_eq!(mode, Some(RunMode::Profile));
        assert_eq!(text, "");
    }

    #[test]
    fn query_hash_ignores_whitespace_but_keeps_mode_and_revision() {
        let mut params = BTreeMap::new();
        params.insert("age".to_string(), "42".to_string());
        let a = build_query_hash("find n   from (n:Person)", RunMode::Run, 1, 2, &params);
        let b = build_query_hash("find n from (n:Person)", RunMode::Run, 1, 2, &params);
        let c = build_query_hash("find n from (n:Person)", RunMode::Explain, 1, 2, &params);
        let d = build_query_hash("find n from (n:Person)", RunMode::Run, 2, 2, &params);
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_ne!(a, d);
    }

    #[test]
    fn query_hash_changes_when_params_change() {
        let mut p1 = BTreeMap::new();
        p1.insert("name".to_string(), "Jon".to_string());
        let mut p2 = BTreeMap::new();
        p2.insert("name".to_string(), "Arya".to_string());

        let a = build_query_hash("find n from (n:Character)", RunMode::Run, 1, 1, &p1);
        let b = build_query_hash("find n from (n:Character)", RunMode::Run, 1, 1, &p2);
        assert_ne!(a, b);
    }

    #[test]
    fn lru_cache_evicts_oldest_entry() {
        let mut cache = ResultCache::new(2);
        cache.put(
            "a".to_string(),
            super::CachedQueryResult {
                summary: "a".to_string(),
                normalized_query: normalize_query_for_cache("find a"),
                headers: vec![],
                rows: vec![],
                row_count: 0,
            },
        );
        cache.put(
            "b".to_string(),
            super::CachedQueryResult {
                summary: "b".to_string(),
                normalized_query: normalize_query_for_cache("find b"),
                headers: vec![],
                rows: vec![],
                row_count: 0,
            },
        );
        let _ = cache.get("a", &normalize_query_for_cache("find a"));
        cache.put(
            "c".to_string(),
            super::CachedQueryResult {
                summary: "c".to_string(),
                normalized_query: normalize_query_for_cache("find c"),
                headers: vec![],
                rows: vec![],
                row_count: 0,
            },
        );

        assert!(cache.get("a", &normalize_query_for_cache("find a")).is_some());
        assert!(cache.get("b", &normalize_query_for_cache("find b")).is_none());
        assert!(cache.get("c", &normalize_query_for_cache("find c")).is_some());
    }

    #[test]
    fn cache_rejects_entry_when_normalized_query_differs() {
        let mut cache = ResultCache::new(1);
        cache.put(
            "same-key".to_string(),
            super::CachedQueryResult {
                summary: "cached".to_string(),
                normalized_query: normalize_query_for_cache("find a.name from (a:Family)"),
                headers: vec!["a.name".to_string()],
                rows: vec![vec!["Medici".to_string()]],
                row_count: 1,
            },
        );

        assert!(cache
            .get("same-key", &normalize_query_for_cache("find x.name as bridge from (x:Family)"))
            .is_none());
    }

    #[test]
    fn help_version_line_includes_ndbstudio_and_nopaldb_versions() {
        let line = help_version_line();
        assert!(line.contains("NDStudio"));
        assert!(line.contains("NopalDB"));
        assert!(line.contains(env!("CARGO_PKG_VERSION")));
        assert!(line.contains(nopaldb::VERSION));
    }

    #[test]
    fn compute_hit_rate_ignores_unknown_status_and_calculates_ratio() {
        let rate = compute_hit_rate([
            Some(CacheStatus::Hit),
            Some(CacheStatus::Miss),
            None,
            Some(CacheStatus::Hit),
        ]);
        assert_eq!(rate, Some(2.0 / 3.0));
    }

    #[test]
    fn extract_query_entities_finds_labels_edges_and_properties() {
        let e = super::extract_query_entities(
            "find c.name from (c:Character)-[:ALLY_WITH]->(h:House) where h.region = 'North'",
        );
        assert!(e.labels.iter().any(|v| v == "Character"));
        assert!(e.labels.iter().any(|v| v == "House"));
        assert!(e.edge_types.iter().any(|v| v == "ALLY_WITH"));
        assert!(e.properties.iter().any(|v| v == "name"));
        assert!(e.properties.iter().any(|v| v == "region"));
    }
}
