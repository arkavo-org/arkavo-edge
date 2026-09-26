//! Calibrating semantic margins into thresholds.
//!
//! A margin is not a probability: it is a bare `cos - cos` that can be
//! negative, has no fixed scale across labels, and means nothing until it is
//! compared against a target false-positive rate on held-out data. This module
//! is where that comparison happens, once per label, and where the evidence
//! that justifies the resulting threshold is written down alongside it.
//!
//! Held-out means held-out by *family*, not by sample: a random split of near-
//! duplicate paraphrases would let the same underlying content appear on both
//! sides, and a threshold "validated" that way would really only be validated
//! against itself.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::embed::{Embedder, embed_units};
use crate::semantic_index::{EmbedderRecord, SemanticIndex};

/// A calibrated set of per-label thresholds, tied to the exact embedder and
/// taxonomy that produced them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticCalibration {
    pub detector_version: String,
    pub taxonomy_version: String,
    pub margins: BTreeMap<String, f32>,
    pub embedder: EmbedderRecord,
}

impl SemanticCalibration {
    /// The detector version a calibration built against `sha256` must carry.
    /// Namespaced so it can never collide with a keyed-tier version string.
    pub fn detector_version_for(sha256: &str) -> String {
        format!("semantic:{sha256}")
    }
}

/// One labelled or unlabelled text offered to `calibrate`.
pub struct CalibrationSample {
    pub text: String,
    /// Which corpus family this sample belongs to, for the family-level
    /// fit/held-out split. Never corpus-derived text: family ids are stored
    /// in the sealed index and surface in audit evidence, so a family named
    /// after a document title would leak that title.
    pub family: String,
    /// `Some(label_key)` for a positive; `None` for a negative.
    pub label: Option<String>,
    /// How this sample was produced (e.g. "rewrite", "translation", "short"),
    /// so held-out evidence can be broken down by the transformation that
    /// generated it rather than reported as one undifferentiated count.
    pub kind: String,
}

/// One label's calibrated threshold and the evidence behind it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LabelEvidence {
    pub label: String,
    pub threshold: f32,
    pub fit_positives: usize,
    pub fit_negatives: usize,
    /// kind -> (caught, total), over held-out positives labelled for this key.
    pub held_out_positives: BTreeMap<String, (usize, usize)>,
    /// kind -> (fired, total), over held-out negatives.
    pub held_out_negatives: BTreeMap<String, (usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticEvalEvidence {
    pub embedder_sha256: String,
    pub target_fpr: f32,
    pub labels: Vec<LabelEvidence>,
}

/// Distinct family names, sorted, assigned alternately to the fitting half
/// (index 0, 2, 4, ...); the rest are held out. Deterministic, and both halves
/// are non-empty whenever there are at least two families.
fn split_families(samples: &[CalibrationSample]) -> BTreeSet<String> {
    split_family_names(samples.iter().map(|s| s.family.as_str()))
}

/// Same split, taken directly over family names rather than samples — lets a
/// caller split just one label's positives without collecting a filtered copy
/// of them into a new `Vec<CalibrationSample>` first.
fn split_family_names<'a>(families: impl Iterator<Item = &'a str>) -> BTreeSet<String> {
    let all: BTreeSet<String> = families.map(str::to_string).collect();
    all.into_iter()
        .enumerate()
        .filter_map(|(i, family)| (i % 2 == 0).then_some(family))
        .collect()
}

/// Best margin per label for one sample, maxed over its units. Computed once
/// per sample rather than once per (sample, label) — `SemanticIndex::margins`
/// already returns every label's best margin for a unit in one pass.
fn label_scores(index: &SemanticIndex, units: &[Vec<f32>]) -> BTreeMap<String, f32> {
    let mut best: BTreeMap<String, f32> = BTreeMap::new();
    for unit in units {
        for (label, (margin, _family)) in index.margins(unit) {
            let slot = best.entry(label).or_insert(f32::NEG_INFINITY);
            if margin > *slot {
                *slot = margin;
            }
        }
    }
    best
}

fn score_for(scores: &BTreeMap<String, f32>, label: &str) -> f32 {
    scores.get(label).copied().unwrap_or(f32::NEG_INFINITY)
}

