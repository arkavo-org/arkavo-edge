use anyhow::Result;
use metrics::histogram;
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem},
};

use crate::event::AppEvent;
use crate::renderer::Renderable;
use std::time::{Duration, Instant};
use tracing::{trace, warn};

#[derive(Debug, Clone)]
pub struct DiffHunk {
    pub old_start: usize,
    pub old_lines: usize,
    pub new_start: usize,
    pub new_lines: usize,
    pub header: String,
    pub lines: Vec<DiffLine>,
}

#[derive(Debug, Clone)]
pub struct DiffLine {
    pub line_type: DiffLineType,
    pub old_line_num: Option<usize>,
    pub new_line_num: Option<usize>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffLineType {
    Context,
    Addition,
    Deletion,
    Header,
}

impl DiffLineType {
    pub fn style(&self) -> Style {
        match self {
            DiffLineType::Context => Style::default().fg(Color::White),
            DiffLineType::Addition => Style::default().fg(Color::Green),
            DiffLineType::Deletion => Style::default().fg(Color::Red),
            DiffLineType::Header => Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        }
    }

    pub fn prefix(&self) -> &str {
        match self {
            DiffLineType::Context => " ",
            DiffLineType::Addition => "+",
            DiffLineType::Deletion => "-",
            DiffLineType::Header => "@",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DiffViewMode {
    Unified,
    SideBySide,
}

pub struct DiffView {
    pub(crate) file_path: String,
    pub(crate) hunks: Vec<DiffHunk>,
    pub(crate) scroll_offset: u16,
    pub(crate) view_mode: DiffViewMode,
    pub(crate) needs_redraw: bool,
    pub(crate) current_hunk: usize,
    last_render_duration: Option<Duration>,
    pending_diff_render: Option<Instant>,
    last_router_latency: Option<Duration>,
}

impl DiffView {
    pub fn new() -> Self {
        Self {
            file_path: String::new(),
            hunks: Vec::new(),
            scroll_offset: 0,
            view_mode: DiffViewMode::Unified,
            needs_redraw: true,
            current_hunk: 0,
            last_render_duration: None,
            pending_diff_render: None,
            last_router_latency: None,
        }
    }
}

impl Default for DiffView {
    fn default() -> Self {
        Self::new()
    }
}

impl DiffView {
    pub fn set_diff(&mut self, file_path: String, hunks: Vec<DiffHunk>) {
        self.file_path = file_path;
        self.hunks = hunks;
        self.scroll_offset = 0;
        self.current_hunk = 0;
        self.needs_redraw = true;
        self.pending_diff_render = Some(Instant::now());
    }

    pub fn parse_unified_diff(&mut self, diff_text: &str) {
        let mut hunks = Vec::new();
        let mut current_hunk: Option<DiffHunk> = None;
        let mut old_line = 0;
        let mut new_line = 0;

        for line in diff_text.lines() {
            if line.starts_with("+++") || line.starts_with("---") {
                // File headers - extract filename
                if line.starts_with("+++") {
                    self.file_path = line[4..].split('\t').next().unwrap_or("").to_string();
                }
            } else if line.starts_with("@@") {
                // Hunk header
                if let Some(hunk) = current_hunk.take() {
                    hunks.push(hunk);
                }

                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 3 {
                    let old_range = parts[1].trim_start_matches('-');
                    let new_range = parts[2].trim_start_matches('+');

                    let (old_start, old_count) = parse_range(old_range);
                    let (new_start, new_count) = parse_range(new_range);

                    old_line = old_start;
                    new_line = new_start;

                    current_hunk = Some(DiffHunk {
                        old_start,
                        old_lines: old_count,
                        new_start,
                        new_lines: new_count,
                        header: line.to_string(),
                        lines: vec![DiffLine {
                            line_type: DiffLineType::Header,
                            old_line_num: None,
                            new_line_num: None,
                            content: line.to_string(),
                        }],
                    });
                }
            } else if let Some(ref mut hunk) = current_hunk {
                // Diff content
                let (line_type, content) = if let Some(stripped) = line.strip_prefix('+') {
                    (DiffLineType::Addition, stripped)
                } else if let Some(stripped) = line.strip_prefix('-') {
                    (DiffLineType::Deletion, stripped)
                } else if let Some(stripped) = line.strip_prefix(' ') {
                    (DiffLineType::Context, stripped)
                } else {
                    (DiffLineType::Context, line)
                };

                let diff_line = match line_type {
                    DiffLineType::Addition => {
                        let l = DiffLine {
                            line_type,
                            old_line_num: None,
                            new_line_num: Some(new_line),
                            content: content.to_string(),
                        };
                        new_line += 1;
                        l
                    }
                    DiffLineType::Deletion => {
                        let l = DiffLine {
                            line_type,
                            old_line_num: Some(old_line),
                            new_line_num: None,
                            content: content.to_string(),
                        };
                        old_line += 1;
                        l
                    }
                    DiffLineType::Context => {
                        let l = DiffLine {
                            line_type,
                            old_line_num: Some(old_line),
                            new_line_num: Some(new_line),
                            content: content.to_string(),
                        };
                        old_line += 1;
                        new_line += 1;
                        l
                    }
                    _ => continue,
                };

                hunk.lines.push(diff_line);
            }
        }

        if let Some(hunk) = current_hunk {
            hunks.push(hunk);
        }

        self.hunks = hunks;
        self.needs_redraw = true;
        self.pending_diff_render = Some(Instant::now());
    }

    pub fn last_render_duration(&self) -> Option<Duration> {
        self.last_render_duration
    }

    pub fn last_router_latency(&self) -> Option<Duration> {
        self.last_router_latency
    }

    fn render_unified(&self, frame: &mut ratatui::Frame, area: Rect) {
        let block = Block::default()
            .title(format!("Diff: {} [Unified]", self.file_path))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::White));

        let inner_area = block.inner(area);
        frame.render_widget(block, area);

        let mut items = Vec::new();

        for hunk in &self.hunks {
            for line in &hunk.lines {
                let mut spans = vec![];

                // Line numbers
                if let Some(old_num) = line.old_line_num {
                    spans.push(Span::styled(
                        format!("{old_num:4}"),
                        Style::default().fg(Color::DarkGray),
                    ));
                } else {
                    spans.push(Span::raw("    "));
                }

                spans.push(Span::raw(" "));

                if let Some(new_num) = line.new_line_num {
                    spans.push(Span::styled(
                        format!("{new_num:4}"),
                        Style::default().fg(Color::DarkGray),
                    ));
                } else {
                    spans.push(Span::raw("    "));
                }

                spans.push(Span::raw(" │ "));

                // Diff marker and content
                spans.push(Span::styled(
                    line.line_type.prefix(),
                    line.line_type.style(),
                ));
                spans.push(Span::styled(line.content.clone(), line.line_type.style()));

                items.push(ListItem::new(Line::from(spans)));
            }
        }

        let list = List::new(items);
        frame.render_widget(list, inner_area);
    }

    fn render_side_by_side(&self, frame: &mut ratatui::Frame, area: Rect) {
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);

        // Left side (old)
        let old_block = Block::default()
            .title(format!("Old: {}", self.file_path))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Red));
        let old_inner = old_block.inner(chunks[0]);
        frame.render_widget(old_block, chunks[0]);

