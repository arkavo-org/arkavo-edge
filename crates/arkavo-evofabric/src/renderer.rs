/// Render a `syn::File` AST back to deterministic Rust source code.
///
/// Uses `prettyplease` for consistent, reproducible formatting.
/// Same AST always produces the same output.
pub fn render_file(file: &syn::File) -> String {
    prettyplease::unparse(file)
}

/// Render and return a summary of changes compared to the original source.
pub fn render_diff_summary(original: &str, modified: &syn::File) -> DiffSummary {
    let rendered = render_file(modified);

    DiffSummary {
        original_lines: original.lines().count(),
        modified_lines: rendered.lines().count(),
        rendered_source: rendered,
    }
}

/// Summary of changes between original and modified source.
#[derive(Debug)]
pub struct DiffSummary {
    pub original_lines: usize,
    pub modified_lines: usize,
    pub rendered_source: String,
}

impl DiffSummary {
    pub fn lines_delta(&self) -> isize {
        #[allow(clippy::cast_possible_wrap)]
        let delta = self.modified_lines as isize - self.original_lines as isize;
        delta
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rust_op::RustOp;

    #[test]
    fn render_roundtrip() {
        let source = "fn foo() -> i32 { 42 }";
        let file = RustOp::parse(source).unwrap();
        let rendered = render_file(&file);
        assert!(rendered.contains("fn foo"));
        assert!(rendered.contains("42"));
    }

    #[test]
    fn render_is_deterministic() {
        let source = "fn a() {} fn b() {} struct C;";
        let file = RustOp::parse(source).unwrap();
        let r1 = render_file(&file);
        let r2 = render_file(&file);
        assert_eq!(r1, r2);
    }

    #[test]
    fn diff_summary() {
        let original = "fn foo() { 1 }";
        let file = RustOp::parse("fn foo() { 1 }\nfn bar() { 2 }").unwrap();
        let summary = render_diff_summary(original, &file);
        assert!(summary.lines_delta() > 0);
        assert!(summary.rendered_source.contains("bar"));
    }
}
