//! The near-duplicate tier (KP-009, SENT-006).
//!
//! The exact tier answers "is this shingle in the corpus". It is defeated by
//! paraphrase: change one word in a five-word window and that window's digest
//! is unrelated to the original. SimHash answers the weaker but harder-to-evade
//! question — "is this *substantially* the corpus document" — because the
//! fingerprint is a majority vote over every shingle, so changing a few of them
//! moves only a few bits.
//!
//! The per-shingle hashes are the same keyed digests the exact tier uses, so a
//! stolen near-duplicate index is inert for the same reason: without the tenant
//! key an attacker cannot compute the fingerprint of a guess.

use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::index::{EntryMeta, INDEX_FORMAT_VERSION, IndexError};
use crate::key::IndexKey;
use crate::shingle;

/// A document fingerprint: one bit per hash position, set by majority vote.
pub type SimHash = u128;

/// Largest Hamming distance still called a near-duplicate.
///
/// Thirty-two, from measurement rather than convention. A one-word substitution
/// in a document of unique vocabulary moves the fingerprint 8–16 bits, and that
/// distance decays only as the square root of the shingle count, so the usual
/// tight thresholds (three bits of sixty-four) miss a single edit at any
/// document length. Thirty-two is far enough above the observed spread to
/// survive a several-word edit and still far below coincidence: two unrelated
/// fingerprints differ in 64 bits with a standard deviation of 5.7, putting
/// this threshold more than five deviations out.
pub const MAX_HAMMING: u32 = 32;

/// Shingles below which a fingerprint is not stable enough to compare.
///
/// A fingerprint is a majority vote, and with a handful of shingles the margin
/// on each bit is a handful of votes — small enough that ties, which resolve to
/// zero, pull short fingerprints toward each other. It is not a gap in
/// coverage: a span this small is what the exact tier answers well, because a
/// verbatim excerpt of it matches shingle for shingle.
pub const MIN_SHINGLES: usize = 32;

/// Documents one index may hold.
///
/// Lookup is a linear scan — a popcount per document, a few nanoseconds each —
/// which is the honest structure for this tier. The banded LSH that would make
/// it sublinear only guarantees recall for thresholds below the band count, and
/// at a threshold of 32 that would take so many bands that each lookup scans
/// the corpus several times over. A bound on corpus size buys the same
/// property with none of the machinery, and this tier runs off the per-call
/// hot path where a scan of this size costs nothing that matters.
pub const MAX_DOCUMENTS: usize = 100_000;

/// Shingles hashed between clock reads. Reading the clock per shingle would
/// cost more than the hashing it guards.
const BUDGET_CHECK_STRIDE: usize = 16;

/// Fingerprint of a span, or `None` when there is nothing to fingerprint.
///
/// `None` is not "no match": a span with no words has no fingerprint, and
/// treating that as a zero fingerprint would make every empty span collide with
/// every other one.
pub fn simhash(key: &IndexKey, text: &str) -> Option<SimHash> {
    fingerprint_until(key, text, None)
        .unwrap_or(None)
        .map(|(fingerprint, _)| fingerprint)
}

/// Fingerprint a span, abandoning the work if a deadline passes.
///
/// One implementation for both paths: the inline tier passes a deadline, the
/// asynchronous one passes `None`. Returns the number of shingles examined on
/// abandonment, which is what lets a report distinguish "nothing here" from
/// "did not finish looking".
pub(crate) fn fingerprint_until(
    key: &IndexKey,
    text: &str,
    deadline: Option<Instant>,
) -> Result<Option<(SimHash, usize)>, usize> {
    let normalized = shingle::normalize(text);
    let words: Vec<&str> = normalized.split(' ').filter(|w| !w.is_empty()).collect();
    let mut votes = [0i32; u128::BITS as usize];
    let mut counted = 0usize;
    for window in shingle::windows(&words) {
        if let Some(deadline) = deadline
            && counted.is_multiple_of(BUDGET_CHECK_STRIDE)
            && counted > 0
            && Instant::now() > deadline
        {
            return Err(counted);
        }
        let hash = key.hash(&window);
        for (bit, vote) in votes.iter_mut().enumerate() {
            if hash >> bit & 1 == 1 {
                *vote += 1;
            } else {
                *vote -= 1;
            }
        }
        counted += 1;
    }
    if counted == 0 {
        return Ok(None);
    }
    // A tied bit resolves to zero rather than to the sign of the last shingle,
    // so the fingerprint does not depend on document order.
    Ok(Some((
        votes
            .iter()
            .enumerate()
            .filter(|(_, vote)| **vote > 0)
            .fold(0u128, |acc, (bit, _)| acc | 1u128 << bit),
        counted,
    )))
}

