use anyhow::Result;
use chrono::{DateTime, Utc};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
};
use std::collections::VecDeque;

use crate::event::AppEvent;
use crate::renderer::Renderable;

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub is_streaming: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

impl MessageRole {
    pub fn style(&self) -> Style {
        match self {
            MessageRole::User => Style::default().fg(Color::Cyan),
            MessageRole::Assistant => Style::default().fg(Color::Green),
            MessageRole::System => Style::default().fg(Color::Yellow),
        }
    }

    pub fn prefix(&self) -> &str {
        match self {
            MessageRole::User => "You",
            MessageRole::Assistant => "AI",
            MessageRole::System => "System",
        }
    }
}

pub struct ChatView {
    pub(crate) messages: VecDeque<ChatMessage>,
    pub(crate) scroll_offset: u16,
    pub(crate) input_buffer: String,
    pub(crate) needs_redraw: bool,
    pub(crate) max_messages: usize,
}

impl ChatView {
    pub fn new() -> Self {
        let mut messages = VecDeque::new();

        // Add welcome message
        messages.push_back(ChatMessage {
            role: MessageRole::System,
            content: "Welcome to Arkavo Terminal UI! Press Tab to switch views, q to quit."
                .to_string(),
            timestamp: Utc::now(),
            is_streaming: false,
        });

        Self {
            messages,
            scroll_offset: 0,
            input_buffer: String::new(),
            needs_redraw: true,
            max_messages: 1000,
        }
    }
}

impl Default for ChatView {
    fn default() -> Self {
        Self::new()
    }
}

impl ChatView {
    pub fn add_message(&mut self, role: MessageRole, content: String) {
        let message = ChatMessage {
            role,
            content,
            timestamp: Utc::now(),
            is_streaming: false,
        };

        self.messages.push_back(message);

        if self.messages.len() > self.max_messages {
            self.messages.pop_front();
        }

        self.needs_redraw = true;
        self.scroll_to_bottom();
    }

    pub fn start_streaming_message(&mut self, role: MessageRole) {
        let message = ChatMessage {
            role,
            content: String::new(),
            timestamp: Utc::now(),
            is_streaming: true,
        };

        self.messages.push_back(message);
        self.needs_redraw = true;
    }

    pub fn append_to_streaming(&mut self, text: &str) {
        if let Some(last_msg) = self.messages.back_mut()
            && last_msg.is_streaming {
                last_msg.content.push_str(text);
                self.needs_redraw = true;
                self.scroll_to_bottom(); // Auto-scroll to show new content
            }
    }

    pub fn finish_streaming(&mut self) {
        if let Some(last_msg) = self.messages.back_mut() {
            last_msg.is_streaming = false;
            self.needs_redraw = true;
        }
    }

