use anyhow::Result;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use syntect::{
    easy::HighlightLines,
    highlighting::{Style as SyntectStyle, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

use crate::event::AppEvent;
use crate::renderer::Renderable;

pub struct CodeView {
    content: String,
    language: String,
    syntax_set: SyntaxSet,
    theme_set: ThemeSet,
    scroll_offset: u16,
    needs_redraw: bool,
    line_numbers: bool,
}

impl CodeView {
    pub fn new() -> Self {
        Self {
            content: String::new(),
            language: "rust".to_string(),
            syntax_set: SyntaxSet::load_defaults_newlines(),
            theme_set: ThemeSet::load_defaults(),
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
            self.language = lang;
        }
        self.needs_redraw = true;
        self.scroll_offset = 0;
    }

    pub fn detect_language(&mut self, filename: &str) {
        if let Some(syntax) = self
            .syntax_set
            .find_syntax_for_file(filename)
            .ok()
            .flatten()
        {
            self.language = syntax.name.to_lowercase();
            self.needs_redraw = true;
        }
    }

    fn syntect_to_ratatui_style(&self, style: &SyntectStyle) -> Style {
        let mut ratatui_style = Style::default();

        // Convert foreground color
        ratatui_style = ratatui_style.fg(Color::Rgb(
            style.foreground.r,
            style.foreground.g,
            style.foreground.b,
        ));

        // Convert font style
        if style
            .font_style
            .contains(syntect::highlighting::FontStyle::BOLD)
        {
            ratatui_style = ratatui_style.add_modifier(Modifier::BOLD);
        }
        if style
            .font_style
            .contains(syntect::highlighting::FontStyle::ITALIC)
        {
            ratatui_style = ratatui_style.add_modifier(Modifier::ITALIC);
        }
        if style
            .font_style
            .contains(syntect::highlighting::FontStyle::UNDERLINE)
        {
            ratatui_style = ratatui_style.add_modifier(Modifier::UNDERLINED);
        }

        ratatui_style
    }

    fn highlight_lines(&self) -> Vec<Line> {
        let mut highlighted_lines = Vec::new();

        let syntax = self
            .syntax_set
            .find_syntax_by_name(&self.language)
            .or_else(|| self.syntax_set.find_syntax_by_extension(&self.language))
            .unwrap_or_else(|| self.syntax_set.find_syntax_plain_text());

        let theme = &self.theme_set.themes["base16-ocean.dark"];
        let mut highlighter = HighlightLines::new(syntax, theme);

        for (line_num, line) in LinesWithEndings::from(&self.content).enumerate() {
            let mut spans = Vec::new();

            // Add line number if enabled
            if self.line_numbers {
                spans.push(Span::styled(
                    format!("{:4} │ ", line_num + 1),
                    Style::default().fg(Color::DarkGray),
                ));
            }

            // Highlight the line
            let highlighted = highlighter
                .highlight_line(line, &self.syntax_set)
                .unwrap_or_default();

            for (style, text) in highlighted {
                spans.push(Span::styled(
                    text.to_string(),
                    self.syntect_to_ratatui_style(&style),
                ));
            }

            highlighted_lines.push(Line::from(spans));
        }

        highlighted_lines
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

        let highlighted_lines = self.highlight_lines();

        // Calculate visible range
        let visible_height = inner_area.height as usize;
        let start = self.scroll_offset as usize;
        let end = (start + visible_height).min(highlighted_lines.len());

        if start < highlighted_lines.len() {
            let visible_lines: Vec<Line> = highlighted_lines[start..end].to_vec();

            let paragraph = Paragraph::new(visible_lines)
                .style(Style::default())
                .wrap(Wrap { trim: false });

            frame.render_widget(paragraph, inner_area);
        }

        // Render scroll indicator
        if highlighted_lines.len() > visible_height {
            let scroll_percentage = if highlighted_lines.len() > 1 {
                (self.scroll_offset as f32 / (highlighted_lines.len() - 1) as f32 * 100.0) as u16
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
