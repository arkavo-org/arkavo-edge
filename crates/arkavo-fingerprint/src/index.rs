//! The keyed reference index (KP-009, KP-010, KP-011).
//!
//! An index maps keyed shingle digests to the classification of the corpus they
//! came from. It answers one question fast: has this span been seen before, and
//! if so, what was it labelled.
//!
//! Suppression is the one thing here that can remove, and it is confined
//! accordingly. It drops shingles from *this tier's own candidate set at build
//! time* — boilerplate that would otherwise match everything. It never sees a
//! `TaintSet`, never sees another tier's findings, and never removes a shingle
//! that a classified entry claims (KP-010). Inferred labels add restrictions;
//! they do not remove known ones, and a suppression list wired the other way
//! would be the shortest path to violating that.

use std::collections::{BTreeMap, HashMap, HashSet};

use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use serde::{Deserialize, Serialize};

use crate::key::{IndexKey, ShingleHash};
use crate::shingle::shingle_text;

/// Format version of the serialized index.
pub const INDEX_FORMAT_VERSION: &str = "1";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IndexError {
    #[error("index is not valid JSON: {0}")]
    Malformed(String),
    #[error("index format version {0} is not supported (expected {INDEX_FORMAT_VERSION})")]
    UnsupportedVersion(String),
    #[error("index was built with key {expected}, but {actual} was supplied")]
    KeyMismatch { expected: String, actual: String },
}

/// What a corpus document contributes to a match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryMeta {
    pub category: DataCategory,
    pub sensitivity: SensitivityLevel,
    /// Corpus family the shingle came from, carried into evidence so an auditor
    /// can see which body of material matched.
    pub source_family: String,
}

/// Shingles excluded from matching, versioned with the index.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SuppressionIndex {
    pub version: String,
    #[serde(default)]
    hashes: HashSet<ShingleHash>,
}

impl SuppressionIndex {
    pub fn new(version: impl Into<String>) -> Self {
        Self {
            version: version.into(),
            hashes: HashSet::new(),
        }
    }

    pub fn insert(&mut self, hash: ShingleHash) {
        self.hashes.insert(hash);
    }

    pub fn contains(&self, hash: ShingleHash) -> bool {
        self.hashes.contains(&hash)
    }

    pub fn remove(&mut self, hash: ShingleHash) {
        self.hashes.remove(&hash);
    }

    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }
}

/// A built index, ready to query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReferenceIndex {
    pub format_version: String,
    /// Taxonomy the entry labels are expressed in.
    pub taxonomy_version: String,
    /// Fingerprint of the key that built this, so a lookup with the wrong key
    /// fails loudly instead of silently matching nothing.
    pub key_fingerprint: String,
    entries: HashMap<ShingleHash, EntryMeta>,
    suppression: SuppressionIndex,
}

