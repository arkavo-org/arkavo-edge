//! Small, shared text helpers used by multiple orchestrator modules.
//!
//! Kept intentionally tiny — if this file grows, split it into a dedicated
//! crate rather than a kitchen-sink module.

/// Truncate a string to at most `max` bytes, respecting UTF-8 character
/// boundaries. If truncated, appends a single ellipsis character.
///
/// Safe for arbitrary Unicode input, including 4-byte characters.
pub fn truncate_ellipsis(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\u{2026}", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorter_than_max_returns_original() {
        let out = truncate_ellipsis("hello", 50);
        assert_eq!(out, "hello");
    }

    #[test]
    fn longer_than_max_is_truncated_with_ellipsis() {
        let input = "a".repeat(100);
        let out = truncate_ellipsis(&input, 10);
        // 10 'a's + 1 ellipsis char (3 UTF-8 bytes, but display as 1 char)
        assert_eq!(out.chars().count(), 11);
        assert!(out.starts_with("aaaaaaaaaa"));
        assert!(out.ends_with('\u{2026}'));
    }

    #[test]
    fn respects_utf8_boundaries() {
        // Each 'é' is 2 bytes.
        let input = "éééééééééé"; // 20 bytes, 10 chars
        let out = truncate_ellipsis(input, 5);
        // Should truncate at a char boundary (4 bytes = 2 chars), not mid-char.
        assert!(out.ends_with('\u{2026}'));
        // No panic, no garbage — the text slice before the ellipsis must be
        // valid UTF-8 (already guaranteed since we only accepted char boundaries).
    }

    #[test]
    fn empty_input() {
        assert_eq!(truncate_ellipsis("", 10), "");
    }

    #[test]
    fn zero_max_produces_just_ellipsis() {
        let out = truncate_ellipsis("hello", 0);
        assert_eq!(out, "\u{2026}");
    }
}
