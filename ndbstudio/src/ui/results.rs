use std::time::Duration;

use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, List, ListItem, Paragraph, Row, Table},
    Frame,
};
use serde_json::{Map, Value};

use crate::app::{App, FocusedPane};
use crate::ui::{ACCENT, BG, BORDER, FG};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResultsMode {
    Table,
    Json,
    Graph,
    Plan,
}

pub struct ResultsView {
    headers: Vec<String>,
    rows: Vec<Vec<String>>,
    scroll_offset: usize,
    mode: ResultsMode,
    execution_time_ms: Option<f64>,
    plan_operator_idx: usize,
    collapse_explain_body: bool,
    collapse_profile_summary: bool,
    collapse_profile_preview: bool,
}

impl ResultsView {
    pub fn new() -> Self {
        Self {
            headers: Vec::new(),
            rows: Vec::new(),
            scroll_offset: 0,
            mode: ResultsMode::Table,
            execution_time_ms: None,
            plan_operator_idx: 0,
            collapse_explain_body: false,
            collapse_profile_summary: false,
            collapse_profile_preview: false,
        }
    }

    pub fn set_data(&mut self, headers: Vec<String>, rows: Vec<Vec<String>>) {
        self.headers = headers;
        self.rows = rows;
        self.scroll_offset = 0;
        self.plan_operator_idx = 0;
        self.collapse_explain_body = false;
        self.collapse_profile_summary = false;
        self.collapse_profile_preview = false;
    }

    pub fn set_execution_time(&mut self, elapsed: Duration) {
        self.execution_time_ms = Some(elapsed.as_secs_f64() * 1000.0);
    }

    pub fn clear_execution_time(&mut self) {
        self.execution_time_ms = None;
    }

    pub fn set_mode_from_name(&mut self, mode: &str) -> bool {
        match mode.to_ascii_lowercase().as_str() {
            "table" | "tabular" => {
                self.mode = ResultsMode::Table;
                true
            }
            "json" => {
                self.mode = ResultsMode::Json;
                true
            }
            "graph" => {
                self.mode = ResultsMode::Graph;
                true
            }
            "plan" | "explain" | "profile" => {
                self.mode = ResultsMode::Plan;
                true
            }
            _ => false,
        }
    }

    pub fn cycle_mode(&mut self) -> ResultsMode {
        self.mode = match self.mode {
            ResultsMode::Table => ResultsMode::Json,
            ResultsMode::Json => ResultsMode::Graph,
            ResultsMode::Graph => ResultsMode::Plan,
            ResultsMode::Plan => ResultsMode::Table,
        };
        self.scroll_offset = 0;
        self.mode
    }

