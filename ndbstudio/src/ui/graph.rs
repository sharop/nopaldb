use std::collections::{HashMap, HashSet};

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Frame,
};

use crate::app::App;
use crate::ui::{ACCENT, BG, FG, SUCCESS};

#[derive(Clone)]
struct NodeEntry {
    id: String,
    label: String,
    degree: usize,
}

pub struct GraphView {
    nodes: Vec<NodeEntry>,
    adjacency: HashMap<String, Vec<String>>,
    edge_types: HashMap<(String, String), String>,
    focus_id: Option<String>,
    neighbor_candidates: Vec<String>,
    selected_neighbor: usize,
    depth: usize,
    label_filter: Option<String>,
    available_labels: Vec<String>,
    scroll_offset: usize,
    lines: Vec<String>,
}

impl GraphView {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            adjacency: HashMap::new(),
            edge_types: HashMap::new(),
            focus_id: None,
            neighbor_candidates: Vec::new(),
            selected_neighbor: 0,
            depth: 1,
            label_filter: None,
            available_labels: Vec::new(),
            scroll_offset: 0,
            lines: Vec::new(),
        }
    }

    pub fn set_snapshot(
        &mut self,
        nodes: Vec<(String, String)>,
        edges: Vec<(String, String, String)>,
    ) {
        self.nodes.clear();
        self.adjacency.clear();
        self.edge_types.clear();
        self.available_labels.clear();

        let mut degree_map: HashMap<String, usize> = HashMap::new();
        for (src, dst, edge_type) in &edges {
            *degree_map.entry(src.clone()).or_insert(0) += 1;
            *degree_map.entry(dst.clone()).or_insert(0) += 1;

            self.adjacency.entry(src.clone()).or_default().push(dst.clone());
            self.adjacency.entry(dst.clone()).or_default().push(src.clone());

            self.edge_types
                .insert((src.clone(), dst.clone()), edge_type.clone());
            self.edge_types
                .insert((dst.clone(), src.clone()), edge_type.clone());
        }

        let allowed_nodes: HashSet<String> = if let Some(filter) = &self.label_filter {
            nodes.iter()
                .filter(|(_, label)| label.eq_ignore_ascii_case(filter))
                .map(|(id, _)| id.clone())
                .collect()
        } else {
            nodes.iter().map(|(id, _)| id.clone()).collect()
        };

        self.nodes = nodes
            .into_iter()
            .filter(|(id, _)| allowed_nodes.contains(id))
            .map(|(id, label)| NodeEntry {
                degree: degree_map.get(&id).copied().unwrap_or(0),
                id,
                label,
            })
            .collect();

        let mut labels = self
            .nodes
            .iter()
            .map(|n| n.label.clone())
            .collect::<Vec<_>>();
        labels.sort();
        labels.dedup();
        self.available_labels = labels;

        self.adjacency.retain(|k, _| allowed_nodes.contains(k));
        for neighs in self.adjacency.values_mut() {
            neighs.retain(|n| allowed_nodes.contains(n));
        }
        self.edge_types
            .retain(|(src, dst), _| allowed_nodes.contains(src) && allowed_nodes.contains(dst));

        self.nodes.sort_by_key(|n| std::cmp::Reverse(n.degree));

        if self.nodes.is_empty() {
            self.focus_id = None;
            self.lines = vec!["Graph is empty".to_string()];
            return;
        }

        let focus_exists = self
            .focus_id
            .as_ref()
            .map(|focus| self.nodes.iter().any(|n| &n.id == focus))
            .unwrap_or(false);

        if !focus_exists {
            self.focus_id = Some(self.nodes[0].id.clone());
        }

        self.rebuild_lines();
    }

    pub fn scroll_down(&mut self) {
        if self.neighbor_candidates.is_empty() {
            if self.scroll_offset < self.lines.len().saturating_sub(1) {
                self.scroll_offset += 1;
            }
            return;
        }

        self.selected_neighbor = (self.selected_neighbor + 1) % self.neighbor_candidates.len();
        self.rebuild_lines();
    }

    pub fn scroll_up(&mut self) {
        if self.neighbor_candidates.is_empty() {
            if self.scroll_offset > 0 {
                self.scroll_offset -= 1;
            }
            return;
        }

        if self.selected_neighbor == 0 {
            self.selected_neighbor = self.neighbor_candidates.len().saturating_sub(1);
        } else {
            self.selected_neighbor -= 1;
        }
        self.rebuild_lines();
    }

    pub fn focus_selected_neighbor(&mut self) -> bool {
        let Some(next_focus) = self.neighbor_candidates.get(self.selected_neighbor).cloned() else {
            return false;
        };
        self.focus_id = Some(next_focus);
        self.rebuild_lines();
        true
    }

    pub fn increase_depth(&mut self) -> bool {
        if self.depth >= 3 {
            return false;
        }
        self.depth += 1;
        self.rebuild_lines();
        true
    }

    pub fn decrease_depth(&mut self) -> bool {
        if self.depth <= 1 {
            return false;
        }
        self.depth -= 1;
        self.rebuild_lines();
        true
    }

    pub fn depth(&self) -> usize {
        self.depth
    }

    pub fn set_label_filter(&mut self, label: Option<String>) {
        self.label_filter = label;
    }

    pub fn label_filter(&self) -> Option<&str> {
        self.label_filter.as_deref()
    }

    pub fn available_labels(&self) -> &[String] {
        &self.available_labels
    }

    pub fn suggest_label(&self, prefix: &str) -> Option<String> {
        let prefix = prefix.trim();
        if prefix.is_empty() {
            return self.available_labels.first().cloned();
        }

        let lower_prefix = prefix.to_lowercase();
        self.available_labels
            .iter()
            .find(|label| label.to_lowercase().starts_with(&lower_prefix))
            .cloned()
    }

    pub fn focus_by_id(&mut self, node_id: &str) -> bool {
        if self.nodes.iter().any(|n| n.id == node_id) {
            self.focus_id = Some(node_id.to_string());
            self.rebuild_lines();
            true
        } else {
            false
        }
    }

    pub fn focus_summary(&self) -> Option<String> {
        let focus = self.focus_id.as_ref()?;
        let node = self.nodes.iter().find(|n| &n.id == focus)?;
        Some(format!("{}:{}", node.label, short_id(&node.id)))
    }

    fn rebuild_lines(&mut self) {
        self.scroll_offset = 0;
        self.lines.clear();

        let Some(focus_id) = self.focus_id.clone() else {
            self.lines.push("No focus node".to_string());
            return;
        };

        let focus = self
            .nodes
            .iter()
            .find(|n| n.id == focus_id)
            .cloned()
            .unwrap_or_else(|| self.nodes[0].clone());

        self.lines.push("Neighborhood View".to_string());
        if let Some(filter) = self.label_filter() {
            self.lines.push(format!("  Label filter: {}", filter));
        } else {
            self.lines.push("  Label filter: *".to_string());
        }
        self.lines.push(format!("  Depth: {}", self.depth));
        self.lines.push(format!(
            "  Focus: {}:{} deg={}",
            focus.label,
            short_id(&focus.id),
            focus.degree
        ));
        self.lines.push(String::new());

        let mut neighbors = self
            .adjacency
            .get(&focus.id)
            .cloned()
            .unwrap_or_default();
        neighbors.sort_by_key(|id| std::cmp::Reverse(node_degree(&self.nodes, id)));
        neighbors.dedup();

        self.neighbor_candidates = neighbors.clone();
        if self.selected_neighbor >= self.neighbor_candidates.len() {
            self.selected_neighbor = 0;
        }

        self.lines.push("Neighbors".to_string());
        if neighbors.is_empty() {
            self.lines.push("  (no neighbors)".to_string());
        } else {
            for (idx, neighbor_id) in neighbors.iter().take(16).enumerate() {
                let neighbor = self
                    .nodes
                    .iter()
                    .find(|n| &n.id == neighbor_id)
                    .cloned();
                let marker = if idx == self.selected_neighbor { ">" } else { " " };
                if let Some(node) = neighbor {
                    let edge_type = self
                        .edge_types
                        .get(&(focus.id.clone(), node.id.clone()))
                        .cloned()
                        .unwrap_or_else(|| "REL".to_string());
                    self.lines.push(format!(
                        "{} {}:{} via {}",
                        marker,
                        node.label,
                        short_id(&node.id),
                        edge_type
                    ));
                }
            }
        }

        if self.depth > 1 {
            self.lines.push(String::new());
            self.lines.push("Reachability".to_string());

            let mut visited: HashSet<String> = HashSet::new();
            visited.insert(focus.id.clone());
            let mut frontier = vec![focus.id.clone()];

            for level in 1..=self.depth {
                let mut next = Vec::new();
                let mut level_count = 0usize;

                for src in &frontier {
                    let neighs = self.adjacency.get(src).cloned().unwrap_or_default();
                    for dst in neighs {
                        if visited.contains(&dst) {
                            continue;
                        }
                        visited.insert(dst.clone());
                        next.push(dst.clone());
                        level_count += 1;
                        if level_count <= 12 {
                            let src_label = node_label(&self.nodes, src);
                            let dst_label = node_label(&self.nodes, &dst);
                            self.lines.push(format!(
                                "  L{} {} -> {}",
                                level,
                                src_label,
                                dst_label
                            ));
                        }
                    }
                }

                if level_count == 0 {
                    self.lines.push(format!("  L{} (no new nodes)", level));
                    break;
                }

                if level_count > 12 {
                    self.lines
                        .push(format!("  L{} ... +{} more", level, level_count - 12));
                }

                frontier = next;
            }
        }
    }
}

