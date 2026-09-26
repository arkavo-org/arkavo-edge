//! The embedding contract shared by index build and runtime.
//!
//! Build and runtime both call `embed_units`, so a paraphrase is compared
//! against vectors produced by exactly the same chunking, embedder and
//! normalisation — a unit cut differently on one side would be scored against
//! vectors of text the other side never saw. The chunker counts words and
//! characters rather than model tokens because it must live here, where no
//! tokenizer is linked, and because those counts are the same on every
//! machine.
//!
//! The embedding context holds 2048 tokens and refuses a longer text rather
//! than truncating it, so a single oversized unit fails the whole embed call
//! and the gate holds that completion for good. Words alone do not bound a
//! unit: CJK text and base64 carry no spaces, so a paragraph of either arrives
//! as one "word", and a unit of such words is both too long to embed and too
//! diluted to match. Two limits close that. A whitespace-free run longer than
//! `LONG_RUN_CHARS` is cut into `PIECE_CHARS`-character pieces, each a word;
//! and a unit closes before its words exceed `UNIT_CHARS` characters. At about
//! one token per character — the density of CJK and base64 — that keeps every
//! unit under the context. Ordinary prose meets neither limit (96 words of at
//! most 16 characters fit exactly), so its units are cut as before. Text
//! denser than one token per character can still overflow; embedding then
//! fails and the tier reports a gap, which holds rather than releases.

use serde::{Deserialize, Serialize};

pub const UNIT_WORDS: usize = 96;
pub const UNIT_OVERLAP: usize = 24;
/// Longest whitespace-free run kept whole as one word.
pub const LONG_RUN_CHARS: usize = 64;
/// Length of the pieces a longer run is cut into.
pub const PIECE_CHARS: usize = 16;
/// Most characters (spaces between words excluded) one unit carries.
pub const UNIT_CHARS: usize = UNIT_WORDS * PIECE_CHARS;
/// Most characters a carried overlap takes: the same quarter of a unit that
/// `UNIT_OVERLAP` is of `UNIT_WORDS`, so a tail of long words cannot use up
/// the next unit's whole budget.
const OVERLAP_CHARS: usize = UNIT_OVERLAP * PIECE_CHARS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmbeddingPooling {
    Mean,
    Cls,
    Last,
}

pub trait Embedder: Send + Sync {
    /// SHA-256 hex of the model file this embedder runs.
    fn digest(&self) -> &str;
    fn pooling(&self) -> EmbeddingPooling;
    /// One raw vector per input, in order. Errors are reported, never
    /// replaced by an empty result.
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String>;
}

/// Split text into the units the index stores and a query is scored by.
pub fn semantic_units(text: &str) -> Vec<String> {
    let mut units = Vec::new();
    let mut window: Vec<&str> = Vec::new();
    for sentence in sentences(text) {
        let words = words_of(sentence);
        if words.is_empty() {
            continue;
        }
        let (count, chars) = (words.len(), chars_of(&words));
        if count > UNIT_WORDS || chars > UNIT_CHARS {
            flush(&mut units, &mut window);
            split_long(&words, &mut units);
            continue;
        }
        if window.len() + count > UNIT_WORDS || chars_of(&window) + chars > UNIT_CHARS {
            // The carried overlap shrinks to whatever room the next sentence
            // leaves, so no unit ever exceeds either budget.
            let start = overlap_start(&window, UNIT_WORDS - count, UNIT_CHARS - chars);
            let carried = window[start..].to_vec();
            flush(&mut units, &mut window);
            window.extend(carried);
        }
        window.extend(words);
    }
    flush(&mut units, &mut window);
    units
}

fn sentences(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive(['.', '!', '?', '\n', '。', '！', '？', '；'])
}

