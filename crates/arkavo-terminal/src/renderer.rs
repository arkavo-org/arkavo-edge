use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::time::{Duration, Instant};

pub struct RenderMetrics {
    pub frame_time: Duration,
    pub last_update: Instant,
    pub frames_rendered: u64,
}

impl RenderMetrics {
    pub fn new() -> Self {
        Self {
            frame_time: Duration::from_millis(0),
            last_update: Instant::now(),
            frames_rendered: 0,
        }
    }
}

impl Default for RenderMetrics {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderMetrics {
    pub fn update(&mut self) {
        let now = Instant::now();
        self.frame_time = now.duration_since(self.last_update);
        self.last_update = now;
        self.frames_rendered += 1;
    }

    pub fn is_within_budget(&self, budget_ms: u64) -> bool {
        self.frame_time.as_millis() <= u128::from(budget_ms)
    }
}

pub struct DiffRenderer {
    previous_buffer: Option<Buffer>,
    metrics: RenderMetrics,
}

impl DiffRenderer {
    pub fn new() -> Self {
        Self {
            previous_buffer: None,
            metrics: RenderMetrics::new(),
        }
    }
}

impl Default for DiffRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl DiffRenderer {
    pub fn render_diff(&mut self, current: &Buffer, area: Rect) -> Vec<(u16, u16)> {
        let mut changed_cells = Vec::new();

        if let Some(ref prev) = self.previous_buffer {
            for y in area.top()..area.bottom() {
                for x in area.left()..area.right() {
                    let idx = current.index_of(x, y);
                    if prev.content[idx] != current.content[idx] {
                        changed_cells.push((x, y));
                    }
                }
            }
        } else {
            // First render, all cells are changed
            for y in area.top()..area.bottom() {
                for x in area.left()..area.right() {
                    changed_cells.push((x, y));
                }
            }
        }

        self.previous_buffer = Some(current.clone());
        self.metrics.update();

        changed_cells
    }

    pub fn get_metrics(&self) -> &RenderMetrics {
        &self.metrics
    }
}

pub trait Renderable {
    fn render(&mut self, frame: &mut ratatui::Frame, area: Rect);
    fn needs_redraw(&self) -> bool;
}
