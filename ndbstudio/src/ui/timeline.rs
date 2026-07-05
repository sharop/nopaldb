use ratatui::{
    layout::Rect,
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::app::App;
use crate::ui::{ACCENT, BG, FG};

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let block = Block::default()
        .title(" Timeline / Session ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT));

    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = app.timeline_rows(inner.height.saturating_sub(1) as usize);
    if rows.is_empty() {
        let empty = Paragraph::new("No runs yet. Execute a query.")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty, inner);
        return;
    }

    let items = rows
        .iter()
        .map(|row| ListItem::new(row.clone()).style(Style::default().fg(FG)))
        .collect::<Vec<_>>();
    let list = List::new(items).style(Style::default().fg(FG).bg(BG));
    f.render_widget(list, inner);
}
