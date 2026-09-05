//! Shared exact matching for inline and deferred classification.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

use arkavo_protocol::data_classification::DataCategory;

use crate::index::{EntryMeta, ReferenceIndex};
use crate::key::IndexKey;
use crate::shingle::{SHINGLE_WORDS, normalize};

/// How much of a span the index recognized.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MatchSummary {
    pub shingles_examined: usize,
    pub shingles_matched: usize,
    /// Highest classification any matched shingle carried, per family.
    pub by_family: BTreeMap<String, EntryMeta>,
    /// Independent categories must survive even when their family is shared.
    pub(crate) labels: BTreeMap<(String, DataCategory), EntryMeta>,
    pub(crate) complete_documents: BTreeSet<(String, DataCategory)>,
}

impl MatchSummary {
    pub fn coverage(&self) -> f32 {
        if self.shingles_examined == 0 {
            return 0.0;
        }
        self.shingles_matched as f32 / self.shingles_examined as f32
    }

    pub fn is_empty(&self) -> bool {
        self.labels.is_empty()
    }

    /// Every matched category and source family, with its highest sensitivity.
    pub fn labels(&self) -> impl Iterator<Item = &EntryMeta> {
        self.labels.values()
    }

    fn record(&mut self, meta: &EntryMeta, complete_document: bool) {
        let label = (meta.source_family.clone(), meta.category);
        if complete_document {
            self.complete_documents.insert(label.clone());
        }
        let existing = self.labels.entry(label).or_insert_with(|| meta.clone());
        existing.sensitivity = existing.sensitivity.max(meta.sensitivity);
        let family = self
            .by_family
            .entry(meta.source_family.clone())
            .or_insert_with(|| meta.clone());
        if meta.sensitivity > family.sensitivity {
            *family = meta.clone();
        }
    }
}

/// Query an index without a deadline, for deferred classification.
///
/// # Panics
///
/// Panics if the internal matcher abandons a lookup without a deadline.
pub fn match_span(index: &ReferenceIndex, key: &IndexKey, text: &str) -> MatchSummary {
    match_until(index, key, text, None).expect("matching without a deadline cannot expire")
}

