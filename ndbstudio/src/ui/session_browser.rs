use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
};

use crate::app::App;
use crate::ui::{ACCENT, BG, FG, SUCCESS};

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let filter = app.session_browser_filter_text();
    let edit_marker = if app.session_browser_filter_editing() {
        " (editing)"
    } else {
        ""
    };
    let title = if filter.is_empty() {
        format!(" Session Browser{} ", edit_marker)
    } else {
        format!(" Session Browser [{}]{} ", filter, edit_marker)
    };
    let outer = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT));
    let inner = outer.inner(area);
    f.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(2)])
        .split(inner);

    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ])
        .split(chunks[0]);

    draw_timeline_pane(f, panes[0], app);
    draw_snippets_pane(f, panes[1], app);
    draw_tabs_pane(f, panes[2], app);

    let hint = "/:filter (mode:run|explain|profile) • Tab:pane • j/k:move • Enter/r:run • l:load • p:pin • g:dag • d:detail • q/Esc:back";
    let footer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(chunks[1]);
    let hint_widget = Paragraph::new(hint).style(Style::default().fg(Color::DarkGray));
    f.render_widget(hint_widget, footer[0]);
    let cache_widget = Paragraph::new(app.session_browser_cache_health_text())
        .style(Style::default().fg(Color::Rgb(120, 190, 150)));
    f.render_widget(cache_widget, footer[1]);
}

fn draw_timeline_pane(f: &mut Frame, area: Rect, app: &App) {
    let active = app.session_browser_active_pane() == "timeline";
    let block = Block::default()
        .title(if active { " Timeline * " } else { " Timeline " })
        .borders(Borders::ALL)
        .border_style(if active {
            Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = app.session_browser_timeline_rows(inner.height as usize);
    if rows.is_empty() {
        f.render_widget(
            Paragraph::new("No timeline entries").style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    }

    let items = rows
        .iter()
        .map(|row| {
            let style = if row.starts_with('>') {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG)
            };
            ListItem::new(row.clone()).style(style)
        })
        .collect::<Vec<_>>();

    let list = List::new(items).style(Style::default().fg(FG).bg(BG));
    f.render_widget(list, inner);
}

fn draw_snippets_pane(f: &mut Frame, area: Rect, app: &App) {
    let active = app.session_browser_active_pane() == "snippets";
    let block = Block::default()
        .title(if active { " Snippets * " } else { " Snippets " })
        .borders(Borders::ALL)
        .border_style(if active {
            Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = app.session_browser_snippet_rows(inner.height as usize);
    if rows.is_empty() {
        f.render_widget(
            Paragraph::new("No snippets saved").style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    }

    let items = rows
        .iter()
        .map(|row| {
            let style = if row.starts_with('>') {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG)
            };
            ListItem::new(row.clone()).style(style)
        })
        .collect::<Vec<_>>();

    let list = List::new(items).style(Style::default().fg(FG).bg(BG));
    f.render_widget(list, inner);
}

fn draw_tabs_pane(f: &mut Frame, area: Rect, app: &App) {
    let active = app.session_browser_active_pane() == "tabs";
    let block = Block::default()
        .title(if active { " Tabs * " } else { " Tabs " })
        .borders(Borders::ALL)
        .border_style(if active {
            Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::DarkGray)
        });

    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = app.session_browser_tab_rows(inner.height as usize);
    if rows.is_empty() {
        f.render_widget(
            Paragraph::new("No tabs").style(Style::default().fg(Color::DarkGray)),
            inner,
        );
        return;
    }

    let items = rows
        .iter()
        .map(|row| {
            let style = if row.starts_with('>') {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG)
            };
            ListItem::new(row.clone()).style(style)
        })
        .collect::<Vec<_>>();

    let list = List::new(items).style(Style::default().fg(FG).bg(BG));
    f.render_widget(list, inner);
}
