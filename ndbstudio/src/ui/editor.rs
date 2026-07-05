use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, FocusedPane};
use crate::ui::{ACCENT, BG, FG, BORDER};

pub struct QueryEditor {
    lines: Vec<String>,
    cursor_line: usize,
    cursor_col: usize,
}

impl QueryEditor {
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            cursor_line: 0,
            cursor_col: 0,
        }
    }

    pub fn content(&self) -> String {
        self.lines.join("\n")
    }

    pub fn set_content(&mut self, content: String) {
        self.lines = content.lines().map(|s| s.to_string()).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.cursor_line = self.lines.len().saturating_sub(1);
        self.cursor_col = self.lines[self.cursor_line].len();
    }

    pub fn insert_char(&mut self, c: char) {
        self.lines[self.cursor_line].insert(self.cursor_col, c);
        self.cursor_col += 1;
    }

    pub fn delete_char(&mut self) {
        if self.cursor_col > 0 {
            self.lines[self.cursor_line].remove(self.cursor_col - 1);
            self.cursor_col -= 1;
        } else if self.cursor_line > 0 {
            // Join with previous line
            let current_line = self.lines.remove(self.cursor_line);
            self.cursor_line -= 1;
            self.cursor_col = self.lines[self.cursor_line].len();
            self.lines[self.cursor_line].push_str(&current_line);
        }
    }

    pub fn insert_newline(&mut self) {
        let rest = self.lines[self.cursor_line].split_off(self.cursor_col);
        self.lines.insert(self.cursor_line + 1, rest);
        self.cursor_line += 1;
        self.cursor_col = 0;
    }

    pub fn move_left(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        }
    }

    pub fn move_right(&mut self) {
        if self.cursor_col < self.lines[self.cursor_line].len() {
            self.cursor_col += 1;
        }
    }

    pub fn move_up(&mut self) {
        if self.cursor_line > 0 {
            self.cursor_line -= 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len());
        }
    }

    pub fn move_down(&mut self) {
        if self.cursor_line < self.lines.len() - 1 {
            self.cursor_line += 1;
            self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len());
        }
    }

    pub fn cursor_position(&self) -> (usize, usize) {
        (self.cursor_line, self.cursor_col)
    }

    pub fn delete_current_line(&mut self) -> bool {
        if self.lines.is_empty() {
            self.lines.push(String::new());
            self.cursor_line = 0;
            self.cursor_col = 0;
            return false;
        }

        self.lines.remove(self.cursor_line);

        if self.lines.is_empty() {
            self.lines.push(String::new());
            self.cursor_line = 0;
            self.cursor_col = 0;
            return true;
        }

        if self.cursor_line >= self.lines.len() {
            self.cursor_line = self.lines.len() - 1;
        }
        self.cursor_col = self.cursor_col.min(self.lines[self.cursor_line].len());
        true
    }

    pub fn clear(&mut self) {
        self.lines.clear();
        self.lines.push(String::new());
        self.cursor_line = 0;
        self.cursor_col = 0;
    }
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let is_focused = matches!(app.focused_pane(), FocusedPane::Editor);
    
    let border_style = if is_focused {
        Style::default().fg(ACCENT)
    } else {
        Style::default().fg(BORDER)
    };

    let title = if app.session_v2_enabled() {
        format!(
            " Query Editor [{}] • {} ({}) ",
            1,
            app.active_tab_title(),
            app.tab_position_text()
        )
    } else {
        " Query Editor [1] ".to_string()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(border_style);

    let inner = block.inner(area);
    f.render_widget(block, area);

    // Render editor content with line numbers
    let (cursor_line, cursor_col) = app.editor.cursor_position();
    
    let lines: Vec<Line> = app.editor.lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let line_num = format!("{:3} │ ", i + 1);
            let line_num_style = if i == cursor_line {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            // Basic syntax highlighting
            let content_spans = highlight_nql(line);
            
            let mut spans = vec![Span::styled(line_num, line_num_style)];
            spans.extend(content_spans);
            
            Line::from(spans)
        })
        .collect();

    let paragraph = Paragraph::new(lines)
        .style(Style::default().fg(FG).bg(BG))
        .wrap(Wrap { trim: false });

    f.render_widget(paragraph, inner);

    // Show cursor in insert mode
    if matches!(app.mode(), crate::app::Mode::Insert) && is_focused {
        // Calculate cursor position on screen
        let cursor_x = inner.x + 6 + cursor_col as u16; // 6 = line number width
        let cursor_y = inner.y + cursor_line as u16;
        
        if cursor_x < inner.right() && cursor_y < inner.bottom() {
            f.set_cursor(cursor_x, cursor_y);
        }
    }
}

/// Basic NQL syntax highlighting
fn highlight_nql(line: &str) -> Vec<Span<'_>> {
    let keywords = ["find", "from", "where", "order", "by", "limit", "add", "delete", "update"];
    let functions = ["pagerank", "shortestPath", "louvain", "betweenness"];
    
    let mut spans = Vec::new();
    let mut current = String::new();
    
    for word in line.split_whitespace() {
        if !current.is_empty() {
            spans.push(Span::raw(" "));
        }
        
        let style = if keywords.contains(&word.to_lowercase().as_str()) {
            Style::default().fg(Color::LightBlue).add_modifier(Modifier::BOLD)
        } else if functions.iter().any(|f| word.contains(f)) {
            Style::default().fg(Color::LightGreen)
        } else if word.starts_with('"') || word.starts_with('\'') {
            Style::default().fg(Color::LightYellow)
        } else if word.parse::<i64>().is_ok() || word.parse::<f64>().is_ok() {
            Style::default().fg(Color::LightMagenta)
        } else {
            Style::default().fg(FG)
        };
        
        spans.push(Span::styled(word.to_string(), style));
        current = word.to_string();
    }
    
    if spans.is_empty() {
        spans.push(Span::raw(line.to_string()));
    }
    
    spans
}