    pub fn mode(&self) -> ResultsMode {
        self.mode
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn current_row(&self) -> Option<&Vec<String>> {
        self.rows.get(self.scroll_offset)
    }

    pub fn scroll_down(&mut self) {
        if self.mode == ResultsMode::Plan {
            self.move_plan_selection(1);
            return;
        }
        if self.scroll_offset < self.current_scroll_limit() {
            self.scroll_offset += 1;
        }
    }

    pub fn scroll_up(&mut self) {
        if self.mode == ResultsMode::Plan {
            self.move_plan_selection(-1);
            return;
        }
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }

    pub fn jump_to_top(&mut self) {
        if self.mode == ResultsMode::Plan {
            self.plan_operator_idx = 0;
            self.scroll_offset = 0;
            return;
        }
        self.scroll_offset = 0;
    }

    pub fn jump_to_bottom(&mut self) {
        if self.mode == ResultsMode::Plan {
            let model = build_plan_model(self);
            self.plan_operator_idx = model.operator_line_indices.len().saturating_sub(1);
            self.scroll_offset = self.current_scroll_limit();
            return;
        }
        self.scroll_offset = self.current_scroll_limit();
    }

    pub fn scroll_by(&mut self, amount: usize) {
        let limit = self.current_scroll_limit();
        self.scroll_offset = (self.scroll_offset + amount).min(limit);
    }

    pub fn scroll_back_by(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    fn current_scroll_limit(&self) -> usize {
        match self.mode {
            ResultsMode::Table => self.rows.len().saturating_sub(1),
            ResultsMode::Json => build_json_lines(&self.headers, &self.rows)
                .len()
                .saturating_sub(1),
            ResultsMode::Graph => build_graph_lines(&self.headers, &self.rows)
                .len()
                .saturating_sub(1),
            ResultsMode::Plan => build_plan_lines(self)
                .len()
                .saturating_sub(1),
        }
    }

    pub fn toggle_plan_section(&mut self) -> Option<String> {
        if self.mode != ResultsMode::Plan {
            return None;
        }
        let model = build_plan_model(self);
        if model.lines.is_empty() {
            return None;
        }
        let selected_line_idx = self
            .selected_plan_line_index(&model)
            .unwrap_or_else(|| self.scroll_offset.min(model.lines.len().saturating_sub(1)));
        let section = model.lines[selected_line_idx].section?;
        let message = match section {
            PlanSection::ExplainBody => {
                self.collapse_explain_body = !self.collapse_explain_body;
                format!(
                    "Explain body {}",
                    if self.collapse_explain_body {
                        "collapsed"
                    } else {
                        "expanded"
                    }
                )
            }
            PlanSection::ProfileSummary => {
                self.collapse_profile_summary = !self.collapse_profile_summary;
                format!(
                    "Profile summary {}",
                    if self.collapse_profile_summary {
                        "collapsed"
                    } else {
                        "expanded"
                    }
                )
            }
            PlanSection::ProfilePreview => {
                self.collapse_profile_preview = !self.collapse_profile_preview;
                format!(
                    "Plan preview {}",
                    if self.collapse_profile_preview {
                        "collapsed"
                    } else {
                        "expanded"
                    }
                )
            }
        };
        self.clamp_plan_selection();
        Some(message)
    }

    fn move_plan_selection(&mut self, delta: isize) {
        let model = build_plan_model(self);
        if model.operator_line_indices.is_empty() {
            let limit = model.lines.len().saturating_sub(1);
            if delta > 0 {
                self.scroll_offset = (self.scroll_offset + delta as usize).min(limit);
            } else {
                self.scroll_offset = self.scroll_offset.saturating_sub((-delta) as usize);
            }
            return;
        }

        let max_idx = model.operator_line_indices.len().saturating_sub(1);
        if delta > 0 {
            self.plan_operator_idx = (self.plan_operator_idx + delta as usize).min(max_idx);
        } else {
            self.plan_operator_idx = self.plan_operator_idx.saturating_sub((-delta) as usize);
        }
        if let Some(line_idx) = model.operator_line_indices.get(self.plan_operator_idx).copied() {
            self.scroll_offset = line_idx.saturating_sub(2);
        }
    }

    fn clamp_plan_selection(&mut self) {
        let model = build_plan_model(self);
        self.plan_operator_idx = self
            .plan_operator_idx
            .min(model.operator_line_indices.len().saturating_sub(1));
        self.scroll_offset = self.scroll_offset.min(model.lines.len().saturating_sub(1));
    }

    fn selected_plan_line_index(&self, model: &PlanRenderModel) -> Option<usize> {
        model.operator_line_indices.get(self.plan_operator_idx).copied()
    }
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let is_focused = matches!(app.focused_pane(), FocusedPane::Results);

    let border_style = if is_focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(BORDER)
    };

    let exec = app
        .results
        .execution_time_ms
        .map(|ms| format!(" • {:.1}ms", ms))
        .unwrap_or_default();
    let cache_badge = app
        .cache_badge()
        .map(|v| format!(" • {}", v))
        .unwrap_or_default();
    let title = format!(
        " Results ({} rows • {} cols{}{} ) [2] ",
        app.results.row_count(),
        app.results.headers.len(),
        exec,
        cache_badge
    );

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.results.headers.is_empty() {
        let empty_message = Paragraph::new("No results yet. Execute a query with <Enter>")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty_message, inner);
        return;
    }

    let chunks = Layout::default()
        .direction(ratatui::layout::Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);

    draw_tabs(f, chunks[0], app);

    match app.results.mode() {
        ResultsMode::Table => draw_table_mode(f, chunks[1], app),
        ResultsMode::Json => draw_json_mode(f, chunks[1], app),
        ResultsMode::Graph => draw_graph_mode(f, chunks[1], app),
        ResultsMode::Plan => draw_plan_mode(f, chunks[1], app),
    }
}

