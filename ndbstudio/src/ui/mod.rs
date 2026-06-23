use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::app::{App, Mode, SidePanel};

pub mod editor;
pub mod graph;
pub mod help;
pub mod history;
pub mod palette;
pub mod results;
pub mod schema;
pub mod session_browser;
pub mod timeline;

// Color palette (gruvbox-inspired minimalist)
pub const BG: Color = Color::Rgb(40, 40, 40);
pub const FG: Color = Color::Rgb(235, 219, 178);
pub const ACCENT: Color = Color::Rgb(184, 187, 38);
pub const ERROR: Color = Color::Rgb(251, 73, 52);
pub const SUCCESS: Color = Color::Rgb(142, 192, 124);
pub const BORDER: Color = Color::Rgb(80, 80, 80);

pub fn draw(f: &mut Frame, app: &App) {
    match app.mode() {
        Mode::History => {
            draw_history_view(f, app);
        }
        Mode::Session => {
            draw_session_browser_view(f, app);
        }
        Mode::Help => {
            draw_main_view(f, app);
            help::draw(f, app);
        }
        Mode::Palette => {
            draw_main_view(f, app);
            palette::draw(f, app);
        }
        _ => {
            draw_main_view(f, app);
        }
    }
}

fn draw_main_view(f: &mut Frame, app: &App) {
    let timeline_height = if app.timeline_visible() { 7 } else { 0 };
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),               // Header
            Constraint::Min(0),                  // Main area
            Constraint::Length(timeline_height), // Timeline area
            Constraint::Length(1),               // Status bar
        ])
        .split(f.size());

    // Header
    draw_header(f, outer[0], app);

    let body = if app.is_schema_panel_visible() || app.is_graph_panel_visible() {
        let schema_width = app.schema_panel_width().clamp(20, 45);
        let left_width = 100u16.saturating_sub(schema_width);
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(left_width),   // Editor + results
                Constraint::Percentage(schema_width), // Side panel
            ])
            .split(outer[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100), Constraint::Length(0)])
            .split(outer[1])
    };

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage(42), // Editor
            Constraint::Percentage(58), // Results
        ])
        .split(body[0]);

    editor::draw(f, left[0], app);
    results::draw(f, left[1], app);

    match app.side_panel() {
        SidePanel::Schema => schema::draw(f, body[1], app),
        SidePanel::Graph => graph::draw(f, body[1], app),
        SidePanel::None => {}
    }

    if app.timeline_visible() {
        timeline::draw(f, outer[2], app);
    }

    // Status bar
    draw_status_bar(f, outer[3], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let mut header_text = format!(" NDStudio • {} • {}", app.mode_string(), app.db_info());
    if let Some(cache) = app.cache_hit_rate_summary() {
        header_text.push_str(&format!(" • {}", cache));
    }
    if let Some(focus) = app.graph_focus_badge() {
        header_text.push_str(&format!(" • focus {}", focus));
    }
    if let Some(updated) = app.graph_last_refresh_text() {
        header_text.push_str(&format!(" • g@{}", updated));
    }
    header_text.push(' ');

    let header = Paragraph::new(header_text).style(
        Style::default()
            .bg(ACCENT)
            .fg(BG)
            .add_modifier(Modifier::BOLD),
    );

    f.render_widget(header, area);
}

fn draw_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let mode_indicator = match app.mode() {
        Mode::Normal => Span::styled(" NORMAL ", Style::default().bg(ACCENT).fg(BG)),
        Mode::Insert => Span::styled(" INSERT ", Style::default().bg(SUCCESS).fg(BG)),
        Mode::Command => {
            let cmd_text = format!(" :{} ", app.command_buffer());
            Span::styled(cmd_text, Style::default().bg(Color::Blue).fg(FG))
        }
        Mode::Visual => Span::styled(" VISUAL ", Style::default().bg(Color::Magenta).fg(FG)),
        Mode::History => Span::styled(" HISTORY ", Style::default().bg(Color::Yellow).fg(BG)),
        Mode::Session => Span::styled(" SESSION ", Style::default().bg(Color::Cyan).fg(BG)),
        Mode::Help => Span::styled(" HELP ", Style::default().bg(Color::LightBlue).fg(BG)),
        Mode::Palette => Span::styled(" PALETTE ", Style::default().bg(Color::Magenta).fg(FG)),
    };

    let status_text = Line::from(vec![
        mode_indicator,
        Span::raw(" "),
        Span::raw(&app.status_message),
    ]);

    let status = Paragraph::new(status_text).style(Style::default().fg(FG).bg(BG));

    f.render_widget(status, area);
}

fn draw_history_view(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // Header
            Constraint::Min(0),    // History content
            Constraint::Length(1), // Status bar
        ])
        .split(f.size());

    draw_header(f, chunks[0], app);
    history::draw(f, chunks[1], app);
    draw_status_bar(f, chunks[2], app);
}

fn draw_session_browser_view(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ])
        .split(f.size());

    draw_header(f, chunks[0], app);
    session_browser::draw(f, chunks[1], app);
    draw_status_bar(f, chunks[2], app);
}

impl App {
    pub fn mode_string(&self) -> &str {
        if self.is_schema_panel_visible() {
            return "SCHEMA";
        }
        if self.is_graph_panel_visible() {
            return "GRAPH";
        }
        match self.mode() {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Command => "COMMAND",
            Mode::Visual => "VISUAL",
            Mode::History => "HISTORY",
            Mode::Session => "SESSION",
            Mode::Help => "HELP",
            Mode::Palette => "PALETTE",
        }
    }

    pub fn db_info(&self) -> String {
        self.db_info_text().to_string()
    }
}