pub fn draw(f: &mut Frame, area: Rect, app: &App) {
    let title = if let Some(updated) = app.graph_last_refresh_text() {
        format!(" Graph View [{}] ", updated)
    } else {
        " Graph View ".to_string()
    };
    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ACCENT));

    let inner = block.inner(area);
    f.render_widget(block, area);

    if app.graph_view.lines.is_empty() {
        let empty_message = Paragraph::new("No graph snapshot")
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(empty_message, inner);
        return;
    }

    let available_height = inner.height as usize;
    let visible_items: Vec<ListItem> = app
        .graph_view
        .lines
        .iter()
        .skip(app.graph_view.scroll_offset)
        .take(available_height)
        .map(|item| {
            let style = if item.starts_with('>') {
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else if item.starts_with("  ") {
                Style::default().fg(FG)
            } else if item.starts_with("Neighbors")
                || item.starts_with("Neighborhood")
                || item.starts_with("Reachability")
            {
                Style::default().fg(SUCCESS).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(FG)
            };
            ListItem::new(item.clone()).style(style)
        })
        .collect();

    let list = List::new(visible_items).style(Style::default().fg(FG).bg(BG));
    f.render_widget(list, inner);

    let label_filter = app.graph_view.label_filter().unwrap_or("*");
    let hint = format!(
        " Shift+J/K: select • o:focus • +/-:depth({}) • f:focus row • r:refresh • label:{} • x:hide ",
        app.graph_view.depth(),
        label_filter
    );
    let hint_y = area.bottom().saturating_sub(1);
    let hint_x = area.x + 1;

    if hint_x < area.right() && hint_y > area.y {
        let width = hint.len().min(area.width.saturating_sub(2) as usize) as u16;
        let hint_area = Rect::new(hint_x, hint_y, width, 1);
        let hint_widget = Paragraph::new(hint).style(Style::default().fg(Color::DarkGray));
        f.render_widget(hint_widget, hint_area);
    }
}

fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

fn node_label(nodes: &[NodeEntry], id: &str) -> String {
    nodes
        .iter()
        .find(|n| n.id == id)
        .map(|n| format!("{}:{}", n.label, short_id(&n.id)))
        .unwrap_or_else(|| short_id(id))
}

fn node_degree(nodes: &[NodeEntry], id: &str) -> usize {
    nodes
        .iter()
        .find(|n| n.id == id)
        .map(|n| n.degree)
        .unwrap_or(0)
}