fn draw_tabs(f: &mut Frame, area: Rect, app: &App) {
    let mode = app.results.mode();
    let table = if mode == ResultsMode::Table {
        Span::styled(" [Table] ", Style::default().fg(BG).bg(ACCENT).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" Table ", Style::default().fg(FG))
    };

    let json = if mode == ResultsMode::Json {
        Span::styled(" [JSON] ", Style::default().fg(BG).bg(ACCENT).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" JSON ", Style::default().fg(FG))
    };

    let graph = if mode == ResultsMode::Graph {
        Span::styled(" [Graph] ", Style::default().fg(BG).bg(ACCENT).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" Graph ", Style::default().fg(FG))
    };
    let plan = if mode == ResultsMode::Plan {
        Span::styled(" [Plan] ", Style::default().fg(BG).bg(ACCENT).add_modifier(Modifier::BOLD))
    } else {
        Span::styled(" Plan ", Style::default().fg(FG))
    };

    let mut spans = vec![
        table,
        Span::raw("  "),
        json,
        Span::raw("  "),
        graph,
        Span::raw("  "),
        plan,
        Span::raw("   (t: cycle / :results <mode>)"),
    ];
    if let Some(quick) = app.results_quick_view() {
        spans.push(Span::raw("   "));
        spans.push(Span::styled(
            quick,
            Style::default().fg(Color::Rgb(120, 210, 180)),
        ));
    }

    let tabs = Paragraph::new(ratatui::text::Line::from(spans))
    .style(Style::default().fg(FG).bg(BG));

    f.render_widget(tabs, area);
}

fn draw_table_mode(f: &mut Frame, area: Rect, app: &App) {
    let available_height = area.height.saturating_sub(2);

    let header_cells: Vec<_> = app
        .results
        .headers
        .iter()
        .map(|h| {
            Span::styled(
                h.clone(),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )
        })
        .collect();

    let visible_rows: Vec<Row> = app
        .results
        .rows
        .iter()
        .skip(app.results.scroll_offset)
        .take(available_height as usize)
        .map(|row| {
            let cells: Vec<_> = row
                .iter()
                .map(|cell| Span::styled(cell.clone(), Style::default().fg(FG)))
                .collect();
            Row::new(cells)
        })
        .collect();

    let col_count = app.results.headers.len();
    let col_width = if col_count > 0 {
        (area.width.saturating_sub(col_count as u16 + 1)) / col_count as u16
    } else {
        10
    };

    let widths: Vec<_> = (0..col_count)
        .map(|_| ratatui::layout::Constraint::Length(col_width))
        .collect();

    let table = Table::new(visible_rows, widths)
        .header(Row::new(header_cells).style(Style::default().bg(Color::Rgb(60, 60, 60))))
        .style(Style::default().fg(FG).bg(BG))
        .column_spacing(1);

    f.render_widget(table, area);
}

fn draw_json_mode(f: &mut Frame, area: Rect, app: &App) {
    let lines = build_json_lines(&app.results.headers, &app.results.rows);
    draw_line_list(f, area, &lines, app.results.scroll_offset);
}

fn draw_graph_mode(f: &mut Frame, area: Rect, app: &App) {
    let lines = build_graph_lines(&app.results.headers, &app.results.rows);
    draw_line_list(f, area, &lines, app.results.scroll_offset);
}

fn draw_plan_mode(f: &mut Frame, area: Rect, app: &App) {
    let model = build_plan_model(&app.results);
    let selected = app.results.selected_plan_line_index(&model);

    let chunks = Layout::default()
        .direction(ratatui::layout::Direction::Horizontal)
        .constraints([Constraint::Percentage(68), Constraint::Percentage(32)])
        .split(area);

    let items = model
        .lines
        .iter()
        .enumerate()
        .skip(app.results.scroll_offset)
        .take(chunks[0].height as usize)
        .map(|(idx, line)| {
            let style = if Some(idx) == selected {
                Style::default()
                    .fg(BG)
                    .bg(ACCENT)
                    .add_modifier(Modifier::BOLD)
            } else if line.is_section_header {
                Style::default().fg(Color::Rgb(220, 200, 120)).add_modifier(Modifier::BOLD)
            } else if line.is_operator {
                let (_, band, _) = estimate_operator_cost(&line.text);
                Style::default().fg(cost_band_color(band))
            } else {
                Style::default().fg(FG)
            };
            ListItem::new(line.text.clone()).style(style)
        })
        .collect::<Vec<_>>();
    let list = List::new(items).style(Style::default().fg(FG).bg(BG));
    f.render_widget(list, chunks[0]);

    let detail = selected
        .and_then(|idx| model.lines.get(idx))
        .map(|line| plan_operator_detail(&line.text))
        .unwrap_or_else(|| {
            PlanDetail {
                text: "Selecciona un operador con j/k.\nz: colapsar/expandir seccion.".to_string(),
                band: OperatorCostBand::Low,
            }
        });
    let detail = Paragraph::new(detail.text)
        .block(
            Block::default()
                .title(" Plan Detail ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(cost_band_color(detail.band))),
        )
        .style(Style::default().fg(FG));
    f.render_widget(detail, chunks[1]);
}

fn draw_line_list(f: &mut Frame, area: Rect, lines: &[String], scroll: usize) {
    if lines.is_empty() {
        let empty = Paragraph::new("No data").style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty, area);
        return;
    }

    let items = lines
        .iter()
        .skip(scroll)
        .take(area.height as usize)
        .map(|line| {
            let style = if line.starts_with('{')
                || line.starts_with('}')
                || line.starts_with('[')
                || line.starts_with(']')
            {
                Style::default().fg(ACCENT)
            } else if line.contains("--[") {
                Style::default().fg(Color::Rgb(90, 200, 170))
            } else {
                Style::default().fg(FG)
            };
            ListItem::new(line.clone()).style(style)
        })
        .collect::<Vec<_>>();

    let list = List::new(items).style(Style::default().fg(FG).bg(BG));
    f.render_widget(list, area);
}

