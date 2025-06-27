use chrono::{DateTime, Utc};
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::collections::VecDeque;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct TaskWindow {
    pub id: Uuid,
    pub agent_name: String,
    pub model_name: String,
    pub messages: VecDeque<TaskMessage>,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub scroll_offset: u16,
}

#[derive(Debug, Clone)]
pub struct TaskMessage {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskStatus {
    Idle,
    Processing,
    Streaming,
    Error,
    Complete,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

impl TaskWindow {
    pub fn new(agent_name: String, model_name: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            agent_name,
            model_name,
            messages: VecDeque::new(),
            status: TaskStatus::Idle,
            created_at: Utc::now(),
            scroll_offset: 0,
        }
    }

    pub fn add_message(&mut self, role: MessageRole, content: String) {
        self.messages.push_back(TaskMessage {
            role,
            content,
            timestamp: Utc::now(),
        });
    }

    pub fn set_status(&mut self, status: TaskStatus) {
        self.status = status;
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    pub fn render(&self, frame: &mut ratatui::Frame, area: Rect, is_active: bool) {
        let status_color = match self.status {
            TaskStatus::Idle => Color::Gray,
            TaskStatus::Processing => Color::Yellow,
            TaskStatus::Streaming => Color::Cyan,
            TaskStatus::Error => Color::Red,
            TaskStatus::Complete => Color::Green,
        };

        let status_text = match self.status {
            TaskStatus::Idle => "●",
            TaskStatus::Processing => "◐",
            TaskStatus::Streaming => "◉",
            TaskStatus::Error => "✗",
            TaskStatus::Complete => "✓",
        };

        let title = format!(
            " {} {} - {} ",
            status_text, self.agent_name, self.model_name
        );

        let block = Block::default()
            .title(title)
            .title_style(Style::default().fg(status_color))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(if is_active {
                Color::Cyan
            } else if self.status == TaskStatus::Processing || self.status == TaskStatus::Streaming
            {
                Color::Yellow
            } else {
                Color::DarkGray
            }));

        let inner = block.inner(area);
        frame.render_widget(block, area);

        // Render messages
        let mut lines: Vec<Line> = Vec::new();

        for msg in &self.messages {
            let role_style = match msg.role {
                MessageRole::User => Style::default().fg(Color::Cyan),
                MessageRole::Assistant => Style::default().fg(Color::Green),
                MessageRole::System => Style::default().fg(Color::Yellow),
            };

            let role_prefix = match msg.role {
                MessageRole::User => "You",
                MessageRole::Assistant => &self.model_name,
                MessageRole::System => "System",
            };

            lines.push(Line::from(vec![
                Span::styled(format!("[{}] ", role_prefix), role_style),
                Span::raw(&msg.content),
            ]));
            lines.push(Line::from(""));
        }

        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .scroll((self.scroll_offset, 0));

        frame.render_widget(paragraph, inner);
    }
}

pub struct TaskManager {
    pub tasks: Vec<TaskWindow>,
    pub active_task: Option<usize>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Vec::new(),
            active_task: None,
        }
    }

    pub fn create_task(&mut self, agent_name: String, model_name: String) -> &mut TaskWindow {
        let task = TaskWindow::new(agent_name, model_name);
        self.tasks.push(task);
        let idx = self.tasks.len() - 1;
        self.active_task = Some(idx);
        &mut self.tasks[idx]
    }

    pub fn get_active_task(&mut self) -> Option<&mut TaskWindow> {
        self.active_task.and_then(|idx| self.tasks.get_mut(idx))
    }

    pub fn next_task(&mut self) {
        if !self.tasks.is_empty() {
            self.active_task = Some(
                self.active_task
                    .map(|idx| (idx + 1) % self.tasks.len())
                    .unwrap_or(0),
            );
        }
    }

    pub fn prev_task(&mut self) {
        if !self.tasks.is_empty() {
            self.active_task = Some(
                self.active_task
                    .map(|idx| {
                        if idx == 0 {
                            self.tasks.len() - 1
                        } else {
                            idx - 1
                        }
                    })
                    .unwrap_or(0),
            );
        }
    }

    pub fn remove_completed_tasks(&mut self) {
        self.tasks
            .retain(|task| task.status != TaskStatus::Complete);
        if self.active_task.is_some() && self.tasks.is_empty() {
            self.active_task = None;
        }
    }
}

impl Default for TaskManager {
    fn default() -> Self {
        Self::new()
    }
}