        // Right side (new)
        let new_block = Block::default()
            .title(format!("New: {}", self.file_path))
            .borders(Borders::ALL)
            .style(Style::default().fg(Color::Green));
        let new_inner = new_block.inner(chunks[1]);
        frame.render_widget(new_block, chunks[1]);

        // Collect old and new lines separately
        let mut old_lines = Vec::new();
        let mut new_lines = Vec::new();

        for hunk in &self.hunks {
            for line in &hunk.lines {
                match line.line_type {
                    DiffLineType::Context => {
                        // Context lines appear on both sides
                        old_lines.push(self.format_line_for_side(line, true));
                        new_lines.push(self.format_line_for_side(line, false));
                    }
                    DiffLineType::Deletion => {
                        // Deletions only on old side
                        old_lines.push(self.format_line_for_side(line, true));
                        new_lines.push(Line::from(vec![Span::raw("")])); // Empty line on new side
                    }
                    DiffLineType::Addition => {
                        // Additions only on new side
                        old_lines.push(Line::from(vec![Span::raw("")])); // Empty line on old side
                        new_lines.push(self.format_line_for_side(line, false));
                    }
                    DiffLineType::Header => {
                        // Header appears on both sides
                        let header_line =
                            Line::from(vec![Span::styled(&line.content, line.line_type.style())]);
                        old_lines.push(header_line.clone());
                        new_lines.push(header_line);
                    }
                }
            }
        }

        // Apply scroll offset
        let visible_height = old_inner.height as usize;
        let start = self.scroll_offset as usize;
        let end = (start + visible_height).min(old_lines.len());