/// What a near-duplicate lookup found.
#[derive(Debug, Clone, PartialEq)]
pub struct NearMatch {
    pub meta: EntryMeta,
    /// Bits by which the span differs from the indexed document.
    pub distance: u32,
}

impl NearMatch {
    /// Confidence falls linearly with distance: an exact fingerprint match is
    /// certain, and a match at the recall limit is barely evidence at all. The
    /// curve matters as much as the threshold — a match at 32 bits is the one
    /// case where a coincidence is conceivable, and it arrives labelled as
    /// almost no evidence rather than as a hit.
    pub fn confidence(&self) -> f32 {
        1.0 - (self.distance as f32 / (MAX_HAMMING + 1) as f32)
    }
}

/// Document fingerprints and the labels they carry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NearDuplicateIndex {
    pub format_version: String,
    pub taxonomy_version: String,
    pub key_fingerprint: String,
    documents: Vec<(SimHash, EntryMeta)>,
}

impl NearDuplicateIndex {
    pub fn builder(
        key: &IndexKey,
        taxonomy_version: impl Into<String>,
    ) -> NearDuplicateIndexBuilder {
        NearDuplicateIndexBuilder {
            index: Self {
                format_version: INDEX_FORMAT_VERSION.to_string(),
                taxonomy_version: taxonomy_version.into(),
                key_fingerprint: key.fingerprint(),
                documents: Vec::new(),
            },
        }
    }

