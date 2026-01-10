use anyhow::Result;
use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};

use crate::event::AppEvent;
use crate::renderer::Renderable;

pub struct CodeView {
    content: String,
    language: String,
    scroll_offset: u16,
    needs_redraw: bool,
    line_numbers: bool,
}

impl CodeView {
    pub fn new() -> Self {
        Self {
            content: String::new(),
            language: "rust".to_string(),
            scroll_offset: 0,
            needs_redraw: true,
            line_numbers: true,
        }
    }
}

impl Default for CodeView {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeView {
    pub fn set_content(&mut self, content: String, language: Option<String>) {
        self.content = content;
        if let Some(lang) = language {
            self.language = lang.to_lowercase();
        }
        self.needs_redraw = true;
        self.scroll_offset = 0;
    }

    pub fn detect_language(&mut self, filename: &str) {
        let lang = match filename.rsplit('.').next() {
            Some("rs") => "rust",
            Some("py") => "python",
            Some("js") => "javascript",
            Some("ts") => "typescript",
            Some("tsx") => "typescript",
            Some("jsx") => "javascript",
            Some("go") => "go",
            _ => return,
        };
        self.language = lang.to_string();
        self.needs_redraw = true;
    }

    fn plain_text_lines(&self) -> Vec<Line<'_>> {
        self.content
            .lines()
            .enumerate()
            .map(|(line_num, line)| {
                let mut spans = Vec::new();
                if self.line_numbers {
                    spans.push(Span::styled(
                        format!("{:4} │ ", line_num + 1),
                        Style::default().fg(Color::DarkGray),
                    ));
                }
                spans.push(Span::raw(line.to_string()));
                Line::from(spans)
            })
            .collect()
    }

    pub fn handle_event(&mut self, event: AppEvent) -> Result<()> {
        if let AppEvent::KeyPress(key) = event {
            use crossterm::event::KeyCode;
            match key.code {
                KeyCode::Up => {
                    if self.scroll_offset > 0 {
                        self.scroll_offset -= 1;
                        self.needs_redraw = true;
                    }
                }
                KeyCode::Down => {
                    let line_count = self.content.lines().count() as u16;
                    if self.scroll_offset < line_count.saturating_sub(1) {
                        self.scroll_offset += 1;
                        self.needs_redraw = true;
                    }
                }
                KeyCode::PageUp => {
                    self.scroll_offset = self.scroll_offset.saturating_sub(10);
                    self.needs_redraw = true;
                }
                KeyCode::PageDown => {
                    let line_count = self.content.lines().count() as u16;
                    self.scroll_offset =
                        (self.scroll_offset + 10).min(line_count.saturating_sub(1));
                    self.needs_redraw = true;
                }
                KeyCode::Home => {
                    self.scroll_offset = 0;
                    self.needs_redraw = true;
                }
                KeyCode::End => {
                    let line_count = self.content.lines().count() as u16;
                    self.scroll_offset = line_count.saturating_sub(1);
                    self.needs_redraw = true;
                }
                KeyCode::Char('n') => {
                    self.line_numbers = !self.line_numbers;
                    self.needs_redraw = true;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl Renderable for CodeView {
    fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .title(format!("Code [{}]", self.language))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White));

        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        let lines = self.plain_text_lines();

        // Calculate visible range
        let visible_height = inner_area.height as usize;
        let start = self.scroll_offset as usize;
        let end = (start + visible_height).min(lines.len());

        if start < lines.len() {
            let visible_lines: Vec<Line> = lines[start..end].to_vec();

            let paragraph = Paragraph::new(visible_lines)
                .style(Style::default())
                .wrap(Wrap { trim: false });

            frame.render_widget(paragraph, inner_area);
        }

        // Render scroll indicator
        if lines.len() > visible_height {
            let scroll_percentage = if lines.len() > 1 {
                (f32::from(self.scroll_offset) / (lines.len() - 1) as f32 * 100.0) as u16
            } else {
                0
            };

            let scroll_indicator = format!(" {}% ", scroll_percentage);
            let indicator_x = area.right() - scroll_indicator.len() as u16 - 1;
            let indicator_y = area.top();

            frame.render_widget(
                Paragraph::new(scroll_indicator.clone())
                    .style(Style::default().fg(Color::DarkGray)),
                Rect::new(indicator_x, indicator_y, scroll_indicator.len() as u16, 1),
            );
        }

        self.needs_redraw = false;
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}