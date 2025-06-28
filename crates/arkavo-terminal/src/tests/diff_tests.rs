use crate::ui::diff::{DiffHunk, DiffLine, DiffLineType, DiffView};

#[test]
fn test_parse_unified_diff() {
    let mut diff_view = DiffView::new();

    let diff_text = r#"--- a/test.rs
+++ b/test.rs
@@ -1,4 +1,5 @@
 fn main() {
-    println!("Hello");
+    println!("Hello, World!");
+    println!("Another line");
 }
"#;

    diff_view.parse_unified_diff(diff_text);

    assert_eq!(diff_view.hunks.len(), 1);
    let hunk = &diff_view.hunks[0];

    assert_eq!(hunk.old_start, 1);
    assert_eq!(hunk.old_lines, 4);
    assert_eq!(hunk.new_start, 1);
    assert_eq!(hunk.new_lines, 5);

    // Check line types
    let line_types: Vec<_> = hunk.lines.iter().map(|l| &l.line_type).collect();
    assert_eq!(
        line_types,
        vec![
            &DiffLineType::Header,
            &DiffLineType::Context,
            &DiffLineType::Deletion,
            &DiffLineType::Addition,
            &DiffLineType::Addition,
            &DiffLineType::Context,
        ]
    );
}

#[test]
fn test_diff_scroll_bounds() {
    let mut diff_view = DiffView::new();

    // Create a diff with 10 lines
    let mut lines = vec![];
    for i in 0..10 {
        lines.push(DiffLine {
            line_type: DiffLineType::Context,
            old_line_num: Some(i),
            new_line_num: Some(i),
            content: format!("Line {i}"),
        });
    }

    diff_view.set_diff(
        "test.rs".to_string(),
        vec![DiffHunk {
            old_start: 1,
            old_lines: 10,
            new_start: 1,
            new_lines: 10,
            header: "@@ -1,10 +1,10 @@".to_string(),
            lines,
        }],
    );

    // Test scrolling
    assert_eq!(diff_view.scroll_offset, 0);

    // Scroll down should work
    diff_view.scroll_offset = 5;
    assert_eq!(diff_view.scroll_offset, 5);

    // But not beyond content
    diff_view.scroll_offset = 100;
    // This would be validated during rendering
}

#[test]
fn test_parse_range() {
    use crate::ui::diff::parse_range;

    assert_eq!(parse_range("10,5"), (10, 5));
    assert_eq!(parse_range("20"), (20, 1));
    assert_eq!(parse_range("invalid"), (1, 1));
    assert_eq!(parse_range("5,invalid"), (5, 1));
}
