//! Normalization and shingling (KP-009).
//!
//! Content is normalized before hashing so that reformatting is not an escape.
//! An exact-hash tier that keys on raw bytes is defeated by changing a newline;
//! the normalization here is what makes "the same document, re-wrapped" the
//! same document.
//!
//! Normalization is deliberately lossy in one direction only. It removes
//! distinctions an attacker could introduce for free (case, whitespace run
//! length, surrounding punctuation) and preserves everything else. Removing
//! more would start merging genuinely different content, which costs precision
//! and shows up as false positives.

/// Words per shingle.
///
/// Five is short enough that a paragraph yields many overlapping windows — so
/// quoting part of a document still matches — and long enough that ordinary
/// English five-grams are rare, which is what keeps the false-positive rate
/// down without a suppression list doing all the work.
pub const SHINGLE_WORDS: usize = 5;

/// Normalize text for hashing.
///
/// Lowercases, collapses every whitespace run to one space, and trims
/// punctuation from the edges of each token. Interior punctuation stays:
/// `sk-abc` and `sk abc` are not the same token and must not collide.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for token in text.split_whitespace() {
        let trimmed = token.trim_matches(|c: char| c.is_ascii_punctuation());
        if trimmed.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.extend(trimmed.chars().flat_map(char::to_lowercase));
    }
    out
}

/// Overlapping word windows over already-normalized text.
///
/// Overlapping, not disjoint: disjoint windows shift out of alignment when a
/// single word is inserted, so an attacker prepends one word and the whole
/// document stops matching.
pub fn shingles(normalized: &str) -> Vec<String> {
    let words: Vec<&str> = normalized.split(' ').filter(|w| !w.is_empty()).collect();
    if words.is_empty() {
        return Vec::new();
    }
    if words.len() <= SHINGLE_WORDS {
        // Short spans still have to be indexable, or a one-line secret is
        // invisible to the tier that exists to catch it.
        return vec![words.join(" ")];
    }
    words
        .windows(SHINGLE_WORDS)
        .map(|window| window.join(" "))
        .collect()
}

/// Overlapping windows over pre-split words, produced one at a time.
///
/// The allocating [`shingles`] is fine at build time, where the whole corpus is
/// being read anyway. On the per-call path a budget has to be able to stop the
/// work part-way, which it cannot do if every window was built up front.
pub fn windows<'a>(words: &'a [&'a str]) -> impl Iterator<Item = String> + 'a {
    let short = words.len() <= SHINGLE_WORDS;
    let count = if words.is_empty() {
        0
    } else if short {
        1
    } else {
        words.len() - SHINGLE_WORDS + 1
    };
    (0..count).map(move |i| {
        if short {
            words.join(" ")
        } else {
            words[i..i + SHINGLE_WORDS].join(" ")
        }
    })
}

/// Normalize and shingle in one step.
pub fn shingle_text(text: &str) -> Vec<String> {
    shingles(&normalize(text))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reformatting_does_not_change_the_shingles() {
        // The whole point: whitespace and case are free for an attacker to change.
        let a = shingle_text("The Quick Brown Fox Jumps Over");
        let b = shingle_text("the   quick\nbrown\tfox jumps over");

        assert_eq!(a, b);
    }

    #[test]
    fn edge_punctuation_is_stripped_but_interior_is_kept() {
        assert_eq!(normalize("(hello), world!"), "hello world");
        // Interior structure distinguishes a token from a different token.
        assert_eq!(normalize("api-key=value"), "api-key=value");
    }

    #[test]
    fn windows_overlap_so_an_inserted_word_does_not_shift_everything_out() {
        let original = shingle_text("alpha bravo charlie delta echo foxtrot golf");
        let prefixed = shingle_text("zulu alpha bravo charlie delta echo foxtrot golf");

        let shared = original.iter().filter(|s| prefixed.contains(s)).count();

        assert!(
            shared >= original.len() - 1,
            "inserting one word lost {} of {} shingles",
            original.len() - shared,
            original.len()
        );
    }

    #[test]
    fn a_span_shorter_than_a_window_still_yields_one_shingle() {
        assert_eq!(shingle_text("two words"), vec!["two words".to_string()]);
    }

    #[test]
    fn empty_and_punctuation_only_text_yields_nothing() {
        assert!(shingle_text("").is_empty());
        assert!(shingle_text("   ").is_empty());
        assert!(shingle_text("--- ...").is_empty());
    }

    #[test]
    fn lazy_windows_match_the_allocating_form() {
        let normalized = normalize("alpha bravo charlie delta echo foxtrot golf");
        let words: Vec<&str> = normalized.split(' ').collect();

        let lazy: Vec<String> = windows(&words).collect();

        assert_eq!(lazy, shingles(&normalized));
    }

    #[test]
    fn shingle_count_tracks_word_count() {
        let text = (0..20)
            .map(|i| format!("w{i}"))
            .collect::<Vec<_>>()
            .join(" ");

        assert_eq!(shingle_text(&text).len(), 20 - SHINGLE_WORDS + 1);
    }
}