/// A sentence's words, with every run longer than `LONG_RUN_CHARS` cut into
/// `PIECE_CHARS`-character pieces on character boundaries.
fn words_of(sentence: &str) -> Vec<&str> {
    let mut words = Vec::new();
    for word in sentence.split_whitespace() {
        if word.chars().count() <= LONG_RUN_CHARS {
            words.push(word);
            continue;
        }
        let mut rest = word;
        while !rest.is_empty() {
            let cut = rest
                .char_indices()
                .nth(PIECE_CHARS)
                .map_or(rest.len(), |(i, _)| i);
            let (piece, tail) = rest.split_at(cut);
            words.push(piece);
            rest = tail;
        }
    }
    words
}

fn chars_of(words: &[&str]) -> usize {
    words.iter().map(|w| w.chars().count()).sum()
}

/// Where the overlap carried out of `window` begins: its longest tail of at
/// most `UNIT_OVERLAP` words and `OVERLAP_CHARS` characters that also fits
/// the room left.
fn overlap_start(window: &[&str], room_words: usize, room_chars: usize) -> usize {
    let max_words = UNIT_OVERLAP.min(room_words);
    let max_chars = OVERLAP_CHARS.min(room_chars);
    let mut start = window.len();
    let mut chars = 0;
    while start > 0 && window.len() - start < max_words {
        let next = chars + window[start - 1].chars().count();
        if next > max_chars {
            break;
        }
        chars = next;
        start -= 1;
    }
    start
}

fn flush(units: &mut Vec<String>, window: &mut Vec<&str>) {
    if !window.is_empty() {
        units.push(window.join(" "));
        window.clear();
    }
}

fn split_long(words: &[&str], units: &mut Vec<String>) {
    let mut start = 0;
    loop {
        let end = window_end(words, start);
        units.push(words[start..end].join(" "));
        if end == words.len() {
            break;
        }
        // Always advance by at least one word, whatever the overlap.
        let overlap = overlap_start(&words[start..end], UNIT_WORDS, UNIT_CHARS);
        start += overlap.max(1);
    }
}

/// End of the longest window from `start` within both unit budgets. Every
/// word is at most `LONG_RUN_CHARS`, well under `UNIT_CHARS`, so the window
/// always holds at least one.
fn window_end(words: &[&str], start: usize) -> usize {
    let mut end = start;
    let mut chars = 0;
    while end < words.len() && end - start < UNIT_WORDS {
        let next = chars + words[end].chars().count();
        if next > UNIT_CHARS {
            break;
        }
        chars = next;
        end += 1;
    }
    end
}

pub fn embed_units(embedder: &dyn Embedder, text: &str) -> Result<Vec<Vec<f32>>, String> {
    let units = semantic_units(text);
    if units.is_empty() {
        return Ok(Vec::new());
    }
    let refs: Vec<&str> = units.iter().map(String::as_str).collect();
    let mut vectors = embedder.embed(&refs)?;
    if vectors.len() != refs.len() {
        return Err(format!(
            "embedder returned {} vectors for {} units",
            vectors.len(),
            refs.len()
        ));
    }
    for v in &mut vectors {
        normalize_vector(v);
    }
    Ok(vectors)
}