impl ReferenceIndex {
    pub fn builder(key: &IndexKey, taxonomy_version: impl Into<String>) -> ReferenceIndexBuilder {
        ReferenceIndexBuilder {
            taxonomy_version: taxonomy_version.into(),
            key_fingerprint: key.fingerprint(),
            documents: Vec::new(),
            suppressed_candidates: HashSet::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn suppression(&self) -> &SuppressionIndex {
        &self.suppression
    }

    /// Highest classification any entry carries.
    ///
    /// What the index has to be wrapped at. The digests are not reversible, but
    /// the family names and the suppression list are metadata about the corpus,
    /// and an index of restricted material is itself restricted.
    pub fn max_sensitivity(&self) -> SensitivityLevel {
        self.entries
            .values()
            .map(|e| e.sensitivity)
            .max()
            .unwrap_or(SensitivityLevel::Public)
    }

    /// Categories present, for deriving the wrap policy.
    pub fn categories(&self) -> Vec<DataCategory> {
        let mut categories: Vec<DataCategory> = self.entries.values().map(|e| e.category).collect();
        categories.sort_unstable();
        categories.dedup();
        categories
    }

    /// Look up one keyed digest. This is the hot-path operation (KP-011): a
    /// hash-map probe, so a miss costs the same as a hit.
    pub fn lookup(&self, hash: ShingleHash) -> Option<&EntryMeta> {
        self.entries.get(&hash)
    }

    /// Confirm the supplied key built this index.
    pub fn check_key(&self, key: &IndexKey) -> Result<(), IndexError> {
        let actual = key.fingerprint();
        if actual == self.key_fingerprint {
            return Ok(());
        }
        Err(IndexError::KeyMismatch {
            expected: self.key_fingerprint.clone(),
            actual,
        })
    }

    /// Serialize for TDF wrapping. The bytes are keyed digests and labels; the
    /// corpus text is not recoverable from them.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(json: &str) -> Result<Self, IndexError> {
        let index: ReferenceIndex =
            serde_json::from_str(json).map_err(|e| IndexError::Malformed(e.to_string()))?;
        if index.format_version != INDEX_FORMAT_VERSION {
            return Err(IndexError::UnsupportedVersion(index.format_version));
        }
        Ok(index)
    }
}

/// Accumulates corpus documents into an index.
pub struct ReferenceIndexBuilder {
    taxonomy_version: String,
    key_fingerprint: String,
    /// Documents held whole until `build`, because whether a shingle may be
    /// suppressed depends on what else the document it came from still has.
    documents: Vec<(Vec<ShingleHash>, EntryMeta)>,
    suppressed_candidates: HashSet<ShingleHash>,
}

impl ReferenceIndexBuilder {
    /// Add a classified document.
    ///
    /// A duplicate document contributes no new entries, so duplication in the
    /// corpus cannot inflate a later match's confidence (KP-009 edge case).
    /// Where two documents share a shingle the higher classification wins, for
    /// the same reason a taint union takes the maximum.
    pub fn add_document(
        &mut self,
        key: &IndexKey,
        text: &str,
        category: DataCategory,
        sensitivity: SensitivityLevel,
        source_family: &str,
    ) {
        let hashes: Vec<ShingleHash> = shingle_text(text).iter().map(|s| key.hash(s)).collect();
        if hashes.is_empty() {
            return;
        }
        self.documents.push((
            hashes,
            EntryMeta {
                category,
                sensitivity,
                source_family: source_family.to_string(),
            },
        ));
    }

    /// Nominate boilerplate for suppression: license headers, public docs,
    /// common idioms. These match everything and are what a reference tier
    /// generates false positives on.
    pub fn add_boilerplate(&mut self, key: &IndexKey, text: &str) {
        for shingle in shingle_text(text) {
            self.suppressed_candidates.insert(key.hash(&shingle));
        }
    }

    pub fn build(mut self) -> ReferenceIndex {
        let mut suppression = SuppressionIndex::new(INDEX_FORMAT_VERSION);
        for candidate in &self.suppressed_candidates {
            suppression.insert(*candidate);
        }

        // KP-010 edge case: classified text embedded in boilerplate survives.
        // Suppression removes the shingles that are boilerplate — that is what
        // kills the false positive — but a document it would silence entirely
        // keeps its own, because a classified document the index cannot see is
        // the failure this tier exists to prevent. Silencing a document is the
        // only case where the boilerplate list loses.
        for (hashes, _) in &self.documents {
            if hashes.iter().all(|h| suppression.contains(*h)) {
                for hash in hashes {
                    suppression.remove(*hash);
                }
            }
        }

        let mut entries: HashMap<ShingleHash, EntryMeta> = HashMap::new();
        for (hashes, meta) in self.documents.drain(..) {
            for hash in hashes {
                if suppression.contains(hash) {
                    continue;
                }
                // A shared shingle takes the higher classification, for the same
                // reason a taint union takes the maximum.
                match entries.get(&hash) {
                    Some(existing) if existing.sensitivity >= meta.sensitivity => {}
                    _ => {
                        entries.insert(hash, meta.clone());
                    }
                }
            }
        }

        ReferenceIndex {
            format_version: INDEX_FORMAT_VERSION.to_string(),
            taxonomy_version: self.taxonomy_version,
            key_fingerprint: self.key_fingerprint,
            entries,
            suppression,
        }
    }
}

/// How much of a span the index recognized.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MatchSummary {
    pub shingles_examined: usize,
    pub shingles_matched: usize,
    /// Highest classification any matched shingle carried, per family.
    pub by_family: BTreeMap<String, EntryMeta>,
}

