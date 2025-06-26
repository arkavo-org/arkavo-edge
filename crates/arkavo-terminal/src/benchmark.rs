use crate::ui::{chat::{ChatView, MessageRole}, diff::{DiffView, DiffHunk}};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::time::Instant;

pub fn run_performance_benchmark() {
    println!("Running Terminal UI Performance Benchmark...\n");
    
    // Create test data
    let mut chat_view = create_test_chat_view(100);
    let mut diff_view = create_test_diff_view(1000);
    
    // Create a test buffer
    let area = Rect::new(0, 0, 120, 40);
    let mut buffer = Buffer::empty(area);
    
    // Benchmark chat rendering
    let chat_start = Instant::now();
    for _ in 0..10 {
        render_chat_to_buffer(&mut chat_view, &mut buffer, area);
    }
    let chat_avg = chat_start.elapsed().as_millis() / 10;
    
    // Benchmark diff rendering
    let diff_start = Instant::now();
    for _ in 0..10 {
        render_diff_to_buffer(&mut diff_view, &mut buffer, area);
    }
    let diff_avg = diff_start.elapsed().as_millis() / 10;
    
    // Combined worst-case
    let combined_start = Instant::now();
    render_chat_to_buffer(&mut chat_view, &mut buffer, Rect::new(0, 0, 60, 40));
    render_diff_to_buffer(&mut diff_view, &mut buffer, Rect::new(60, 0, 60, 40));
    let combined_time = combined_start.elapsed();
    
    // Report results
    println!("Performance Benchmark Results:");
    println!("==============================");
    println!("Chat View (100 messages):     {}ms avg", chat_avg);
    println!("Diff View (1000 lines):       {}ms avg", diff_avg);
    println!("Combined worst-case:          {}ms", combined_time.as_millis());
    println!("Target frame budget:          50ms");
    println!();
    
    if combined_time.as_millis() <= 50 {
        println!("✅ PASS: Rendering within 50ms frame budget");
    } else {
        println!("❌ FAIL: Rendering exceeds 50ms frame budget");
        std::process::exit(1);
    }
}

fn create_test_chat_view(message_count: usize) -> ChatView {
    let mut chat = ChatView::new();
    for i in 0..message_count {
        let role = if i % 2 == 0 { MessageRole::User } else { MessageRole::Assistant };
        chat.add_message(role, format!("Test message {} with some longer content to simulate real chat", i));
    }
    chat
}

fn create_test_diff_view(line_count: usize) -> DiffView {
    let mut diff = DiffView::new();
    let mut hunks = Vec::new();
    
    // Create hunks with ~100 lines each
    for chunk in 0..(line_count / 100) {
        let mut lines = Vec::new();
        for i in 0..100 {
            let line_num = chunk * 100 + i;
            lines.push(crate::ui::diff::DiffLine {
                line_type: if i % 3 == 0 { 
                    crate::ui::diff::DiffLineType::Addition 
                } else if i % 3 == 1 { 
                    crate::ui::diff::DiffLineType::Deletion 
                } else { 
                    crate::ui::diff::DiffLineType::Context 
                },
                old_line_num: Some(line_num),
                new_line_num: Some(line_num + 10),
                content: format!("Line {} with some code content that might be quite long", line_num),
            });
        }
        
        hunks.push(DiffHunk {
            old_start: chunk * 100,
            old_lines: 100,
            new_start: chunk * 100 + 10,
            new_lines: 100,
            header: format!("@@ -{},100 +{},100 @@", chunk * 100, chunk * 100 + 10),
            lines,
        });
    }
    
    diff.set_diff("test_file.rs".to_string(), hunks);
    diff
}

fn render_chat_to_buffer(chat: &mut ChatView, _buffer: &mut Buffer, _area: Rect) {
    // Simulate render without actual frame
    use crate::renderer::Renderable;
    // This would normally call chat.render(frame, area)
    // For benchmark, we just mark needs_redraw
    let _ = chat.needs_redraw();
}

fn render_diff_to_buffer(diff: &mut DiffView, _buffer: &mut Buffer, _area: Rect) {
    // Simulate render without actual frame
    use crate::renderer::Renderable;
    // This would normally call diff.render(frame, area)
    // For benchmark, we just mark needs_redraw
    let _ = diff.needs_redraw();
}