/// Calibrate one threshold per label the index holds, at `target_fpr` measured
/// on a held-out family split.
///
/// Rules:
/// - `target_fpr` must be a finite number in `(0.0, 1.0)` — a rate outside
///   that range has no held-out fraction it could mean, and (for 1.0
///   specifically) indexes one past the end of the sorted score list rather
///   than refusing.
/// - Negatives are split into fitting/held-out families once, for every
///   label; positives are split *per label*, since two labels' positives are
///   free to live in disjoint families and a single split over the whole
///   positive set would put every one of a label's families on the same side
///   by accident. Fewer than two families — for the negatives overall, or for
///   one label's own positives — is refused, because there is nothing to hold
///   out.
/// - Every positive must carry a label the index holds. A label with zero
///   positives is refused as uncalibrated; a label whose positives all share
///   one family is refused by name, distinctly from having none at all.
/// - The threshold for a label is set from its fitting negatives' scores so
///   that at most `floor(target_fpr * n)` of them reach it, and is refused
///   outright if there are too few fitting negatives to resolve that rate, if
///   the resulting threshold is not finite (every fitting negative at or past
///   the cutoff scored `NEG_INFINITY`, which would let anything through), or
///   if no fitting positive reaches it (a threshold that catches nothing is a
///   detector that never fires, not a lenient one).
pub fn calibrate(
    index: &SemanticIndex,
    embedder: &dyn Embedder,
    positives: &[CalibrationSample],
    negatives: &[CalibrationSample],
    target_fpr: f32,
) -> Result<(SemanticCalibration, SemanticEvalEvidence), String> {
    if !(target_fpr.is_finite() && target_fpr > 0.0 && target_fpr < 1.0) {
        return Err(format!(
            "target false-positive rate {target_fpr} must be a finite number in (0.0, 1.0)"
        ));
    }

    let labels = index.labels();

    // Order matters here, not just for which `Err` wins when several checks
    // would fire: each check below is written so the *first* one to trip on a
    // given input is the one whose name matches the problem — a positive
    // labelled outside the taxonomy is a label-validity problem even when it
    // also happens to be the input's only family, and an index label with
    // zero positives is a coverage problem distinct from one whose positives
    // cannot be split into fit/held-out families.
    for p in positives {
        match &p.label {
            Some(k) if labels.contains(k) => {}
            Some(k) => {
                return Err(format!(
                    "positive sample labelled {k}, which the index does not hold"
                ));
            }
            None => return Err("every calibration positive needs a label".to_string()),
        }
    }

    let neg_family_count = negatives
        .iter()
        .map(|s| s.family.as_str())
        .collect::<BTreeSet<_>>()
        .len();
    if neg_family_count < 2 {
        return Err(
            "calibration negatives span fewer than two families; nothing to hold out".to_string(),
        );
    }
    let fit_neg_families = split_families(negatives);

    // Each label's positives are split into fit/held-out families
    // independently of every other label's, so one label's families can never
    // decide whether another label has anything left to hold out.
    let mut fit_pos_families_by_label: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for k in &labels {
        let label_positives = || {
            positives
                .iter()
                .filter(|p| p.label.as_deref() == Some(k.as_str()))
        };
        let family_count = label_positives()
            .map(|p| p.family.as_str())
            .collect::<BTreeSet<_>>()
            .len();
        if family_count == 0 {
            return Err(format!(
                "label {k} has no calibration positives; an uncalibrated label cannot ship"
            ));
        }
        if family_count < 2 {
            return Err(format!(
                "label {k} needs positives from at least two families so recall can be \
                 measured on held-out families"
            ));
        }
        fit_pos_families_by_label.insert(
            k.clone(),
            split_family_names(label_positives().map(|p| p.family.as_str())),
        );
    }

    let pos_scores: Vec<BTreeMap<String, f32>> = positives
        .iter()
        .map(|s| embed_units(embedder, &s.text).map(|units| label_scores(index, &units)))
        .collect::<Result<_, _>>()?;
    let neg_scores: Vec<BTreeMap<String, f32>> = negatives
        .iter()
        .map(|s| embed_units(embedder, &s.text).map(|units| label_scores(index, &units)))
        .collect::<Result<_, _>>()?;

    let min_fit_negatives = (1.0 / target_fpr).ceil() as usize;
    let mut margins = BTreeMap::new();
    let mut label_evidence = Vec::new();

    for k in &labels {
        let fit_pos_families = &fit_pos_families_by_label[k];

        let mut fit_neg: Vec<f32> = negatives
            .iter()
            .zip(&neg_scores)
            .filter(|(s, _)| fit_neg_families.contains(&s.family))
            .map(|(_, scores)| score_for(scores, k))
            .collect();
        // `total_cmp` rather than `partial_cmp`: a margin is a plain
        // subtraction of two cosines and never NaN, but a sort that could
        // panic on this path would need a `# Panics` section that describes a
        // case that cannot happen.
        fit_neg.sort_by(|a, b| b.total_cmp(a));

        if fit_neg.len() < min_fit_negatives {
            return Err(format!(
                "label {k} has only {} fitting negatives, need at least {min_fit_negatives} \
                 to resolve a {target_fpr} false-positive rate",
                fit_neg.len()
            ));
        }

        let cutoff = (target_fpr * fit_neg.len() as f32).floor() as usize;
        let threshold = fit_neg[cutoff] + 1e-6;
        if !threshold.is_finite() {
            return Err(format!(
                "label {k} has no finite threshold at a {target_fpr} false-positive rate; \
                 too many fitting negatives produced no embedding"
            ));
        }

        let fit_pos: Vec<f32> = positives
            .iter()
            .zip(&pos_scores)
            .filter(|(s, _)| {
                fit_pos_families.contains(&s.family) && s.label.as_deref() == Some(k.as_str())
            })
            .map(|(_, scores)| score_for(scores, k))
            .collect();

        if !fit_pos.iter().any(|&score| score >= threshold) {
            return Err(format!(
                "no threshold for {k} catches any fitting positive at a {target_fpr} \
                 false-positive rate"
            ));
        }

        let held_out_positives = tally(
            positives
                .iter()
                .zip(&pos_scores)
                .filter(|(s, _)| {
                    !fit_pos_families.contains(&s.family) && s.label.as_deref() == Some(k.as_str())
                })
                .map(|(s, scores)| (s.kind.as_str(), score_for(scores, k) >= threshold)),
        );
        let held_out_negatives = tally(
            negatives
                .iter()
                .zip(&neg_scores)
                .filter(|(s, _)| !fit_neg_families.contains(&s.family))
                .map(|(s, scores)| (s.kind.as_str(), score_for(scores, k) >= threshold)),
        );

        margins.insert(k.clone(), threshold);
        label_evidence.push(LabelEvidence {
            label: k.clone(),
            threshold,
            fit_positives: fit_pos.len(),
            fit_negatives: fit_neg.len(),
            held_out_positives,
            held_out_negatives,
        });
    }

    let calibration = SemanticCalibration {
        detector_version: SemanticCalibration::detector_version_for(&index.embedder.sha256),
        taxonomy_version: index.taxonomy_version.clone(),
        margins,
        embedder: index.embedder.clone(),
    };
    let evidence = SemanticEvalEvidence {
        embedder_sha256: index.embedder.sha256.clone(),
        target_fpr,
        labels: label_evidence,
    };
    Ok((calibration, evidence))
}

