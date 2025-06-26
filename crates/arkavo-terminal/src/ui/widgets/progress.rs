use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::Span,
    widgets::{Block, Gauge, Widget},
};
use std::time::{Duration, Instant};

pub struct ProgressIndicator {
    pub label: String,
    pub current: f64,
    pub total: f64,
    pub start_time: Instant,
}

impl ProgressIndicator {
    pub fn new(label: String, total: f64) -> Self {
        Self {
            label,
            current: 0.0,
            total,
            start_time: Instant::now(),
        }
    }

    pub fn set_progress(&mut self, current: f64) {
        self.current = current.min(self.total);
    }

    pub fn increment(&mut self, amount: f64) {
        self.set_progress(self.current + amount);
    }

    pub fn percentage(&self) -> f64 {
        if self.total > 0.0 {
            (self.current / self.total * 100.0).min(100.0)
        } else {
            0.0
        }
    }

    pub fn elapsed(&self) -> Duration {
        self.start_time.elapsed()
    }

    pub fn eta(&self) -> Option<Duration> {
        if self.current > 0.0 && self.current < self.total {
            let elapsed = self.elapsed().as_secs_f64();
            let rate = self.current / elapsed;
            let remaining = self.total - self.current;
            let eta_secs = remaining / rate;
            Some(Duration::from_secs_f64(eta_secs))
        } else {
            None
        }
    }

    pub fn render(&self, frame: &mut ratatui::Frame, area: Rect) {
        let percentage = self.percentage();
        let label = format!(
            "{} [{:.0}%] {}/{}",
            self.label, percentage, self.current as u64, self.total as u64
        );

        let gauge = Gauge::default()
            .block(Block::default())
            .gauge_style(Style::default().fg(Color::Green))
            .percent(percentage as u16)
            .label(label);

        frame.render_widget(gauge, area);
    }
}

pub struct SpinnerWidget {
    frames: Vec<&'static str>,
    current_frame: usize,
    last_update: Instant,
    frame_duration: Duration,
}

impl Default for SpinnerWidget {
    fn default() -> Self {
        Self::new()
    }
}

impl SpinnerWidget {
    pub fn new() -> Self {
        Self {
            frames: vec!["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            current_frame: 0,
            last_update: Instant::now(),
            frame_duration: Duration::from_millis(80),
        }
    }

    pub fn tick(&mut self) {
        if self.last_update.elapsed() >= self.frame_duration {
            self.current_frame = (self.current_frame + 1) % self.frames.len();
            self.last_update = Instant::now();
        }
    }

    pub fn current_frame(&self) -> &str {
        self.frames[self.current_frame]
    }
}

impl Widget for SpinnerWidget {
    fn render(mut self, area: Rect, buf: &mut ratatui::buffer::Buffer) {
        self.tick();
        let spinner = Span::styled(self.current_frame(), Style::default().fg(Color::Green));
        buf.set_span(area.x, area.y, &spinner, area.width);
    }
}