fn build_json_lines(headers: &[String], rows: &[Vec<String>]) -> Vec<String> {
    let data = rows
        .iter()
        .map(|row| {
            let mut obj = Map::new();
            for (idx, header) in headers.iter().enumerate() {
                let val = row.get(idx).cloned().unwrap_or_default();
                obj.insert(header.clone(), parse_json_cell(&val));
            }
            Value::Object(obj)
        })
        .collect::<Vec<_>>();

    let value = Value::Array(data);
    serde_json::to_string_pretty(&value)
        .unwrap_or_else(|_| "[]".to_string())
        .lines()
        .map(|s| s.to_string())
        .collect()
}

fn build_plan_lines(results: &ResultsView) -> Vec<String> {
    build_plan_model(results)
        .lines
        .into_iter()
        .map(|l| l.text)
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlanSection {
    ExplainBody,
    ProfileSummary,
    ProfilePreview,
}

#[derive(Debug, Clone)]
struct PlanRenderLine {
    text: String,
    section: Option<PlanSection>,
    is_section_header: bool,
    is_operator: bool,
}

#[derive(Debug, Clone, Default)]
struct PlanRenderModel {
    lines: Vec<PlanRenderLine>,
    operator_line_indices: Vec<usize>,
}

fn build_plan_model(results: &ResultsView) -> PlanRenderModel {
    // EXPLAIN comes as a single "plan" cell with multiline text.
    if results.headers.len() == 1
        && results.headers[0].eq_ignore_ascii_case("plan")
        && !results.rows.is_empty()
    {
        let mut lines = vec![PlanRenderLine {
            text: "EXPLAIN PLAN".to_string(),
            section: None,
            is_section_header: true,
            is_operator: false,
        }];
        lines.push(PlanRenderLine {
            text: String::new(),
            section: None,
            is_section_header: false,
            is_operator: false,
        });
        let collapsed = results.collapse_explain_body;
        lines.push(PlanRenderLine {
            text: format!(
                "{} Plan Tree (z: toggle)",
                if collapsed { "[+]" } else { "[-]" }
            ),
            section: Some(PlanSection::ExplainBody),
            is_section_header: true,
            is_operator: false,
        });
        if collapsed {
            lines.push(PlanRenderLine {
                text: "  ...".to_string(),
                section: Some(PlanSection::ExplainBody),
                is_section_header: false,
                is_operator: false,
            });
        } else if let Some(plan) = results.rows.first().and_then(|r| r.first()) {
            for raw in plan.lines() {
                let text = raw.to_string();
                lines.push(PlanRenderLine {
                    is_operator: looks_like_plan_operator(&text),
                    text,
                    section: Some(PlanSection::ExplainBody),
                    is_section_header: false,
                });
            }
        }
        return model_with_operator_indices(lines);
    }

    // PROFILE is rendered as metric/value pairs.
    if results.headers.len() == 2
        && results.headers[0].eq_ignore_ascii_case("metric")
        && results.headers[1].eq_ignore_ascii_case("value")
    {
        let mut lines = vec![PlanRenderLine {
            text: "PROFILE SUMMARY".to_string(),
            section: None,
            is_section_header: true,
            is_operator: false,
        }];
        lines.push(PlanRenderLine {
            text: String::new(),
            section: None,
            is_section_header: false,
            is_operator: false,
        });
        let mut plan_preview = None::<String>;
        let summary_collapsed = results.collapse_profile_summary;
        lines.push(PlanRenderLine {
            text: format!(
                "{} Metrics (z: toggle)",
                if summary_collapsed { "[+]" } else { "[-]" }
            ),
            section: Some(PlanSection::ProfileSummary),
            is_section_header: true,
            is_operator: false,
        });
        for row in &results.rows {
            let metric = row.first().cloned().unwrap_or_default();
            let value = row.get(1).cloned().unwrap_or_default();
            if metric.eq_ignore_ascii_case("plan_preview") {
                plan_preview = Some(value);
            } else if !summary_collapsed {
                lines.push(PlanRenderLine {
                    text: format!("{:<14} {}", metric, value),
                    section: Some(PlanSection::ProfileSummary),
                    is_section_header: false,
                    is_operator: false,
                });
            }
        }
        if summary_collapsed {
            lines.push(PlanRenderLine {
                text: "  ...".to_string(),
                section: Some(PlanSection::ProfileSummary),
                is_section_header: false,
                is_operator: false,
            });
        }
        if let Some(plan) = plan_preview {
            lines.push(PlanRenderLine {
                text: String::new(),
                section: None,
                is_section_header: false,
                is_operator: false,
            });
            let preview_collapsed = results.collapse_profile_preview;
            lines.push(PlanRenderLine {
                text: format!(
                    "{} Plan Preview (z: toggle)",
                    if preview_collapsed { "[+]" } else { "[-]" }
                ),
                section: Some(PlanSection::ProfilePreview),
                is_section_header: true,
                is_operator: false,
            });
            if preview_collapsed {
                lines.push(PlanRenderLine {
                    text: "  ...".to_string(),
                    section: Some(PlanSection::ProfilePreview),
                    is_section_header: false,
                    is_operator: false,
                });
            } else {
                for raw in plan.lines() {
                    let text = raw.to_string();
                    lines.push(PlanRenderLine {
                        is_operator: looks_like_plan_operator(&text),
                        text,
                        section: Some(PlanSection::ProfilePreview),
                        is_section_header: false,
                    });
                }
            }
        }
        return model_with_operator_indices(lines);
    }

    model_with_operator_indices(vec![
        PlanRenderLine {
            text: "No plan/profile view for current results.".to_string(),
            section: None,
            is_section_header: false,
            is_operator: false,
        },
        PlanRenderLine {
            text: "Use :explain or :profile, or switch to Table/JSON/Graph.".to_string(),
            section: None,
            is_section_header: false,
            is_operator: false,
        },
    ])
}

fn model_with_operator_indices(lines: Vec<PlanRenderLine>) -> PlanRenderModel {
    let operator_line_indices = lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| if line.is_operator { Some(idx) } else { None })
        .collect();
    PlanRenderModel {
        lines,
        operator_line_indices,
    }
}

