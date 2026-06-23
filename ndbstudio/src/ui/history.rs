use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::app::App;
use crate::ui::{ACCENT, BG, ERROR, FG, SUCCESS};

#[derive(Clone)]
pub struct QueryHistoryEntry {
    pub query: String,
    pub timestamp: chrono::DateTime<chrono::Local>,
    pub success: bool,
}

pub struct HistoryView {
    entries: Vec<QueryHistoryEntry>,
    current_index: usize,
}

impl HistoryView {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            current_index: 0,
        }
    }

    pub fn add_query(&mut self, query: String, success: bool) {
        let entry = QueryHistoryEntry {
            query,
            timestamp: chrono::Local::now(),
            success,
        };
        self.entries.push(entry);
        self.current_index = self.entries.len();
    }

    pub fn previous(&mut self) -> Option<String> {
        if self.current_index > 0 {
            self.current_index -= 1;
            Some(self.entries[self.current_index].query.clone())
        } else {
            None
        }
    }

    pub fn next(&mut self) -> Option<String> {
        if self.current_index < self.entries.len().saturating_sub(1) {
            self.current_index += 1;
            Some(self.entries[self.current_index].query.clone())
        } else {
            None
        }
    }
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" Query History ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.history.entries.is_empty() {
        let empty_message =
            Paragraph::new("No query history yet").style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty_message, inner);
        return;
    }

    // Show history entries (most recent first)
    let items: Vec<ListItem> = app
        .history
        .entries
        .iter()
        .rev()
        .take(inner.height as usize)
        .map(|entry| {
            let status_icon = if entry.success { "✓" } else { "✗" };
            let status_style = if entry.success {
                Style::default().fg(SUCCESS)
            } else {
                Style::default().fg(ERROR)
            };

            let time_str = entry.timestamp.format("%H:%M:%S").to_string();

            // Truncate long queries
            let query_preview = if entry.query.len() > 60 {
                format!("{}...", &entry.query[..60])
            } else {
                entry.query.clone()
            };

            let line = Line::from(vec![
                Span::styled(format!("{} ", status_icon), status_style),
                Span::styled(
                    format!("[{}] ", time_str),
                    Style::default().fg(Color::DarkGray),
                ),
                Span::styled(query_preview, Style::default().fg(FG)),
            ]);

            ListItem::new(line)
        })
        .collect();

    let list = List::new(items).style(Style::default().fg(FG).bg(BG));

    f.render_widget(list, inner);

    // Show navigation hint
    let hint = format!(
        " {} queries • Ctrl+p/n: navigate • q/Esc: back ",
        app.history.entries.len()
    );
    let hint_y = area.bottom().saturating_sub(1);
    let hint_x = area.x + 1;

    if hint_x < area.right() && hint_y > area.y {
        let width = hint.len().min(area.width.saturating_sub(2) as usize) as u16;
        if width == 0 {
            return;
        }
        let hint_area = Rect::new(hint_x, hint_y, width, 1);
        let hint_widget = Paragraph::new(hint).style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint_widget, hint_area);
    }
}