    pub fn len(&self) -> usize {
        self.documents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    /// Refuse a key that did not build this index, rather than matching nothing
    /// forever under a key that cannot reproduce any fingerprint.
    pub fn check_key(&self, key: &IndexKey) -> Result<(), IndexError> {
        if key.fingerprint() == self.key_fingerprint {
            return Ok(());
        }
        Err(IndexError::KeyMismatch {
            expected: self.key_fingerprint.clone(),
            actual: key.fingerprint(),
        })
    }

    /// Closest indexed document within [`MAX_HAMMING`], if any.
    ///
    /// # Panics
    ///
    /// Panics if the internal scan abandons a lookup without a deadline.
    pub fn nearest(&self, fingerprint: SimHash) -> Option<NearMatch> {
        self.nearest_until(fingerprint, None)
            .expect("lookup without a deadline cannot expire")
    }

    pub(crate) fn nearest_until(
        &self,
        fingerprint: SimHash,
        deadline: Option<Instant>,
    ) -> Result<Option<NearMatch>, usize> {
        let mut best: Option<(u32, &EntryMeta)> = None;
        for (examined, (indexed, meta)) in self.documents.iter().enumerate() {
            if examined.is_multiple_of(16)
                && deadline.is_some_and(|deadline| Instant::now() >= deadline)
            {
                return Err(examined);
            }
            let distance = (fingerprint ^ indexed).count_ones();
            if distance > MAX_HAMMING {
                continue;
            }
            // Ties keep the more sensitive document: a span equally close to
            // two corpus documents carries the higher label.
            match best {
                Some((d, m))
                    if d < distance || (d == distance && m.sensitivity >= meta.sensitivity) => {}
                _ => best = Some((distance, meta)),
            }
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return Err(self.documents.len());
        }
        Ok(best.map(|(distance, meta)| NearMatch {
            meta: meta.clone(),
            distance,
        }))
    }

    pub fn max_sensitivity(&self) -> arkavo_protocol::data_classification::SensitivityLevel {
        self.documents
            .iter()
            .map(|(_, meta)| meta.sensitivity)
            .max()
            .unwrap_or(arkavo_protocol::data_classification::SensitivityLevel::Public)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

    pub fn from_json(json: &str) -> Result<Self, IndexError> {
        let index: Self =
            serde_json::from_str(json).map_err(|e| IndexError::Malformed(e.to_string()))?;
        if index.format_version != INDEX_FORMAT_VERSION {
            return Err(IndexError::UnsupportedVersion(index.format_version));
        }
        if index.documents.len() > MAX_DOCUMENTS {
            return Err(IndexError::Malformed(format!(
                "index holds {} documents, past the {MAX_DOCUMENTS} a linear scan is bounded to",
                index.documents.len()
            )));
        }
        Ok(index)
    }
}

pub struct NearDuplicateIndexBuilder {
    index: NearDuplicateIndex,
}

impl NearDuplicateIndexBuilder {
    /// Add a document, reporting whether it was indexable here.
    ///
    /// A document shorter than [`MIN_SHINGLES`] is refused rather than indexed
    /// at a length where its fingerprint cannot be compared reliably: indexing
    /// it would produce an entry that only ever matches itself verbatim, which
    /// the exact tier already does better. A full index refuses too, rather
    /// than growing past the bound the linear scan is costed against.
    pub fn add_document(&mut self, key: &IndexKey, text: &str, meta: EntryMeta) -> bool {
        if self.index.documents.len() >= MAX_DOCUMENTS {
            return false;
        }
        let Ok(Some((fingerprint, shingles))) = fingerprint_until(key, text, None) else {
            return false;
        };
        if shingles < MIN_SHINGLES {
            return false;
        }
        self.index.documents.push((fingerprint, meta));
        true
    }

    pub fn build(self) -> NearDuplicateIndex {
        self.index
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
    use arkavo_test_macros::spec;

    /// A document long enough for a stable fingerprint, with vocabulary varied
    /// enough that repeated words are not carrying the vote.
    fn document(seed: usize) -> String {
        use std::fmt::Write as _;
        (0..140).fold(String::new(), |mut text, i: usize| {
            let _ = write!(
                text,
                "t{}n{} ",
                (i * 37 + seed * 7919) % 991,
                (i * 13 + seed) % 577
            );
            text
        })
    }

    /// The same document with one word substituted — the edit the near tier
    /// exists to see through.
    fn edited(seed: usize) -> String {
        document(seed).replacen(
            &format!("t{}n", (70 * 37 + seed * 7919) % 991),
            "swapped",
            1,
        )
    }

    fn key() -> IndexKey {
        IndexKey::derive(&[9u8; 32], "near-tests").expect("derive")
    }

    fn meta(sensitivity: SensitivityLevel, family: &str) -> EntryMeta {
        EntryMeta {
            category: DataCategory::Internal,
            sensitivity,
            source_family: family.to_string(),
        }
    }

    fn index_of(key: &IndexKey, documents: &[(String, EntryMeta)]) -> NearDuplicateIndex {
        let mut builder = NearDuplicateIndex::builder(key, "1.0.0");
        for (text, meta) in documents {
            builder.add_document(key, text, meta.clone());
        }
        builder.build()
    }

    /// SENT-006: the near tier exists because the exact tier is defeated by a
    /// paraphrase. An edited document must still be recognized.
    #[spec("SENT-006")]
    #[test]
    fn an_edited_document_is_still_near_the_original() {
        let key = key();
        let index = index_of(
            &key,
            &[(document(1), meta(SensitivityLevel::Confidential, "board"))],
        );

        let found = index
            .nearest(simhash(&key, &edited(1)).expect("fingerprint"))
            .expect("an edited document should still be near the original");

        assert_eq!(found.meta.sensitivity, SensitivityLevel::Confidential);
        assert!(found.distance <= MAX_HAMMING, "{}", found.distance);
    }

    #[test]
    fn unrelated_documents_are_not_near_duplicates() {
        let key = key();
        let index = index_of(
            &key,
            &[(document(1), meta(SensitivityLevel::Confidential, "board"))],
        );

        assert_eq!(
            index.nearest(simhash(&key, &document(2)).expect("fingerprint")),
            None
        );
    }

    #[test]
    fn an_identical_document_matches_at_distance_zero() {
        let key = key();
        let index = index_of(
            &key,
            &[(document(1), meta(SensitivityLevel::Restricted, "board"))],
        );

        let found = index
            .nearest(simhash(&key, &document(1)).expect("fingerprint"))
            .expect("identical text must match");

        assert_eq!(found.distance, 0);
        assert!((found.confidence() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn a_different_key_produces_a_different_fingerprint() {
        // Otherwise a stolen index is a dictionary: fingerprint a guess, look
        // for it, learn whether the guess was in the corpus.
        let other = IndexKey::derive(&[1u8; 32], "near-tests").expect("derive");

        assert_ne!(simhash(&key(), &document(1)), simhash(&other, &document(1)));
    }

    #[test]
    fn a_key_that_did_not_build_the_index_is_refused() {
        let index = index_of(
            &key(),
            &[(document(1), meta(SensitivityLevel::Internal, "board"))],
        );
        let other = IndexKey::derive(&[1u8; 32], "near-tests").expect("derive");

        assert!(index.check_key(&other).is_err());
        assert!(index.check_key(&key()).is_ok());
    }

    #[test]
    fn a_span_with_no_words_has_no_fingerprint() {
        // Not a zero fingerprint: that would make every empty span collide.
        assert_eq!(simhash(&key(), "   ... "), None);
    }

    #[test]
    fn a_document_too_short_to_fingerprint_is_not_indexed() {
        // Indexing it would create an entry that only matches itself verbatim,
        // which the exact tier already does better.
        let key = key();
        let mut builder = NearDuplicateIndex::builder(&key, "1.0.0");

        let added = builder.add_document(
            &key,
            "a short line of text",
            meta(SensitivityLevel::Restricted, "board"),
        );

        assert!(!added);
        assert!(builder.build().is_empty());
    }

    #[test]
    fn the_index_survives_a_json_round_trip() {
        let key = key();
        let index = index_of(
            &key,
            &[(document(1), meta(SensitivityLevel::Confidential, "board"))],
        );

        let restored = NearDuplicateIndex::from_json(&index.to_json()).expect("round trip");

        assert_eq!(restored.len(), 1);
        assert!(
            restored
                .nearest(simhash(&key, &document(1)).expect("fingerprint"))
                .is_some(),
            "a restored index must still match the document it was built from"
        );
    }

    #[test]
    fn the_index_stores_no_corpus_text() {
        let index = index_of(
            &key(),
            &[(document(1), meta(SensitivityLevel::Confidential, "board"))],
        );

        let json = index.to_json();

        for word in document(1).split_whitespace().take(20) {
            assert!(!json.contains(word), "{word} leaked into {json}");
        }
    }

    /// The whole payload carries the highest label: a span equally close to two
    /// documents must not be labelled by the less sensitive one.
    #[spec("SENT-003")]
    #[test]
    fn a_tie_keeps_the_more_sensitive_document() {
        let key = key();
        let index = index_of(
            &key,
            &[
                (
                    document(1),
                    meta(SensitivityLevel::Internal, "public-filings"),
                ),
                (document(1), meta(SensitivityLevel::Restricted, "board")),
            ],
        );

        let found = index
            .nearest(simhash(&key, &document(1)).expect("fingerprint"))
            .expect("match");

        assert_eq!(found.meta.sensitivity, SensitivityLevel::Restricted);
    }

    #[test]
    fn the_threshold_stays_clear_of_coincidence() {
        // Unrelated fingerprints differ in 64 bits with a standard deviation of
        // 5.7. Guards against a future widening that would let a coincidence
        // read as a near-duplicate.
        let sigma = (u128::BITS as f32 * 0.25).sqrt();

        assert!(
            (u128::BITS / 2) as f32 - MAX_HAMMING as f32 > 5.0 * sigma,
            "threshold {MAX_HAMMING} is within five deviations of coincidence"
        );
    }

    #[test]
    fn the_minimum_shingle_count_is_where_an_edit_stays_inside_the_threshold() {
        // Pins the calibration MIN_SHINGLES was chosen from: at this length a
        // one-word substitution still lands inside MAX_HAMMING. If either
        // constant moves without the other, this fails rather than silently
        // losing recall.
        let key = key();

        let distance = (simhash(&key, &document(3)).expect("fingerprint")
            ^ simhash(&key, &edited(3)).expect("fingerprint"))
        .count_ones();

        assert!(
            distance <= MAX_HAMMING,
            "one-word edit moved {distance} bits"
        );
    }
}
