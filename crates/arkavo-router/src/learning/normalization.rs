//! Lesson text normalization for deduplication.
//!
//! Strips variable parts (entity IDs, quoted names, UUIDs, numbers)
//! from lesson condition/action text so structurally identical lessons
//! collapse to the same failure mode key.

use regex::Regex;
use std::sync::LazyLock;

static NORMALIZER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}|0x[0-9a-fA-F]+|'[^']{1,60}'|\x22[^\x22]{1,60}\x22|\b[A-Z][a-z]+\d+\b|\b[a-z]+_\d+\b|\b\d{3,}\b",
    )
    .expect("normalization regex")
});

/// Normalize condition and action text into a dedup key.
///
/// Replaces variable parts (IDs, names, numbers) with `<ID>` so
/// structurally identical lessons produce the same key.
pub fn normalize_for_dedup(condition: &str, action: &str) -> String {
    let norm_cond = NORMALIZER.replace_all(condition, "<ID>");
    let norm_act = NORMALIZER.replace_all(action, "<ID>");
    format!("{}|{}", norm_cond.trim(), norm_act.trim())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_entity_ids() {
        let key = normalize_for_dedup(
            "calling game-rl:observe",
            "avoid: SetWorkPriority: ColonistId not found. Use name: ['Human594']",
        );
        assert!(
            !key.contains("Human594"),
            "Should strip Human594, got: {key}"
        );
        assert!(key.contains("<ID>"));
    }

    #[test]
    fn strips_quoted_names() {
        let key = normalize_for_dedup(
            "calling game-rl:step",
            "avoid: ColonistId 'José Naime' not found",
        );
        assert!(
            !key.contains("José"),
            "Should strip quoted name, got: {key}"
        );
    }

    #[test]
    fn strips_uuids() {
        let key = normalize_for_dedup("task 550e8400-e29b-41d4-a716-446655440000 failed", "retry");
        assert!(!key.contains("550e8400"), "Should strip UUID, got: {key}");
    }

    #[test]
    fn strips_hex() {
        let key = normalize_for_dedup("hash 0xdeadbeef mismatch", "recalculate");
        assert!(!key.contains("deadbeef"), "Should strip hex, got: {key}");
    }

    #[test]
    fn strips_large_numbers() {
        let key = normalize_for_dedup("tick 415867 action failed", "retry");
        assert!(
            !key.contains("415867"),
            "Should strip large numbers, got: {key}"
        );
    }

    #[test]
    fn preserves_small_numbers() {
        let key = normalize_for_dedup("priority 1 too low", "set to 3");
        assert!(key.contains("1"), "Should keep small numbers");
        assert!(key.contains("3"), "Should keep small numbers");
    }

    #[test]
    fn identical_failures_same_key() {
        let key1 = normalize_for_dedup(
            "calling game-rl:observe",
            "avoid: SetWorkPriority: ColonistId not found. Use name: ['Human594']",
        );
        let key2 = normalize_for_dedup(
            "calling game-rl:observe",
            "avoid: SetWorkPriority: ColonistId not found. Use name: ['Human22609']",
        );
        assert_eq!(key1, key2, "Same failure with different IDs should match");
    }

    #[test]
    fn different_failures_different_keys() {
        let key1 = normalize_for_dedup("calling game-rl:step", "avoid: Draft failed");
        let key2 = normalize_for_dedup("calling game-rl:step", "avoid: Hunt failed");
        assert_ne!(key1, key2);
    }

    #[test]
    fn empty_strings() {
        let key = normalize_for_dedup("", "");
        assert_eq!(key, "|");
    }

    #[test]
    fn no_variable_parts_unchanged() {
        let key = normalize_for_dedup(
            "calling game-rl:step without prior observe",
            "call observe first",
        );
        assert_eq!(
            key,
            "calling game-rl:step without prior observe|call observe first"
        );
    }
}