fn looks_like_plan_operator(line: &str) -> bool {
    let l = line.to_ascii_lowercase();
    [
        "scan",
        "filter",
        "join",
        "project",
        "index",
        "aggregate",
        "sort",
        "expand",
        "traverse",
        "seek",
        "node",
        "edge",
    ]
    .iter()
    .any(|kw| l.contains(kw))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OperatorCostBand {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone)]
struct PlanDetail {
    text: String,
    band: OperatorCostBand,
}

fn cost_band_color(band: OperatorCostBand) -> Color {
    match band {
        OperatorCostBand::Low => Color::Rgb(90, 200, 120),
        OperatorCostBand::Medium => Color::Rgb(220, 190, 90),
        OperatorCostBand::High => Color::Rgb(220, 95, 95),
    }
}

fn estimate_operator_cost(line: &str) -> (u8, OperatorCostBand, &'static str) {
    let l = line.to_ascii_lowercase();
    let (score, hint) = if l.contains("index") || l.contains("seek") {
        (18, "Acceso selectivo por indice; costo normalmente bajo.")
    } else if l.contains("filter") {
        (38, "Filtro posterior; depende de cardinalidad de entrada.")
    } else if l.contains("project") {
        (28, "Proyeccion de columnas; costo bajo-medio.")
    } else if l.contains("expand") || l.contains("traverse") || l.contains("edge") {
        (62, "Expansion en grafo; costo sensible al grado.")
    } else if l.contains("scan") || l.contains("node") {
        (72, "Scan amplio; costo alto en datasets grandes.")
    } else if l.contains("sort") {
        (76, "Ordenamiento global; costo alto por memoria/CPU.")
    } else if l.contains("aggregate") {
        (68, "Agregacion; costo alto-medio segun cardinalidad.")
    } else if l.contains("join") {
        (84, "Join entre sets; costo alto en ausencia de buenos filtros.")
    } else {
        (50, "Costo no clasificado; revisar plan completo.")
    };

    let band = if score <= 34 {
        OperatorCostBand::Low
    } else if score <= 64 {
        OperatorCostBand::Medium
    } else {
        OperatorCostBand::High
    };
    (score, band, hint)
}

