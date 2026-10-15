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

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;
    use ratatui::layout::Rect;

    fn buffer_with(area: Rect, symbol: &str) -> Buffer {
        let mut buf = Buffer::empty(area);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_symbol(symbol);
            }
        }
        buf
    }

    #[test]
    #[spec("TERM-006")]
    fn first_render_marks_all_cells_changed() {
        let area = Rect::new(0, 0, 4, 3);
        let mut renderer = DiffRenderer::new();
        let buf = buffer_with(area, "a");

        let changed = renderer.render_diff(&buf, area);
        assert_eq!(changed.len(), 12);

        // Metrics are recorded for every render
        let metrics = renderer.get_metrics();
        assert_eq!(metrics.frames_rendered, 1);
        assert!(metrics.is_within_budget(1000));
    }

    #[test]
    #[spec("TERM-006")]
    fn subsequent_render_highlights_only_changed_cells() {
        let area = Rect::new(0, 0, 4, 3);
        let mut renderer = DiffRenderer::new();
        let buf = buffer_with(area, "a");

        renderer.render_diff(&buf, area);

        // Identical frame: nothing changed
        let changed = renderer.render_diff(&buf, area);
        assert_eq!(changed, Vec::<(u16, u16)>::new());

        // Mutate one cell: exactly that cell is reported
        let mut modified = buf.clone();
        modified[(2, 1)].set_symbol("b");
        let changed = renderer.render_diff(&modified, area);
        assert_eq!(changed, vec![(2, 1)]);
        assert_eq!(renderer.get_metrics().frames_rendered, 3);
    }
}
