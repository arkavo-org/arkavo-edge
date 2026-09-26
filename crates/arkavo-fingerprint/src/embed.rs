//! The embedding contract shared by index build and runtime (spec: "one
//! embedding contract").
//!
//! Build and runtime both call `embed_units`, so a paraphrase is compared
//! against vectors produced by exactly the same chunking, embedder and
//! normalisation. The chunker counts words rather than model tokens because it
//! must live here, where no tokenizer is linked, and because a word count is
//! the same on every machine.

use serde::{Deserialize, Serialize};

pub const UNIT_WORDS: usize = 96;
pub const UNIT_OVERLAP: usize = 24;

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
        let words: Vec<&str> = sentence.split_whitespace().collect();
        if words.is_empty() {
            continue;
        }
        if words.len() > UNIT_WORDS {
            flush(&mut units, &mut window);
            split_long(&words, &mut units);
            continue;
        }
        if window.len() + words.len() > UNIT_WORDS {
            // The carried overlap shrinks to whatever room the next sentence
            // leaves, so no unit ever exceeds UNIT_WORDS.
            let room = UNIT_WORDS - words.len();
            let carried = carry_overlap(&window, room);
            flush(&mut units, &mut window);
            window.extend(carried);
        }
        window.extend(words);
    }
    flush(&mut units, &mut window);
    units
}

fn sentences(text: &str) -> impl Iterator<Item = &str> {
    text.split_inclusive(['.', '!', '?', '\n'])
}

fn carry_overlap<'a>(window: &[&'a str], room: usize) -> Vec<&'a str> {
    let start = window.len().saturating_sub(UNIT_OVERLAP.min(room));
    window[start..].to_vec()
}

fn flush(units: &mut Vec<String>, window: &mut Vec<&str>) {
    if !window.is_empty() {
        units.push(window.join(" "));
        window.clear();
    }
}

fn split_long(words: &[&str], units: &mut Vec<String>) {
    let step = UNIT_WORDS - UNIT_OVERLAP;
    let mut start = 0;
    loop {
        let end = (start + UNIT_WORDS).min(words.len());
        units.push(words[start..end].join(" "));
        if end == words.len() {
            break;
        }
        start += step;
    }
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
    /// `pub(crate)` (not `pub`) so tasks 4 and 5 can import
    /// `crate::embed::tests::BagOfWords`; `unreachable_pub` and
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