pub(crate) fn match_until(
    index: &ReferenceIndex,
    key: &IndexKey,
    text: &str,
    deadline: Option<Instant>,
) -> Result<MatchSummary, usize> {
    let expired = || deadline.is_some_and(|deadline| Instant::now() >= deadline);
    if expired() {
        return Err(0);
    }
    let normalized = normalize(text);
    let words: Vec<&str> = normalized
        .split(' ')
        .filter(|word| !word.is_empty())
        .collect();
    let mut summary = MatchSummary::default();
    if words.is_empty() {
        return Ok(summary);
    }
    let primary_width = words.len().min(SHINGLE_WORDS);
    let mut examined = 0usize;
    let widths = std::iter::once(SHINGLE_WORDS)
        .chain(index.short_widths.iter().copied())
        .filter(|&width| width > 0 && width <= words.len());
    for width in widths {
        for window in words.windows(width) {
            if examined.is_multiple_of(16) && expired() {
                return Err(examined);
            }
            examined += 1;
            // Extra short-document probes must not dilute five-word coverage.
            if width == primary_width {
                summary.shingles_examined += 1;
            }
            let hash = key.hash(&window.join(" "));
            if index.suppression().contains(hash) {
                continue;
            }
            let mut matched = false;
            for meta in index.lookup_all(hash) {
                matched = true;
                // A short digest can only originate from an entire short corpus
                // document. Context added around that document cannot weaken it.
                summary.record(meta, width < SHINGLE_WORDS);
            }
            if width == primary_width {
                summary.shingles_matched += usize::from(matched);
            }
        }
    }
    if expired() {
        return Err(examined);
    }
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    use arkavo_protocol::classification_evidence::Confidence;
    use arkavo_protocol::data_classification::SensitivityLevel;

    use crate::tier::ReferenceTier;

    fn key() -> Arc<IndexKey> {
        Arc::new(IndexKey::derive(&[7; 32], "regression").unwrap())
    }

    #[test]
    fn shared_digest_preserves_categories_after_serialization() {
        let key = key();
        let mut builder = ReferenceIndex::builder(&key, "1.0.0");
        for category in [DataCategory::Internal, DataCategory::Credentials] {
            builder.add_document(
                &key,
                "alpha bravo charlie delta echo",
                category,
                SensitivityLevel::Restricted,
                "same-family",
            );
        }
        let index = ReferenceIndex::from_json(&builder.build().to_json()).unwrap();
        assert_eq!(index.categories().len(), 2);
        let tier = ReferenceTier::loaded(Arc::new(index), key);
        for report in [
            tier.examine_unbudgeted("alpha bravo charlie delta echo"),
            tier.examine_until(
                "alpha bravo charlie delta echo",
                Instant::now() + Duration::from_secs(1),
            ),
        ] {
            assert_eq!(report.findings().len(), 2);
            assert!(
                report
                    .findings()
                    .iter()
                    .any(|finding| finding.category == DataCategory::Credentials)
            );
        }
    }

    #[test]
    fn distinct_documents_in_one_family_preserve_categories() {
        let key = key();
        let mut builder = ReferenceIndex::builder(&key, "1.0.0");
        for (text, category) in [
            ("alpha bravo charlie delta echo", DataCategory::Internal),
            ("foxtrot golf hotel india juliet", DataCategory::Credentials),
        ] {
            builder.add_document(&key, text, category, SensitivityLevel::Restricted, "shared");
        }
        let tier = ReferenceTier::loaded(Arc::new(builder.build()), key);
        let text = "alpha bravo charlie delta echo foxtrot golf hotel india juliet";
        for report in [
            tier.examine_unbudgeted(text),
            tier.examine_until(text, Instant::now() + Duration::from_secs(1)),
        ] {
            assert_eq!(report.findings().len(), 2);
        }
    }

    #[test]
    fn complete_short_document_matches_with_surrounding_context() {
        let key = key();
        for secret in [
            "orchid",
            "orchid acquisition",
            "orchid acquisition approved",
            "orchid acquisition approved yesterday",
        ] {
            let mut builder = ReferenceIndex::builder(&key, "1.0.0");
            builder.add_document(
                &key,
                secret,
                DataCategory::Internal,
                SensitivityLevel::Confidential,
                "short",
            );
            let index = ReferenceIndex::from_json(&builder.build().to_json()).unwrap();
            let tier = ReferenceTier::loaded(Arc::new(index), key.clone());
            let query = format!("Please send {secret} today with the usual message");
            for report in [
                tier.examine_unbudgeted(&query),
                tier.examine_until(&query, Instant::now() + Duration::from_secs(1)),
            ] {
                assert_eq!(report.findings().len(), 1, "one finding per planted secret");
                assert_eq!(report.findings()[0].confidence, Confidence::CERTAIN);
            }
        }
    }

    #[test]
    fn legacy_short_index_matches_surrounding_context() {
        let key = key();
        let mut builder = ReferenceIndex::builder(&key, "1.0.0");
        builder.add_document(
            &key,
            "orchid acquisition approved",
            DataCategory::Internal,
            SensitivityLevel::Confidential,
            "short",
        );
        let json = builder.build().to_json();
        // Edit the string directly because a generic JSON Value narrows u128 digests.
        let json = json.replace(",\"short_widths\":[3]", "");
        assert!(!json.contains("short_widths"));
        let index = ReferenceIndex::from_json(&json).unwrap();
        assert!(
            !match_span(
                &index,
                &key,
                "Please send orchid acquisition approved today"
            )
            .is_empty()
        );
    }
    #[test]
    fn short_document_probes_do_not_dilute_long_document_confidence() {
        let key = key();
        let mut builder = ReferenceIndex::builder(&key, "1.0.0");
        for text in [
            "orchid acquisition",
            "alpha bravo charlie delta echo foxtrot",
        ] {
            builder.add_document(
                &key,
                text,
                DataCategory::Internal,
                SensitivityLevel::Confidential,
                "corpus",
            );
        }
        let tier = ReferenceTier::loaded(Arc::new(builder.build()), key);
        let report = tier.examine_unbudgeted("alpha bravo charlie delta echo foxtrot");
        assert_eq!(report.findings()[0].confidence, Confidence::CERTAIN);
    }
}