        if start < old_lines.len() {
            let old_visible: Vec<ListItem> = old_lines[start..end]
                .iter()
                .map(|line| ListItem::new(line.clone()))
                .collect();
            let new_visible: Vec<ListItem> = new_lines[start..end]
                .iter()
                .map(|line| ListItem::new(line.clone()))
                .collect();

            let old_list = List::new(old_visible);
            let new_list = List::new(new_visible);

            frame.render_widget(old_list, old_inner);
            frame.render_widget(new_list, new_inner);
        }
    }

    fn format_line_for_side<'a>(&self, line: &'a DiffLine, is_old_side: bool) -> Line<'a> {
        let mut spans = vec![];

        // Line number
        if is_old_side {
            if let Some(num) = line.old_line_num {
                spans.push(Span::styled(
                    format!("{num:4}"),
                    Style::default().fg(Color::DarkGray),
                ));
            } else {
                spans.push(Span::raw("    "));
            }
        } else if let Some(num) = line.new_line_num {
            spans.push(Span::styled(
                format!("{num:4}"),
                Style::default().fg(Color::DarkGray),
            ));
        } else {
            spans.push(Span::raw("    "));
        }

        spans.push(Span::raw(" │ "));

        // Content with appropriate styling
        spans.push(Span::styled(line.content.clone(), line.line_type.style()));

        Line::from(spans)
    }

    pub fn handle_event(&mut self, event: AppEvent) -> Result<()> {
        if let AppEvent::KeyPress(key) = event {
            use crossterm::event::KeyCode;
            match key.code {
                KeyCode::Char('m') => {
                    self.view_mode = match self.view_mode {
                        DiffViewMode::Unified => DiffViewMode::SideBySide,
                        DiffViewMode::SideBySide => DiffViewMode::Unified,
                    };
                    self.needs_redraw = true;
                }
                KeyCode::Up => {
                    if self.scroll_offset > 0 {
                        self.scroll_offset -= 1;
                        self.needs_redraw = true;
                    }
                }
                KeyCode::Down => {
                    self.scroll_offset += 1;
                    self.needs_redraw = true;
                }
                KeyCode::Char('n') => {
                    // Next hunk
                    if self.current_hunk < self.hunks.len() - 1 {
                        self.current_hunk += 1;
                        self.needs_redraw = true;
                    }
                }
                KeyCode::Char('p') => {
                    // Previous hunk
                    if self.current_hunk > 0 {
                        self.current_hunk -= 1;
                        self.needs_redraw = true;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}

impl Renderable for DiffView {
    fn render(&mut self, frame: &mut ratatui::Frame, area: Rect) {
        let render_start = Instant::now();

        match self.view_mode {
            DiffViewMode::Unified => self.render_unified(frame, area),
            DiffViewMode::SideBySide => self.render_side_by_side(frame, area),
        }

        self.needs_redraw = false;

        let render_duration = render_start.elapsed();
        self.last_render_duration = Some(render_duration);
        const RENDER_BUDGET_MS: u64 = 50;
        const ROUTER_LATENCY_BUDGET_MS: u64 = 50;

        histogram!("arkavo_diff_render_ms").record(render_duration.as_secs_f64() * 1000.0);
        trace!(
            target = "arkavo.performance",
            event = "diff_render",
            ?render_duration,
            file = %self.file_path,
            mode = ?self.view_mode
        );

        if render_duration > Duration::from_millis(RENDER_BUDGET_MS) {
            warn!(
                target = "arkavo.performance",
                event = "diff_render_over_budget",
                ?render_duration,
                file = %self.file_path,
                budget_ms = RENDER_BUDGET_MS
            );
        }

        if let Some(start) = self.pending_diff_render.take() {
            let latency = render_start.duration_since(start);
            self.last_router_latency = Some(latency);
            histogram!("arkavo_router_to_diff_ms").record(latency.as_secs_f64() * 1000.0);
            trace!(
                target = "arkavo.performance",
                event = "router_to_diff",
                ?latency,
                file = %self.file_path
            );

            if latency > Duration::from_millis(ROUTER_LATENCY_BUDGET_MS) {
                warn!(
                    target = "arkavo.performance",
                    event = "router_to_diff_over_budget",
                    ?latency,
                    file = %self.file_path,
                    budget_ms = ROUTER_LATENCY_BUDGET_MS
                );
            }
        }
    }

    fn needs_redraw(&self) -> bool {
        self.needs_redraw
    }
}

pub fn parse_range(range: &str) -> (usize, usize) {
    let parts: Vec<&str> = range.split(',').collect();
    let start = parts[0].parse::<usize>().unwrap_or(1);
    let count = if parts.len() > 1 {
        parts[1].parse::<usize>().unwrap_or(1)
    } else {
        1
    };
    (start, count)
}