    pub(crate) fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    fn render_message<'a>(&self, msg: &'a ChatMessage, width: u16) -> ListItem<'a> {
        let role_style = msg.role.style();
        let timestamp = msg.timestamp.format("%H:%M:%S").to_string();

        let header = Line::from(vec![
            Span::styled(
                format!("[{timestamp}] "),
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled(msg.role.prefix(), role_style.add_modifier(Modifier::BOLD)),
            Span::raw(":"),
        ]);

        let mut lines = vec![header];

        // Split content into lines and wrap if needed
        let content_lines: Vec<&str> = msg.content.lines().collect();
        for line in content_lines {
            if line.is_empty() {
                lines.push(Line::default());
            } else {
                // Simple word wrapping
                let wrapped = textwrap::wrap(line, width as usize - 4);
                for wrapped_line in wrapped {
                    lines.push(Line::from(Span::raw(wrapped_line.to_string())));
                }
            }
        }

        if msg.is_streaming {
            lines.push(Line::from(Span::styled(
                "▊",
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::SLOW_BLINK),
            )));
        }

        // Add spacing between messages
        lines.push(Line::from(""));

        ListItem::new(Text::from(lines))
    }

    pub fn handle_event(&mut self, event: AppEvent) -> Result<()> {
        if let AppEvent::KeyPress(key) = event {
            use crossterm::event::KeyCode;
            match key.code {
                KeyCode::Up => {
                    if self.scroll_offset < self.messages.len() as u16 {
                        self.scroll_offset += 1;
                        self.needs_redraw = true;
                    }
                }
                KeyCode::Down => {
                    if self.scroll_offset > 0 {
                        self.scroll_offset -= 1;
                        self.needs_redraw = true;
                    }
                }
                KeyCode::PageUp => {
                    self.scroll_offset = (self.scroll_offset + 10).min(self.messages.len() as u16);
                    self.needs_redraw = true;
                }
                KeyCode::PageDown => {
                    self.scroll_offset = self.scroll_offset.saturating_sub(10);
                    self.needs_redraw = true;
                }
                KeyCode::Home => {
                    self.scroll_offset = self.messages.len() as u16;
                    self.needs_redraw = true;
                }
                KeyCode::End => {
                    self.scroll_offset = 0;
                    self.needs_redraw = true;
                }
                KeyCode::Char(c) => {
                    self.input_buffer.push(c);
                    self.needs_redraw = true;
                }
                KeyCode::Backspace => {
                    self.input_buffer.pop();
                    self.needs_redraw = true;
                }
                KeyCode::Enter => {
                    // Enter is handled by the App now for LLM integration
                    // Return early to prevent double handling
                    return Ok(());
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl Renderable for ChatView {
    fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),    // Messages area
                Constraint::Length(3), // Input area
            ])
            .split(area);

        // Render messages with scroll indicator
        let title = if self.scroll_offset > 0 {
            format!("Chat [↑ {} more]", self.scroll_offset)
        } else {
            "Chat".to_string()
        };

        let messages_block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White));

        let inner_area = messages_block.inner(chunks[0]);

        // Convert messages to list items
        let items: Vec<ListItem> = self
            .messages
            .iter()
            .map(|msg| self.render_message(msg, inner_area.width))
            .collect();

        // Calculate total height and scrolling
        let total_height: usize = items.iter().map(ratatui::widgets::ListItem::height).sum();
        let visible_height = inner_area.height as usize;

        // Implement proper scrolling
        if total_height <= visible_height {
            // All messages fit, no scrolling needed
            let list = List::new(items)
                .block(messages_block)
                .style(Style::default().fg(Color::White));
            frame.render_widget(list, chunks[0]);
        } else {
            // Need to scroll - show the bottom messages when scroll_offset is 0
            let mut current_height = 0;
            let mut start_idx = items.len();

            // When scroll_offset is 0, we want to show the bottom (most recent) messages
            // Find the starting index that fills the visible area from bottom
            for (i, item) in items.iter().enumerate().rev() {
                current_height += item.height();
                if current_height >= visible_height + self.scroll_offset as usize {
                    start_idx = i;
                    break;
                }
            }

            // Take only the visible items
            let visible_items: Vec<ListItem> = items.into_iter().skip(start_idx).collect();

            let list = List::new(visible_items)
                .block(messages_block)
                .style(Style::default().fg(Color::White));
            frame.render_widget(list, chunks[0]);
        }

        // Render input area
        let input_block = Block::default()
            .title("Input (Press Enter to send)")
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White));

        // Add cursor indicator
        let input_with_cursor = format!("{}_", self.input_buffer);
        let input_text = Paragraph::new(input_with_cursor)
            .style(Style::default().fg(Color::White))
            .wrap(Wrap { trim: false })
            .block(input_block);

        frame.render_widget(input_text, chunks[1]);

        self.needs_redraw = false;
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}

// Helper for text wrapping
pub(crate) mod textwrap {
    #[allow(unreachable_pub)]
    pub fn wrap(text: &str, width: usize) -> Vec<String> {
        let mut result = Vec::new();
        let mut current_line = String::new();

        for word in text.split_whitespace() {
            if current_line.is_empty() {
                current_line = word.to_string();
            } else if current_line.len() + 1 + word.len() <= width {
                current_line.push(' ');
                current_line.push_str(word);
            } else {
                result.push(current_line);
                current_line = word.to_string();
            }
        }

        if !current_line.is_empty() {
            result.push(current_line);
        }

        if result.is_empty() {
            result.push(String::new());
        }

        result
    }
}