/// Fold `(kind, hit)` pairs into kind -> (hits, total).
fn tally<'a>(items: impl Iterator<Item = (&'a str, bool)>) -> BTreeMap<String, (usize, usize)> {
    let mut counts: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (kind, hit) in items {
        let entry = counts.entry(kind.to_string()).or_insert((0, 0));
        entry.1 += 1;
        if hit {
            entry.0 += 1;
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embed::tests::BagOfWords;
    use crate::semantic_index::{EmbedderRecord, SemanticIndexBuilder, label_key};
    use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};

    fn key() -> String {
        label_key(DataCategory::Internal, SensitivityLevel::Confidential)
    }

    fn sample(text: &str, family: &str, label: Option<String>, kind: &str) -> CalibrationSample {
        CalibrationSample {
            text: text.into(),
            family: family.into(),
            label,
            kind: kind.into(),
        }
    }

    fn fixture() -> (
        crate::semantic_index::SemanticIndex,
        Vec<CalibrationSample>,
        Vec<CalibrationSample>,
    ) {
        let mut b = SemanticIndexBuilder::new(
            "1.0.0",
            EmbedderRecord {
                source: "o/m/f.gguf".into(),
                sha256: "test-digest".into(),
                pooling: crate::embed::EmbeddingPooling::Last,
            },
        );
        let secrets: Vec<String> = (0..40)
            .map(|i| format!("secret merger term {i} indemnity northwind board vote"))
            .collect();
        for s in &secrets {
            b.add_document(
                &BagOfWords,
                s,
                DataCategory::Internal,
                SensitivityLevel::Confidential,
                "board",
            )
            .unwrap();
        }
        b.add_anchor(
            &BagOfWords,
            "public filing about quarterly revenue and product labels",
        )
        .unwrap();
        let idx = b.build().unwrap();
        let positives = (0..40)
            .map(|i| {
                sample(
                    &format!("northwind board vote on merger term {i} indemnity"),
                    &format!("fam{}", i % 8),
                    Some(key()),
                    if i % 2 == 0 { "rewrite" } else { "translation" },
                )
            })
            .collect();
        let negatives = (0..400)
            .map(|i| {
                sample(
                    &format!("please help me plan a birthday party number {i}"),
                    &format!("neg{}", i % 16),
                    None,
                    if i % 3 == 0 { "short" } else { "long" },
                )
            })
            .collect();
        (idx, positives, negatives)
    }

    #[test]
    fn the_threshold_meets_the_target_rate_on_fitting_negatives() {
        let (idx, pos, neg) = fixture();
        let (cal, evidence) = calibrate(&idx, &BagOfWords, &pos, &neg, 0.01).unwrap();
        assert!(cal.margins.contains_key(&key()));
        assert_eq!(cal.detector_version, "semantic:test-digest");
        let label = &evidence.labels[0];
        assert!(label.fit_negatives >= 100);
        let (fired, total): (usize, usize) = label
            .held_out_negatives
            .values()
            .fold((0, 0), |(f, t), (a, b)| (f + a, t + b));
        assert!(total > 0 && fired as f32 / total as f32 <= 0.05);
    }

    #[test]
    fn too_few_negatives_to_resolve_the_rate_is_refused() {
        let (idx, pos, neg) = fixture();
        assert!(calibrate(&idx, &BagOfWords, &pos, &neg[..50], 0.01).is_err());
    }

    #[test]
    fn a_threshold_that_catches_nothing_is_refused_not_collapsed() {
        let (idx, _, neg) = fixture();
        let unrelated: Vec<_> = (0..20)
            .map(|i| {
                sample(
                    "gardening tips for spring",
                    &format!("f{i}"),
                    Some(key()),
                    "rewrite",
                )
            })
            .collect();
        assert!(calibrate(&idx, &BagOfWords, &unrelated, &neg, 0.01).is_err());
    }

    #[test]
    fn a_label_the_index_does_not_hold_is_refused() {
        let (idx, _, neg) = fixture();
        let other = label_key(DataCategory::Financial, SensitivityLevel::Restricted);
        let pos = vec![sample("x y z", "f", Some(other), "rewrite")];
        let err = calibrate(&idx, &BagOfWords, &pos, &neg, 0.01).unwrap_err();
        assert!(
            err.contains("does not hold"),
            "expected the unknown-label rejection, got: {err}"
        );
    }

    #[test]
    fn an_index_label_without_positives_is_refused() {
        let (idx, _, neg) = fixture();
        let err = calibrate(&idx, &BagOfWords, &[], &neg, 0.01).unwrap_err();
        assert!(
            err.contains("has no calibration positives"),
            "expected the no-positives rejection, got: {err}"
        );
    }

    #[test]
    fn the_family_split_alternates_over_sorted_names() {
        let samples: Vec<_> = ["c", "a", "b", "d"]
            .iter()
            .map(|f| sample("x", f, None, "long"))
            .collect();
        let fit = split_families(&samples);
        assert_eq!(fit, BTreeSet::from(["a".to_string(), "c".to_string()]));
    }

    #[test]
    fn a_single_family_cannot_be_held_out_and_is_refused() {
        let (idx, pos, _) = fixture();
        let one: Vec<_> = (0..200)
            .map(|i| sample(&format!("party {i}"), "only", None, "long"))
            .collect();
        assert!(calibrate(&idx, &BagOfWords, &pos, &one, 0.01).is_err());
    }

    #[test]
    fn a_target_fpr_of_one_is_refused_not_a_panic() {
        let (idx, pos, neg) = fixture();
        assert!(calibrate(&idx, &BagOfWords, &pos, &neg, 1.0).is_err());
    }

    #[test]
    fn a_nan_target_fpr_is_refused() {
        let (idx, pos, neg) = fixture();
        assert!(calibrate(&idx, &BagOfWords, &pos, &neg, f32::NAN).is_err());
    }

    #[test]
    fn a_zero_target_fpr_is_refused() {
        let (idx, pos, neg) = fixture();
        assert!(calibrate(&idx, &BagOfWords, &pos, &neg, 0.0).is_err());
    }

    #[test]
    fn a_non_finite_threshold_is_refused() {
        // Every fitting negative embeds to zero units, so its score for every
        // label is `NEG_INFINITY`. A threshold picked from those scores would
        // let anything clear it — the exact failure mode "catches nothing" is
        // meant to rule out, from the other direction.
        let (idx, pos, _) = fixture();
        let empty_negatives: Vec<_> = (0..200)
            .map(|i| sample("", &format!("neg{}", i % 4), None, "empty"))
            .collect();
        assert!(calibrate(&idx, &BagOfWords, &pos, &empty_negatives, 0.01).is_err());
    }

    fn two_label_fixture() -> (
        crate::semantic_index::SemanticIndex,
        Vec<CalibrationSample>,
        Vec<CalibrationSample>,
    ) {
        let mut b = SemanticIndexBuilder::new(
            "1.0.0",
            EmbedderRecord {
                source: "o/m/f.gguf".into(),
                sha256: "test-digest".into(),
                pooling: crate::embed::EmbeddingPooling::Last,
            },
        );
        let board_secrets: Vec<String> = (0..40)
            .map(|i| format!("secret merger term {i} indemnity northwind board vote"))
            .collect();
        for s in &board_secrets {
            b.add_document(
                &BagOfWords,
                s,
                DataCategory::Internal,
                SensitivityLevel::Confidential,
                "board",
            )
            .unwrap();
        }
        let payroll_secrets: Vec<String> = (0..40)
            .map(|i| format!("payroll ledger entry {i} quarterly finance budget review"))
            .collect();
        for s in &payroll_secrets {
            b.add_document(
                &BagOfWords,
                s,
                DataCategory::Financial,
                SensitivityLevel::Restricted,
                "fin",
            )
            .unwrap();
        }
        b.add_anchor(
            &BagOfWords,
            "public filing about quarterly revenue and product labels",
        )
        .unwrap();
        let idx = b.build().unwrap();

        let key_a = key();
        let key_b = label_key(DataCategory::Financial, SensitivityLevel::Restricted);

        let positives_a = (0..20).map(|i| {
            sample(
                &format!("northwind board vote on merger term {i} indemnity"),
                if i % 2 == 0 { "a" } else { "c" },
                Some(key_a.clone()),
                if i % 2 == 0 { "rewrite" } else { "translation" },
            )
        });
        let positives_b = (0..20).map(|i| {
            sample(
                &format!("payroll ledger entry {i} quarterly finance budget review"),
                if i % 2 == 0 { "b" } else { "d" },
                Some(key_b.clone()),
                if i % 2 == 0 { "rewrite" } else { "translation" },
            )
        });
        let positives: Vec<_> = positives_a.chain(positives_b).collect();

        let negatives = (0..400)
            .map(|i| {
                sample(
                    &format!("please help me plan a birthday party number {i}"),
                    &format!("neg{}", i % 16),
                    None,
                    if i % 3 == 0 { "short" } else { "long" },
                )
            })
            .collect();

        (idx, positives, negatives)
    }

    /// Positives are split into fit/held-out families
    /// *per label*, not once over the whole positive set. Label A's positives
    /// live in families {a, c} and label B's live in {b, d}; a single split
    /// over all of them (sorted: a, b, c, d -> fit {a, c}, held-out {b, d})
    /// would put every one of label B's families on the held-out side, so B
    /// would wrongly refuse for having no fitting positives.
    #[test]
    fn positives_are_split_into_fit_and_held_out_per_label() {
        let (idx, pos, neg) = two_label_fixture();
        let (cal, evidence) = calibrate(&idx, &BagOfWords, &pos, &neg, 0.01).unwrap();

        let key_a = key();
        let key_b = label_key(DataCategory::Financial, SensitivityLevel::Restricted);
        assert!(cal.margins.contains_key(&key_a));
        assert!(cal.margins.contains_key(&key_b));

        for label_evidence in &evidence.labels {
            let total_held_out: usize = label_evidence
                .held_out_positives
                .values()
                .map(|(_, total)| total)
                .sum();
            assert!(
                total_held_out > 0,
                "label {} has no held-out positives",
                label_evidence.label
            );
        }
    }

    #[test]
    fn a_label_with_positives_from_only_one_family_is_refused_by_name() {
        let (idx, _, neg) = two_label_fixture();
        // Label B's positives all collapse onto family "b": nothing to hold
        // out for B specifically, even though label A's families are fine.
        let key_b = label_key(DataCategory::Financial, SensitivityLevel::Restricted);
        let pos: Vec<_> = (0..20)
            .map(|i| {
                sample(
                    &format!("payroll ledger entry {i} quarterly finance budget review"),
                    "b",
                    Some(key_b.clone()),
                    "rewrite",
                )
            })
            .chain((0..20).map(|i| {
                sample(
                    &format!("northwind board vote on merger term {i} indemnity"),
                    if i % 2 == 0 { "a" } else { "c" },
                    Some(key()),
                    "rewrite",
                )
            }))
            .collect();
        let err = calibrate(&idx, &BagOfWords, &pos, &neg, 0.01).unwrap_err();
        assert!(
            err.contains(&key_b) && err.contains("at least two families"),
            "expected a per-label family-count rejection naming {key_b}, got: {err}"
        );
    }
}