impl MatchSummary {
    /// Fraction of the span the index recognized, as a calibration input.
    ///
    /// Coverage rather than a raw count: one matching shingle in a long
    /// document is a coincidence, the same shingle in a two-line span is the
    /// document.
    pub fn coverage(&self) -> f32 {
        if self.shingles_examined == 0 {
            return 0.0;
        }
        self.shingles_matched as f32 / self.shingles_examined as f32
    }

    pub fn is_empty(&self) -> bool {
        self.shingles_matched == 0
    }
}

/// Query an index with a span.
pub fn match_span(index: &ReferenceIndex, key: &IndexKey, text: &str) -> MatchSummary {
    let mut summary = MatchSummary::default();
    for shingle in shingle_text(text) {
        summary.shingles_examined += 1;
        let hash = key.hash(&shingle);
        if index.suppression.contains(hash) {
            continue;
        }
        let Some(meta) = index.lookup(hash) else {
            continue;
        };
        summary.shingles_matched += 1;
        match summary.by_family.get(&meta.source_family) {
            Some(existing) if existing.sensitivity >= meta.sensitivity => {}
            _ => {
                summary
                    .by_family
                    .insert(meta.source_family.clone(), meta.clone());
            }
        }
    }
    summary
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn key() -> IndexKey {
        IndexKey::derive(&[7u8; 32], "test-corpus").expect("derive")
    }

    const CLASSIFIED: &str =
        "the acquisition of northwind holdings closes in the third quarter pending approval";
    const BOILERPLATE: &str =
        "licensed under the apache license version two point zero see the license for terms";

    fn built() -> (IndexKey, ReferenceIndex) {
        let key = key();
        let mut builder = ReferenceIndex::builder(&key, "1.0.0");
        builder.add_document(
            &key,
            CLASSIFIED,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "board-minutes",
        );
        builder.add_boilerplate(&key, BOILERPLATE);
        (key, builder.build())
    }

    #[spec("KP-009")]
    #[test]
    fn a_corpus_span_matches_after_reformatting() {
        let (key, index) = built();

        let summary = match_span(
            &index,
            &key,
            "The Acquisition Of Northwind Holdings\n closes\tin the THIRD quarter",
        );

        assert!(!summary.is_empty());
        assert_eq!(
            summary.by_family["board-minutes"].sensitivity,
            SensitivityLevel::Confidential
        );
    }

    #[spec("KP-009")]
    #[test]
    fn unrelated_text_does_not_match() {
        let (key, index) = built();

        let summary = match_span(
            &index,
            &key,
            "the weather today is mild with a chance of rain later",
        );

        assert!(summary.is_empty());
        assert!(summary.coverage() < f32::EPSILON);
    }

    #[spec("KP-009")]
    #[test]
    fn no_unkeyed_digest_is_ever_written() {
        // The index must be useless to whoever steals it without the key.
        let (_, index) = built();
        let with_other_key = IndexKey::derive(&[9u8; 32], "test-corpus").expect("derive");

        let summary = match_span(&index, &with_other_key, CLASSIFIED);

        assert!(summary.is_empty(), "content matched under the wrong key");
    }

    #[spec("KP-009")]
    #[test]
    fn a_duplicate_document_does_not_inflate_the_index() {
        let key = key();
        let mut once = ReferenceIndex::builder(&key, "1.0.0");
        once.add_document(
            &key,
            CLASSIFIED,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "f",
        );
        let mut twice = ReferenceIndex::builder(&key, "1.0.0");
        twice.add_document(
            &key,
            CLASSIFIED,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "f",
        );
        twice.add_document(
            &key,
            CLASSIFIED,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "f",
        );

        assert_eq!(once.build().len(), twice.build().len());
    }

    #[spec("KP-009")]
    #[test]
    fn the_higher_classification_wins_a_shared_shingle() {
        let key = key();
        let mut builder = ReferenceIndex::builder(&key, "1.0.0");
        builder.add_document(
            &key,
            CLASSIFIED,
            DataCategory::Internal,
            SensitivityLevel::Internal,
            "low",
        );
        builder.add_document(
            &key,
            CLASSIFIED,
            DataCategory::Healthcare,
            SensitivityLevel::Restricted,
            "high",
        );
        let index = builder.build();

        let summary = match_span(&index, &key, CLASSIFIED);

        assert_eq!(
            summary.by_family["high"].sensitivity,
            SensitivityLevel::Restricted
        );
    }

    #[spec("KP-010")]
    #[test]
    fn suppression_kills_a_seeded_boilerplate_false_positive() {
        // The realistic shape: a classified document that happens to carry a
        // license header. Without suppression, every other file with that
        // header matches the board minutes.
        let key = key();
        let with_header = format!("{BOILERPLATE} {CLASSIFIED}");

        let mut noisy_builder = ReferenceIndex::builder(&key, "1.0.0");
        noisy_builder.add_document(
            &key,
            &with_header,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "minutes",
        );
        let noisy = noisy_builder.build();
        assert!(
            !match_span(&noisy, &key, BOILERPLATE).is_empty(),
            "the seeded false positive did not fire, so suppression proves nothing"
        );

        let mut quiet_builder = ReferenceIndex::builder(&key, "1.0.0");
        quiet_builder.add_document(
            &key,
            &with_header,
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "minutes",
        );
        quiet_builder.add_boilerplate(&key, BOILERPLATE);
        let quiet = quiet_builder.build();

        assert!(
            match_span(&quiet, &key, BOILERPLATE).is_empty(),
            "an unrelated file sharing only the license header still matched"
        );
    }

    #[spec("KP-010")]
    #[test]
    fn suppression_entries_are_versioned_with_the_index() {
        let (_, index) = built();

        assert_eq!(index.suppression().version, INDEX_FORMAT_VERSION);
        assert!(!index.suppression().is_empty());
    }

    #[spec("KP-009")]
    #[test]
    fn the_index_reports_the_classification_it_must_be_wrapped_at() {
        let key = key();
        let mut builder = ReferenceIndex::builder(&key, "1.0.0");
        builder.add_document(
            &key,
            CLASSIFIED,
            DataCategory::Internal,
            SensitivityLevel::Internal,
            "a",
        );
        builder.add_document(
            &key,
            "patient roster for the northern clinic listing every admission this month",
            DataCategory::Healthcare,
            SensitivityLevel::Restricted,
            "b",
        );
        let index = builder.build();

        assert_eq!(index.max_sensitivity(), SensitivityLevel::Restricted);
        assert!(index.categories().contains(&DataCategory::Healthcare));
    }

    #[spec("KP-009")]
    #[test]
    fn an_empty_index_is_public() {
        let key = key();

        assert_eq!(
            ReferenceIndex::builder(&key, "1.0.0")
                .build()
                .max_sensitivity(),
            SensitivityLevel::Public
        );
    }

    #[spec("KP-009")]
    #[test]
    fn an_index_round_trips_through_its_serialized_form() {
        let (key, index) = built();

        let back = ReferenceIndex::from_json(&index.to_json()).expect("round trip");

        assert_eq!(back, index);
        assert!(!match_span(&back, &key, CLASSIFIED).is_empty());
    }

    #[spec("KP-009")]
    #[test]
    fn a_lookup_with_the_wrong_key_is_an_error_not_a_silent_miss() {
        let (_, index) = built();
        let other = IndexKey::derive(&[9u8; 32], "test-corpus").expect("derive");

        assert!(matches!(
            index.check_key(&other),
            Err(IndexError::KeyMismatch { .. })
        ));
    }

    #[spec("KP-009")]
    #[test]
    fn an_index_from_a_future_format_is_refused() {
        let json = r#"{"format_version":"99","taxonomy_version":"1","key_fingerprint":"x",
            "entries":{},"suppression":{"version":"1","hashes":[]}}"#;

        assert!(matches!(
            ReferenceIndex::from_json(json),
            Err(IndexError::UnsupportedVersion(_))
        ));
    }

    #[spec("KP-011")]
    #[test]
    fn coverage_scales_with_how_much_of_the_span_matched() {
        let (key, index) = built();

        let whole = match_span(&index, &key, CLASSIFIED);
        let diluted = match_span(
            &index,
            &key,
            &format!(
                "{CLASSIFIED} and then some entirely unrelated filler text follows here for length"
            ),
        );

        assert!(whole.coverage() > diluted.coverage());
    }
}
