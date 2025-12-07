use anyhow::Result;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::collections::HashMap;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

use crate::event::AppEvent;
use crate::renderer::Renderable;

/// Highlight capture names that tree-sitter uses
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "comment",
    "constant",
    "constant.builtin",
    "constructor",
    "embedded",
    "function",
    "function.builtin",
    "function.macro",
    "keyword",
    "label",
    "number",
    "operator",
    "property",
    "punctuation",
    "punctuation.bracket",
    "punctuation.delimiter",
    "punctuation.special",
    "string",
    "string.special",
    "tag",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "variable.parameter",
];

pub struct CodeView {
    content: String,
    language: String,
    configs: HashMap<String, HighlightConfiguration>,
    scroll_offset: u16,
    needs_redraw: bool,
    line_numbers: bool,
}

impl CodeView {
    pub fn new() -> Self {
        let mut view = Self {
            content: String::new(),
            language: "rust".to_string(),
            configs: HashMap::new(),
            scroll_offset: 0,
            needs_redraw: true,
            line_numbers: true,
        };
        view.init_languages();
        view
    }

    fn init_languages(&mut self) {
        // Initialize Rust
        if let Ok(mut config) = HighlightConfiguration::new(
            tree_sitter_rust::LANGUAGE.into(),
            "rust",
            tree_sitter_rust::HIGHLIGHTS_QUERY,
            tree_sitter_rust::INJECTIONS_QUERY,
            "",
        ) {
            config.configure(HIGHLIGHT_NAMES);
            self.configs.insert("rust".to_string(), config);
        }

        // Initialize Python
        if let Ok(mut config) = HighlightConfiguration::new(
            tree_sitter_python::LANGUAGE.into(),
            "python",
            tree_sitter_python::HIGHLIGHTS_QUERY,
            "",
            "",
        ) {
            config.configure(HIGHLIGHT_NAMES);
            self.configs.insert("python".to_string(), config);
        }

        // Initialize JavaScript
        if let Ok(mut config) = HighlightConfiguration::new(
            tree_sitter_javascript::LANGUAGE.into(),
            "javascript",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ) {
            config.configure(HIGHLIGHT_NAMES);
            self.configs.insert("javascript".to_string(), config);
        }
        // Also add "js" alias
        if let Ok(mut config) = HighlightConfiguration::new(
            tree_sitter_javascript::LANGUAGE.into(),
            "javascript",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::INJECTIONS_QUERY,
            tree_sitter_javascript::LOCALS_QUERY,
        ) {
            config.configure(HIGHLIGHT_NAMES);
            self.configs.insert("js".to_string(), config);
        }

        // Initialize TypeScript
        if let Ok(mut config) = HighlightConfiguration::new(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "typescript",
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        ) {
            config.configure(HIGHLIGHT_NAMES);
            self.configs.insert("typescript".to_string(), config);
        }
        // Also add "ts" alias
        if let Ok(mut config) = HighlightConfiguration::new(
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "typescript",
            tree_sitter_typescript::HIGHLIGHTS_QUERY,
            "",
            tree_sitter_typescript::LOCALS_QUERY,
        ) {
            config.configure(HIGHLIGHT_NAMES);
            self.configs.insert("ts".to_string(), config);
        }

        // Initialize Go
        if let Ok(mut config) = HighlightConfiguration::new(
            tree_sitter_go::LANGUAGE.into(),
            "go",
            tree_sitter_go::HIGHLIGHTS_QUERY,
            "",
            "",
        ) {
            config.configure(HIGHLIGHT_NAMES);
            self.configs.insert("go".to_string(), config);
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

    fn capture_to_style(capture_index: usize) -> Style {
        // Map highlight capture indices to colors (base16-ocean inspired)
        match HIGHLIGHT_NAMES.get(capture_index) {
            Some(&"keyword") => Style::default().fg(Color::Rgb(180, 142, 173)), // Purple
            Some(&"string") | Some(&"string.special") => {
                Style::default().fg(Color::Rgb(163, 190, 140))
            } // Green
            Some(&"comment") => Style::default()
                .fg(Color::Rgb(101, 115, 126))
                .add_modifier(Modifier::ITALIC), // Gray italic
            Some(&"function") | Some(&"function.builtin") | Some(&"function.macro") => {
                Style::default().fg(Color::Rgb(143, 161, 179))
            } // Blue
            Some(&"type") | Some(&"type.builtin") => {
                Style::default().fg(Color::Rgb(235, 203, 139))
            } // Yellow
            Some(&"number") | Some(&"constant") | Some(&"constant.builtin") => {
                Style::default().fg(Color::Rgb(208, 135, 112))
            } // Orange
            Some(&"operator") => Style::default().fg(Color::Rgb(192, 197, 206)), // Light gray
            Some(&"variable") | Some(&"variable.builtin") | Some(&"variable.parameter") => {
                Style::default().fg(Color::Rgb(191, 97, 106))
            } // Red
            Some(&"property") => Style::default().fg(Color::Rgb(143, 161, 179)), // Blue
            Some(&"punctuation") | Some(&"punctuation.bracket") | Some(&"punctuation.delimiter") => {
                Style::default().fg(Color::Rgb(192, 197, 206))
            } // Light gray
            Some(&"attribute") => Style::default().fg(Color::Rgb(235, 203, 139)), // Yellow
            Some(&"constructor") => Style::default().fg(Color::Rgb(235, 203, 139)), // Yellow
            Some(&"tag") => Style::default().fg(Color::Rgb(191, 97, 106)),        // Red
            Some(&"label") => Style::default().fg(Color::Rgb(180, 142, 173)),     // Purple
            _ => Style::default().fg(Color::Rgb(192, 197, 206)),                  // Default light gray
        }
    }

    fn highlight_lines(&self) -> Vec<Line<'_>> {
        let mut highlighted_lines: Vec<Line<'_>> = Vec::new();
        let lines: Vec<&str> = self.content.lines().collect();

        // Check if we have a config for this language
        let config = match self.configs.get(&self.language) {
            Some(c) => c,
            None => {
                // Fall back to plain text rendering
                return self.plain_text_lines();
            }
        };

        let mut highlighter = Highlighter::new();
        let source = self.content.as_bytes();

        let highlights = match highlighter.highlight(config, source, None, |_| None) {
            Ok(h) => h,
            Err(_) => {
                return self.plain_text_lines();
            }
        };

        let mut current_line_spans: Vec<Span<'_>> = Vec::new();
        let mut current_line_num = 0;
        let mut current_style = Style::default();

        // Add line number for first line
        if self.line_numbers {
            current_line_spans.push(Span::styled(
                format!("{:4} │ ", current_line_num + 1),
                Style::default().fg(Color::DarkGray),
            ));
        }

        for event in highlights.flatten() {
            match event {
                HighlightEvent::Source { start, end } => {
                    let text = match std::str::from_utf8(&source[start..end]) {
                        Ok(t) => t,
                        Err(_) => continue,
                    };

                    // Split by newlines and handle each part
                    for (i, part) in text.split('\n').enumerate() {
                        if i > 0 {
                            // We hit a newline, push current line and start new one
                            highlighted_lines.push(Line::from(current_line_spans));
                            current_line_spans = Vec::new();
                            current_line_num += 1;

                            if self.line_numbers {
                                current_line_spans.push(Span::styled(
                                    format!("{:4} │ ", current_line_num + 1),
                                    Style::default().fg(Color::DarkGray),
                                ));
                            }
                        }

                        if !part.is_empty() {
                            current_line_spans.push(Span::styled(part.to_string(), current_style));
                        }
                    }
                }
                HighlightEvent::HighlightStart(highlight) => {
                    current_style = Self::capture_to_style(highlight.0);
                }
                HighlightEvent::HighlightEnd => {
                    current_style = Style::default();
                }
            }
        }

        // Push the last line if it has content
        if !current_line_spans.is_empty() {
            highlighted_lines.push(Line::from(current_line_spans));
        }

        // Ensure we have at least empty lines if content has trailing newlines
        while highlighted_lines.len() < lines.len() {
            let line_num = highlighted_lines.len();
            let mut spans = Vec::new();
            if self.line_numbers {
                spans.push(Span::styled(
                    format!("{:4} │ ", line_num + 1),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            highlighted_lines.push(Line::from(spans));
        }

        highlighted_lines
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
                (f32::from(self.scroll_offset) / (highlighted_lines.len() - 1) as f32 * 100.0)
                    as u16
            } else {
                0
            };

            let scroll_indicator = format!(" {scroll_percentage}% ");
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
