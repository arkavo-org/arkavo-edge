use arkavo_dataflow::dsl::{Blueprint, NodeKind};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use std::collections::HashMap;

/// ASCII renderer for dataflow blueprints
pub struct DataflowView {
    blueprint: Option<Blueprint>,
    selected_node: Option<String>,
    /// Node positions for layout
    node_positions: HashMap<String, (u16, u16)>,
}

impl DataflowView {
    pub fn new() -> Self {
        Self {
            blueprint: None,
            selected_node: None,
            node_positions: HashMap::new(),
        }
    }

    pub fn set_blueprint(&mut self, blueprint: Blueprint) {
        self.blueprint = Some(blueprint);
        self.calculate_layout();
    }

    pub fn select_node(&mut self, node_id: String) {
        self.selected_node = Some(node_id);
    }

    pub fn clear_selection(&mut self) {
        self.selected_node = None;
    }

    /// Calculate node positions for rendering
    fn calculate_layout(&mut self) {
        self.node_positions.clear();

        if let Some(blueprint) = &self.blueprint {
            let nodes = &blueprint.nodes;
            let node_count = nodes.len();

            if node_count == 0 {
                return;
            }

            // Simple grid layout for now
            let cols = ((node_count as f32).sqrt().ceil()) as u16;
            let _rows = ((node_count as f32) / f32::from(cols)).ceil() as u16;

            for (idx, node) in nodes.iter().enumerate() {
                let col = (idx as u16) % cols;
                let row = (idx as u16) / cols;

                // Space nodes with padding
                let x = col * 20 + 5;
                let y = row * 6 + 2;

                self.node_positions.insert(node.id.clone(), (x, y));
            }
        }
    }

    pub fn render(&self, frame: &mut Frame, area: Rect) {
        let block = Block::default()
            .title(" Dataflow Pipeline ")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        if let Some(blueprint) = &self.blueprint {
            // Render nodes
            for node in &blueprint.nodes {
                if let Some(&(x, y)) = self.node_positions.get(&node.id) {
                    let node_type = match &node.kind {
                        NodeKind::Source => "Source",
                        NodeKind::Transform => "Transform",
                        NodeKind::Sink => "Sink",
                        NodeKind::Router => "Router",
                    };
                    self.render_node(frame, inner, node.id.as_str(), node_type, x, y);
                }
            }

            // Render links (connections)
            for link in &blueprint.links {
                self.render_connection(frame, inner, &link.from, &link.to);
            }
        } else {
            // No blueprint loaded
            let message = Paragraph::new("No dataflow pipeline loaded")
                .style(Style::default().fg(Color::DarkGray))
                .alignment(Alignment::Center);

            let centered_area = centered_rect(50, 20, inner);
            frame.render_widget(message, centered_area);
        }
    }

    fn render_node(
        &self,
        frame: &mut Frame,
        area: Rect,
        id: &str,
        node_type: &str,
        x: u16,
        y: u16,
    ) {
        // Node dimensions
        let width = 15;
        let height = 3;

        // Check bounds
        if x + width > area.width || y + height > area.height {
            return;
        }

        let node_area = Rect {
            x: area.x + x,
            y: area.y + y,
            width,
            height,
        };

        let is_selected = self.selected_node.as_ref() == Some(&id.to_string());

        let style = if is_selected {
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Cyan)
        };

        let block = Block::default().borders(Borders::ALL).style(style);

        let inner = block.inner(node_area);
        frame.render_widget(block, node_area);

        // Render node content
        let content = vec![
            Line::from(Span::raw(node_type)),
            Line::from(Span::styled(id, Style::default().fg(Color::DarkGray))),
        ];

        let paragraph = Paragraph::new(content).alignment(Alignment::Center);

        frame.render_widget(paragraph, inner);
    }

    fn render_connection(&self, frame: &mut Frame, area: Rect, from: &str, to: &str) {
        let from_pos = self.node_positions.get(from);
        let to_pos = self.node_positions.get(to);

        if let (Some(&(x1, y1)), Some(&(x2, y2))) = (from_pos, to_pos) {
            // Simple ASCII arrow for now
            // In a full implementation, this would draw proper lines
            let arrow_x = area.x + x1 + 15;
            let arrow_y = area.y + y1 + 1;

            if arrow_x < area.right() && arrow_y < area.bottom() {
                let arrow = if x2 > x1 {
                    "→"
                } else if x2 < x1 {
                    "←"
                } else if y2 > y1 {
                    "↓"
                } else {
                    "↑"
                };

                let span = Span::styled(arrow, Style::default().fg(Color::Green));
                frame.render_widget(
                    Paragraph::new(Line::from(span)),
                    Rect {
                        x: arrow_x,
                        y: arrow_y,
                        width: 1,
                        height: 1,
                    },
                );
            }
        }
    }
}

impl Default for DataflowView {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to create a centered rect
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_dataflow::dsl::{Link, Node};

    #[test]
    fn test_dataflow_view_creation() {
        let view = DataflowView::new();
        assert!(view.blueprint.is_none());
        assert!(view.selected_node.is_none());
        assert!(view.node_positions.is_empty());
    }

    #[test]
    fn test_set_blueprint() {
        let mut view = DataflowView::new();

        let blueprint = Blueprint {
            version: semver::Version::new(1, 0, 0),
            name: Some("Test Pipeline".to_string()),
            description: Some("Test description".to_string()),
            nodes: vec![
                Node {
                    id: "node1".to_string(),
                    kind: NodeKind::Source,
                    params: HashMap::new(),
                    auth_ref: None,
                },
                Node {
                    id: "node2".to_string(),
                    kind: NodeKind::Transform,
                    params: HashMap::new(),
                    auth_ref: None,
                },
            ],
            links: vec![Link {
                from: "node1".to_string(),
                to: "node2".to_string(),
                rule: None,
            }],
            metadata: None,
        };

        view.set_blueprint(blueprint);
        assert!(view.blueprint.is_some());
        assert_eq!(view.node_positions.len(), 2);
    }
}