pub fn normalize_vector(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// A unit vector stored as int8, each vector stretched to its own full range.
///
/// Only direction matters for cosine, so the stretch factor is not stored:
/// `cosine` divides by the stored vector's own norm. Calibration runs against
/// these stored values, so quantisation error is inside the threshold rather
/// than beside it.
#[derive(Debug, Clone, PartialEq)]
pub struct QuantizedVector {
    pub values: Vec<i8>,
}

impl QuantizedVector {
    pub fn quantize(unit: &[f32]) -> Self {
        let max = unit.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        let stretch = if max > 0.0 { 127.0 / max } else { 1.0 };
        let values = unit
            .iter()
            .map(|x| (x * stretch).round().clamp(-127.0, 127.0) as i8)
            .collect();
        Self { values }
    }

    /// Cosine against an L2-normalised f32 query.
    pub fn cosine(&self, query: &[f32]) -> f32 {
        let dot: f32 = self
            .values
            .iter()
            .zip(query)
            .map(|(v, q)| f32::from(*v) * q)
            .sum();
        let norm = self
            .values
            .iter()
            .map(|v| f32::from(*v).powi(2))
            .sum::<f32>()
            .sqrt();
        if norm == 0.0 { 0.0 } else { dot / norm }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Deterministic stand-in: hashes each word into a fixed bucket, so two
    /// texts sharing words share direction. Not a model — the tier's logic is
    /// what these tests exercise.
    ///
    /// `pub(crate)` (not `pub`) so the semantic index, calibration and tier
    /// tests can import `crate::embed::tests::BagOfWords`; `unreachable_pub` and
    /// `redundant_pub_crate` disagree on which visibility a `pub(crate)`-only
    /// module wants, so the latter is silenced here rather than exporting a
    /// test double from the crate's public API.
    #[allow(clippy::redundant_pub_crate)]
    pub(crate) struct BagOfWords;
    impl Embedder for BagOfWords {
        fn digest(&self) -> &'static str {
            "test-digest"
        }
        fn pooling(&self) -> EmbeddingPooling {
            EmbeddingPooling::Last
        }
        fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0.0f32; 64];
                    for w in t.split_whitespace() {
                        let h = blake3::hash(w.to_lowercase().as_bytes());
                        v[(h.as_bytes()[0] as usize) % 64] += 1.0;
                    }
                    v
                })
                .collect())
        }
    }

    #[test]
    fn short_text_is_one_unit() {
        assert_eq!(semantic_units("ok thanks"), vec!["ok thanks".to_string()]);
    }

    #[test]
    fn empty_text_has_no_units() {
        assert!(semantic_units("   \n ").is_empty());
    }

    #[test]
    fn sentences_pack_into_windows_with_overlap() {
        let sentence = format!("{}.", vec!["alpha"; 40].join(" "));
        let text = [sentence.as_str(); 5].join(" ");
        let units = semantic_units(&text);
        assert!(units.len() >= 2);
        for unit in &units {
            assert!(unit.split_whitespace().count() <= UNIT_WORDS);
        }
    }

    #[test]
    fn overlap_never_pushes_a_unit_past_the_window() {
        let first = format!("{}.", vec!["alpha"; 50].join(" "));
        let second = format!("{}.", vec!["beta"; 80].join(" "));
        let units = semantic_units(&format!("{first} {second}"));
        for unit in &units {
            assert!(
                unit.split_whitespace().count() <= UNIT_WORDS,
                "{} words",
                unit.split_whitespace().count()
            );
        }
        assert!(units.iter().any(|u| u.contains("beta")));
    }

    #[test]
    fn a_sentence_longer_than_a_window_is_split_at_word_boundaries() {
        let words: Vec<String> = (0..250).map(|i| format!("w{i}")).collect();
        let units = semantic_units(&words.join(" "));
        assert_eq!(units[0].split_whitespace().count(), UNIT_WORDS);
        let first_last = units[0].split_whitespace().last().unwrap().to_string();
        assert!(
            units[1]
                .split_whitespace()
                .any(|w| w == first_last.as_str()),
            "windows overlap"
        );
        let covered: std::collections::HashSet<&str> =
            units.iter().flat_map(|u| u.split_whitespace()).collect();
        assert_eq!(covered.len(), 250, "no word is dropped");
    }

    fn word_chars(unit: &str) -> usize {
        unit.split_whitespace().map(|w| w.chars().count()).sum()
    }

    #[test]
    fn a_long_unbroken_run_is_bounded_per_unit() {
        let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let run: String = alphabet.chars().cycle().take(5000).collect();
        let units = semantic_units(&run);
        let counts: Vec<usize> = units.iter().map(|u| u.split_whitespace().count()).collect();
        assert_eq!(counts, [96, 96, 96, 96, 25], "313 pieces, 72-piece step");
        for unit in &units {
            assert!(unit.split_whitespace().count() <= UNIT_WORDS);
            assert!(word_chars(unit) <= UNIT_WORDS * 16, "{}", word_chars(unit));
            assert!(unit.split_whitespace().all(|w| w.chars().count() <= 16));
        }
        let first: String = units[0].split_whitespace().collect();
        assert!(run.starts_with(&first), "pieces keep the run's order");
    }

    #[test]
    fn full_width_terminators_end_sentences() {
        assert_eq!(
            semantic_units("甲乙。丙丁！戊己？庚辛；壬癸"),
            vec!["甲乙。 丙丁！ 戊己？ 庚辛； 壬癸".to_string()]
        );
    }

    #[test]
    fn a_cjk_run_without_punctuation_is_split_on_char_boundaries() {
        let run: String = "数据保密协议".chars().cycle().take(100).collect();
        let units = semantic_units(&run);
        assert_eq!(units.len(), 1);
        let pieces: Vec<&str> = units[0].split_whitespace().collect();
        assert_eq!(pieces.len(), 7, "100 chars in 16-char pieces");
        assert!(pieces.iter().all(|p| p.chars().count() <= 16));
        assert_eq!(pieces.concat(), run);
    }

    #[test]
    fn words_up_to_sixty_four_chars_are_kept_whole() {
        let word = "a".repeat(64);
        let units = semantic_units(&format!("see {word} here"));
        assert_eq!(units, vec![format!("see {word} here")]);
    }

    #[test]
    fn long_words_close_a_unit_before_it_outgrows_the_context() {
        let word = "b".repeat(60);
        let one_sentence = vec![word.as_str(); 120].join(" ");
        let sentences = vec![format!("{word}."); 120].join(" ");
        for text in [one_sentence, sentences] {
            let units = semantic_units(&text);
            let counts: Vec<usize> = units.iter().map(|u| u.split_whitespace().count()).collect();
            assert_eq!(
                counts, [25; 6],
                "25 × 60 chars fit, a 6-word overlap carries"
            );
            for unit in &units {
                assert!(word_chars(unit) <= UNIT_WORDS * 16, "{}", word_chars(unit));
            }
        }
    }

    #[test]
    fn chunking_is_deterministic() {
        let text = "One sentence here. Another one follows! And a third?";
        assert_eq!(semantic_units(text), semantic_units(text));
    }

    #[test]
    fn embed_units_normalises_every_vector() {
        let vectors = embed_units(&BagOfWords, "alpha beta gamma. delta").unwrap();
        for v in vectors {
            let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-5);
        }
    }

    #[test]
    fn quantised_cosine_tracks_float_cosine() {
        let mut a: Vec<f32> = (0..64).map(|i| ((i * 7) % 13) as f32 - 6.0).collect();
        let mut b: Vec<f32> = (0..64).map(|i| ((i * 5) % 11) as f32 - 5.0).collect();
        normalize_vector(&mut a);
        normalize_vector(&mut b);
        let exact: f32 = a.iter().zip(&b).map(|(x, y)| x * y).sum();
        let quantised = QuantizedVector::quantize(&a).cosine(&b);
        assert!((exact - quantised).abs() < 0.02, "{exact} vs {quantised}");
    }

    #[test]
    fn an_embedder_error_is_reported_not_swallowed() {
        struct Broken;
        impl Embedder for Broken {
            fn digest(&self) -> &'static str {
                "x"
            }
            fn pooling(&self) -> EmbeddingPooling {
                EmbeddingPooling::Last
            }
            fn embed(&self, _: &[&str]) -> Result<Vec<Vec<f32>>, String> {
                Err("decode failed".into())
            }
        }
        assert!(embed_units(&Broken, "some text").is_err());
    }

    #[test]
    fn a_wrong_vector_count_is_an_error() {
        struct Short;
        impl Embedder for Short {
            fn digest(&self) -> &'static str {
                "x"
            }
            fn pooling(&self) -> EmbeddingPooling {
                EmbeddingPooling::Last
            }
            fn embed(&self, _: &[&str]) -> Result<Vec<Vec<f32>>, String> {
                Ok(Vec::new())
            }
        }
        assert!(embed_units(&Short, "some text").is_err());
    }
}
