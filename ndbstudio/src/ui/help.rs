use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
};

use crate::app::App;
use crate::ui::{ACCENT, BG, FG};

pub fn draw(f: &mut Frame, app: &App) {
    let area = centered_rect(84, 84, f.size());
    f.render_widget(Clear, area);

    let block = Block::default()
        .title(" Help (Esc/q/? to close) ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT).add_modifier(Modifier::BOLD))
        .style(Style::default().bg(BG));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);

    let lines = app.help_lines();
    let items = lines
        .iter()
        .skip(app.help_scroll())
        .take(chunks[0].height as usize)
        .map(|line| {
            let style = if line.starts_with("##") {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else if line.starts_with("  ") {
                Style::default().fg(FG)
            } else if line.starts_with("`") {
                Style::default().fg(Color::Rgb(145, 205, 140))
            } else {
                Style::default().fg(FG)
            };
            ListItem::new(line.clone()).style(style)
        })
        .collect::<Vec<_>>();
    let list = List::new(items).style(Style::default().bg(BG));
    f.render_widget(list, chunks[0]);

    let footer = Paragraph::new("j/k/up/down scroll • PgUp/PgDn • Home/End")
        .alignment(Alignment::Left)
        .style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, chunks[1]);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}
