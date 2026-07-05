use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::app::App;
use crate::ui::{ACCENT, BG, FG, SUCCESS};

pub struct SchemaView {
    scroll_offset: usize,
    items: Vec<String>,
}

impl SchemaView {
    pub fn new() -> Self {
        Self {
            scroll_offset: 0,
            items: Vec::new(),
        }
    }

    pub fn set_items(&mut self, items: Vec<String>) {
        self.items = items;
        self.scroll_offset = 0;
    }

    pub fn scroll_down(&mut self) {
        if self.scroll_offset < self.items.len().saturating_sub(1) {
            self.scroll_offset += 1;
        }
    }

    pub fn scroll_up(&mut self) {
        if self.scroll_offset > 0 {
            self.scroll_offset -= 1;
        }
    }
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" Schema Browser ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.schema.items.is_empty() {
        let empty_message = Paragraph::new("No schema loaded")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty_message, inner);
        return;
    }

    let available_height = inner.height as usize;
    let visible_items: Vec<ListItem> = app
        .schema
        .items
        .iter()
        .skip(app.schema.scroll_offset)
        .take(available_height)
        .map(|item| {
            let style = if item.starts_with("  ") && !item.starts_with("    ") {
                Style::default().fg(SUCCESS)
            } else if item.starts_with("    ") {
                Style::default().fg(Color::DarkGray)
            } else if item.starts_with("Nodes") || item.starts_with("Edges") || item.starts_with("Statistics") {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG)
            };

            ListItem::new(item.clone()).style(style)
        })
        .collect();

    let list = List::new(visible_items).style(Style::default().fg(FG).bg(BG));

    f.render_widget(list, inner);

    let hint = " Shift+J/K: scroll • Ctrl+h/l: resize • s: hide ";
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
