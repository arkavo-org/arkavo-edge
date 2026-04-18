use anyhow::Result;
use chrono::{DateTime, Utc};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};
use std::collections::VecDeque;

use crate::event::AppEvent;
use crate::renderer::Renderable;

#[derive(Debug, Clone)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogLevel {
    Info,
    Debug,
    Warning,
    Error,
}

impl LogLevel {
    pub fn style(&self) -> Style {
        match self {
            LogLevel::Info => Style::default().fg(Color::White),
            LogLevel::Debug => Style::default().fg(Color::Gray),
            LogLevel::Warning => Style::default().fg(Color::Yellow),
            LogLevel::Error => Style::default().fg(Color::Red),
        }
    }

    pub fn prefix(&self) -> &str {
        match self {
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
            LogLevel::Warning => "WARN",
            LogLevel::Error => "ERROR",
        }
    }
}

pub struct DebugView {
    logs: VecDeque<LogEntry>,
    scroll_offset: u16,
    max_logs: usize,
    needs_redraw: bool,
}

impl DebugView {
    pub fn new() -> Self {
        let mut view = Self {
            logs: VecDeque::new(),
            scroll_offset: 0,
            max_logs: 10000,
            needs_redraw: true,
        };

        view.add_log(LogLevel::Info, "Debug view initialized".to_string());
        view
    }

    pub fn add_log(&mut self, level: LogLevel, message: String) {
        let entry = LogEntry {
            timestamp: Utc::now(),
            level,
            message,
        };

        self.logs.push_back(entry);

        if self.logs.len() > self.max_logs {
            self.logs.pop_front();
        }

        self.needs_redraw = true;
        self.scroll_to_bottom();
    }

    fn scroll_to_bottom(&mut self) {
        self.scroll_offset = 0;
    }

    pub fn handle_event(&mut self, event: AppEvent) -> Result<()> {
        if let AppEvent::KeyPress(key) = event {
            use crossterm::event::KeyCode;
            match key.code {
                KeyCode::Up if self.scroll_offset < self.logs.len() as u16 => {
                    self.scroll_offset += 1;
                    self.needs_redraw = true;
                }
                KeyCode::Down if self.scroll_offset > 0 => {
                    self.scroll_offset -= 1;
                    self.needs_redraw = true;
                }
                KeyCode::PageUp => {
                    self.scroll_offset = (self.scroll_offset + 10).min(self.logs.len() as u16);
                    self.needs_redraw = true;
                }
                KeyCode::PageDown => {
                    self.scroll_offset = self.scroll_offset.saturating_sub(10);
                    self.needs_redraw = true;
                }
                KeyCode::Home => {
                    self.scroll_offset = self.logs.len() as u16;
                    self.needs_redraw = true;
                }
                KeyCode::End => {
                    self.scroll_offset = 0;
                    self.needs_redraw = true;
                }
                KeyCode::Char('c') => {
                    // Clear logs
                    self.logs.clear();
                    self.add_log(LogLevel::Info, "Logs cleared".to_string());
                    self.scroll_offset = 0;
                    self.needs_redraw = true;
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn get_all_logs(&self) -> Vec<serde_json::Value> {
        self.logs
            .iter()
            .map(|log| {
                serde_json::json!({
                    "timestamp": log.timestamp.to_rfc3339(),
                    "level": log.level.prefix(),
                    "message": log.message,
                })
            })
            .collect()
    }
}

impl Default for DebugView {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderable for DebugView {
    fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let title = if self.scroll_offset > 0 {
            format!("Debug Logs [↑ {} more] (c: clear)", self.scroll_offset)
        } else {
            "Debug Logs (c: clear)".to_string()
        };

        let block = Block::default()
            .title(title)
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White));

        let inner_area = block.inner(area);

        // Convert logs to list items
        let items: Vec<ListItem> = self
            .logs
            .iter()
            .map(|log| {
                let timestamp = log.timestamp.format("%H:%M:%S.%3f").to_string();
                let line = Line::from(vec![
                    Span::styled(
                        format!("[{timestamp}] "),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(format!("{:5} ", log.level.prefix()), log.level.style()),
                    Span::raw(log.message.clone()),
                ]);
                ListItem::new(line)
            })
            .collect();

        // Calculate visible range
        let total_logs = items.len();
        let visible_height = inner_area.height as usize;

        if total_logs <= visible_height {
            // All logs fit
            let list = List::new(items)
                .block(block)
                .style(Style::default().fg(Color::White));
            frame.render_widget(list, area);
        } else {
            // Need scrolling
            let start_idx = if self.scroll_offset as usize >= total_logs {
                0
            } else {
                total_logs.saturating_sub(visible_height + self.scroll_offset as usize)
            };

            let visible_items: Vec<ListItem> = items
                .into_iter()
                .skip(start_idx)
                .take(visible_height)
                .collect();

            let list = List::new(visible_items)
                .block(block)
                .style(Style::default().fg(Color::White));
            frame.render_widget(list, area);
        }

        self.needs_redraw = false;
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}