fn plan_operator_detail(line: &str) -> PlanDetail {
    let l = line.to_ascii_lowercase();
    let kind = if l.contains("index") || l.contains("seek") {
        "Index / Seek"
    } else if l.contains("scan") {
        "Scan"
    } else if l.contains("filter") {
        "Filter"
    } else if l.contains("join") {
        "Join"
    } else if l.contains("aggregate") {
        "Aggregate"
    } else if l.contains("sort") {
        "Sort"
    } else if l.contains("project") {
        "Project"
    } else if l.contains("expand") || l.contains("traverse") {
        "Graph Expand"
    } else {
        "Operator"
    };
    let (score, band, cost_hint) = estimate_operator_cost(line);
    let band_label = match band {
        OperatorCostBand::Low => "LOW",
        OperatorCostBand::Medium => "MEDIUM",
        OperatorCostBand::High => "HIGH",
    };

    PlanDetail {
        text: format!(
            "Type: {}\nCost Score: {}/100 ({})\n\nLine:\n{}\n\nCost Hint:\n{}\n\nLegend:\n- LOW    0-34   (green)\n- MEDIUM 35-64  (yellow)\n- HIGH   65-100 (red)\n\nShortcuts:\n- j/k: navegar operadores\n- z: colapsar/expandir seccion",
            kind, score, band_label, line, cost_hint
        ),
        band,
    }
}

fn parse_json_cell(cell: &str) -> Value {
    if cell.eq_ignore_ascii_case("null") {
        return Value::Null;
    }
    if cell.eq_ignore_ascii_case("true") {
        return Value::Bool(true);
    }
    if cell.eq_ignore_ascii_case("false") {
        return Value::Bool(false);
    }
    if let Ok(v) = cell.parse::<i64>() {
        return Value::Number(v.into());
    }
    if let Ok(v) = cell.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(v)
    {
        return Value::Number(n);
    }
    Value::String(cell.to_string())
}

fn build_graph_lines(headers: &[String], rows: &[Vec<String>]) -> Vec<String> {
    if rows.is_empty() {
        return vec!["No rows to visualize as graph".to_string()];
    }

    let source_idx = headers
        .iter()
        .position(|h| {
            let l = h.to_ascii_lowercase();
            l.contains("source") || l.contains("from")
        })
        .or(if headers.len() >= 2 { Some(0) } else { None });

    let target_idx = headers
        .iter()
        .position(|h| {
            let l = h.to_ascii_lowercase();
            l.contains("target") || l.contains("to")
        })
        .or(if headers.len() >= 2 { Some(1) } else { None });

    let rel_idx = headers.iter().position(|h| {
        let l = h.to_ascii_lowercase();
        l.contains("edge") || l.contains("relation") || l.contains("type")
    });

    let (Some(s_idx), Some(t_idx)) = (source_idx, target_idx) else {
        return vec![
            "Need at least two columns (source/target) for graph mode".to_string(),
        ];
    };

    let mut nodes = std::collections::BTreeSet::new();
    let mut lines = Vec::new();
    lines.push("Graph Projection".to_string());

    for row in rows.iter().take(300) {
        let src = row.get(s_idx).cloned().unwrap_or_default();
        let dst = row.get(t_idx).cloned().unwrap_or_default();
        if src.is_empty() || dst.is_empty() {
            continue;
        }
        nodes.insert(src.clone());
        nodes.insert(dst.clone());

        let rel = rel_idx
            .and_then(|idx| row.get(idx).cloned())
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "REL".to_string());

        lines.push(format!("{} --[{}]--> {}", src, rel, dst));
    }

    lines.insert(1, format!("Nodes: {} • Edges: {}", nodes.len(), lines.len().saturating_sub(2)));

    if lines.len() == 2 {
        lines.push("No edge-like rows detected with current columns".to_string());
    }

    lines
}
