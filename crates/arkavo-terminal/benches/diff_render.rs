use arkavo_terminal::DiffRenderer;
use criterion::{Criterion, criterion_group, criterion_main};
use ratatui::{buffer::Buffer, layout::Rect};

fn build_buffer(area: Rect, marker: &str) -> Buffer {
    let mut buffer = Buffer::empty(area);
    let width = area.width;
    for y in 0..area.height {
        for x in 0..width {
            if (x + y) % 17 == 0 {
                buffer
                    .get_mut(area.left() + x, area.top() + y)
                    .set_symbol(marker)
                    .set_style(ratatui::style::Style::default().fg(ratatui::style::Color::Cyan));
            }
        }
    }
    buffer
}

fn diff_renderer_benchmark(c: &mut Criterion) {
    let area = Rect::new(0, 0, 120, 40);
    let baseline = build_buffer(area, "·");
    let mut renderer = DiffRenderer::new();

    // Prime renderer with initial frame so subsequent iterations simulate diff updates.
    renderer.render_diff(&baseline, area);

    c.bench_function("diff_renderer_120x40_sparse", |b| {
        b.iter(|| {
            let mut frame = baseline.clone();

            // Apply modifications comparable to diff hunks.
            for (idx, y) in (0..area.height).enumerate().step_by(4) {
                let offset_x = (idx as u16 * 7) % area.width;
                frame
                    .get_mut(area.left() + offset_x, area.top() + y)
                    .set_symbol("+")
                    .set_style(ratatui::style::Style::default().fg(ratatui::style::Color::Green));
            }

            renderer.render_diff(&frame, area);
        });
    });
}

criterion_group!(name = diff; config = Criterion::default().configure_from_args(); targets = diff_renderer_benchmark);
criterion_main!(diff);
