//! JSONL corpus/calibration reading and the semantic pass for
//! `arkavo pack index` (Task 10, consumes Tasks 3, 4, 5, 9).
//!
//! `pack.rs` owns the CLI surface; everything here is either pure parsing
//! (so it has no opinion about where its input came from) or the one place
//! that actually drives an `Embedder`, which is why this whole file is
//! `sentinel`-gated: `LlamaEmbedder` needs llama.cpp linked in, and
//! `arkavo-fingerprint` itself stays free of that dependency.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use arkavo_fingerprint::{
    CalibrationSample, EmbedderRecord, EmbeddingPooling, SemanticCalibration, SemanticEvalEvidence,
    SemanticIndex, SemanticIndexBuilder, calibrate, normalize,
};
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use arkavo_protocol::taxonomy::TaxonomyMap;
use serde::Deserialize;

use crate::sentinel_embedder::{LlamaEmbedder, sha256_file};

/// One corpus document ready for the semantic index: text plus the family id
/// its own JSONL row (or the corpus-wide `--family` flag, for a directory
/// corpus) recorded it under.
#[derive(Debug)]
pub struct CorpusDoc {
    pub text: String,
    pub family: String,
}

#[derive(Deserialize)]
struct CorpusRow {
    text: String,
    family: String,
    #[serde(default)]
    label: Option<String>,
}

/// Read a JSONL corpus, keeping only rows labelled `label` (a `label_key`).
///
/// Every row must carry a label: an unlabelled row is refused by line number
/// rather than silently dropped, and a row filed under a different label is
/// skipped and counted rather than mixed into this label's documents.
pub fn read_corpus_jsonl(path: &Path, label: &str) -> Result<(Vec<CorpusDoc>, usize), String> {
    let content = fs::read_to_string(path)
        .map_err(|e| format!("cannot read corpus {}: {e}", path.display()))?;
    let mut docs = Vec::new();
    let mut skipped = 0usize;
    for (line_no, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: CorpusRow = serde_json::from_str(line)
            .map_err(|e| format!("{}: line {}: {e}", path.display(), line_no + 1))?;
        let Some(row_label) = row.label else {
            return Err(format!(
                "{}: line {} has no label; every corpus row must carry the label_key it \
                 was classified under",
                path.display(),
                line_no + 1
            ));
        };
        if row_label != label {
            skipped += 1;
            continue;
        }
        docs.push(CorpusDoc {
            text: row.text,
            family: row.family,
        });
    }
    Ok((docs, skipped))
}

#[derive(Deserialize)]
struct SampleRow {
    text: String,
    family: String,
    #[serde(default)]
    label: Option<String>,
    kind: String,
}

/// Read calibration positives or negatives: one JSON object per line, each
/// carrying the family the held-out-by-family split needs and, for a
/// positive, the label being calibrated.
pub fn read_samples(path: &Path) -> Result<Vec<CalibrationSample>, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut out = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: SampleRow = serde_json::from_str(line)
            .map_err(|e| format!("{}: line {}: {e}", path.display(), line_no + 1))?;
        out.push(CalibrationSample {
            text: row.text,
            family: row.family,
            label: row.label,
            kind: row.kind,
        });
    }
    Ok(out)
}

#[derive(Deserialize)]
struct AnchorRow {
    text: String,
}

/// Anchor rows are `{text, family, source}`; only the text embeds, but the
/// other two fields stay in the file as provenance for whoever curates it.
fn read_anchor_texts(path: &Path) -> Result<Vec<String>, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut out = Vec::new();
    for (line_no, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let row: AnchorRow = serde_json::from_str(line)
            .map_err(|e| format!("{}: line {}: {e}", path.display(), line_no + 1))?;
        out.push(row.text);
    }
    Ok(out)
}

/// An anchor that is also a negative would let calibration fit a threshold
/// against the very text the anchor exists to subtract, so it is refused
/// before either side is embedded.
pub fn refuse_overlap(anchors: &[String], negatives: &[CalibrationSample]) -> Result<(), String> {
    let anchor_norms: BTreeSet<String> = anchors.iter().map(|a| normalize(a)).collect();
    for negative in negatives {
        let normalized = normalize(&negative.text);
        if anchor_norms.contains(&normalized) {
            return Err(format!(
                "negative sample duplicates an anchor after normalization: {normalized:?}"
            ));
        }
    }
    Ok(())
}

/// Inputs to the semantic pass that come from the embedder-related CLI flags.
pub struct SemanticOptions {
    pub embedder: PathBuf,
    pub embedder_source: String,
    pub pooling: Option<EmbeddingPooling>,
    pub anchors: PathBuf,
    pub calibrate_positives: PathBuf,
    pub calibrate_negatives: PathBuf,
    pub target_fpr: f32,
    pub semantic_thresholds_out: PathBuf,
    pub eval_evidence_out: PathBuf,
}

pub struct SemanticBuild {
    pub index: SemanticIndex,
    pub calibration: SemanticCalibration,
    pub evidence: SemanticEvalEvidence,
}

/// Build and calibrate the semantic section of a pack index.
pub fn build_semantic(
    options: &SemanticOptions,
    documents: &[CorpusDoc],
    category: DataCategory,
    sensitivity: SensitivityLevel,
    taxonomy: &TaxonomyMap,
) -> Result<SemanticBuild, String> {
    if taxonomy.policy_for(category).is_none() {
        return Err(format!(
            "label is not defined by taxonomy {}",
            taxonomy.version()
        ));
    }

    let pooling = LlamaEmbedder::resolve_pooling(&options.embedder, options.pooling)?;
    let embedder = LlamaEmbedder::load(&options.embedder, pooling)?;
    let record = EmbedderRecord {
        source: options.embedder_source.clone(),
        sha256: sha256_file(&options.embedder)?,
        pooling,
    };

    let mut builder = SemanticIndexBuilder::new(taxonomy.version(), record);
    for doc in documents {
        builder.add_document(&embedder, &doc.text, category, sensitivity, &doc.family)?;
    }

    let anchor_texts = read_anchor_texts(&options.anchors)?;
    for anchor in &anchor_texts {
        builder.add_anchor(&embedder, anchor)?;
    }

    let positives = read_samples(&options.calibrate_positives)?;
    let negatives = read_samples(&options.calibrate_negatives)?;
    refuse_overlap(&anchor_texts, &negatives)?;

    let index = builder.build()?;
    let (calibration, evidence) = calibrate(
        &index,
        &embedder,
        &positives,
        &negatives,
        options.target_fpr,
    )?;

    Ok(SemanticBuild {
        index,
        calibration,
        evidence,
    })
}

/// Build, calibrate, write both output files, and print the evaluation
/// summary, in one call.
///
/// `pack.rs` only needs the resulting index to assemble `PackIndexes`, so
/// this is what it calls rather than the three steps separately.
pub fn run(
    options: &SemanticOptions,
    documents: &[CorpusDoc],
    category: DataCategory,
    sensitivity: SensitivityLevel,
    taxonomy: &TaxonomyMap,
) -> Result<SemanticIndex, String> {
    let build = build_semantic(options, documents, category, sensitivity, taxonomy)?;
    write_calibration(&build.calibration, &options.semantic_thresholds_out)?;
    write_evidence(&build.evidence, &options.eval_evidence_out)?;
    print_evaluation(&build.evidence);
    Ok(build.index)
}

/// Write `SemanticCalibration` as the pack's semantic thresholds file.
pub fn write_calibration(calibration: &SemanticCalibration, path: &Path) -> Result<(), String> {
    let encoded = serde_json::to_vec_pretty(calibration)
        .map_err(|e| format!("cannot serialize the semantic calibration: {e}"))?;
    fs::write(path, encoded).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Write `SemanticEvalEvidence` as the pack's evaluation evidence file.
pub fn write_evidence(evidence: &SemanticEvalEvidence, path: &Path) -> Result<(), String> {
    let encoded = serde_json::to_vec_pretty(evidence)
        .map_err(|e| format!("cannot serialize the evaluation evidence: {e}"))?;
    fs::write(path, encoded).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

/// Print held-out recall and false-positive rate per label, broken down by
/// kind, so an operator sees the calibration's cost before trusting it.
pub fn print_evaluation(evidence: &SemanticEvalEvidence) {
    for label in &evidence.labels {
        println!(
            "Semantic label {}: threshold {:.4}",
            label.label, label.threshold
        );
        for (kind, (caught, total)) in &label.held_out_positives {
            let recall = if *total > 0 {
                *caught as f32 / *total as f32
            } else {
                0.0
            };
            println!(
                "  recall [{kind}]: {caught}/{total} ({:.1}%)",
                recall * 100.0
            );
        }
        for (kind, (fired, total)) in &label.held_out_negatives {
            let fpr = if *total > 0 {
                *fired as f32 / *total as f32
            } else {
                0.0
            };
            println!("  fpr    [{kind}]: {fired}/{total} ({:.2}%)", fpr * 100.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &tempfile::TempDir, name: &str, body: &str) -> std::path::PathBuf {
        let p = dir.path().join(name);
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn an_unlabelled_corpus_row_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(&dir, "c.jsonl", "{\"text\":\"a\",\"family\":\"f\"}\n");
        assert!(
            read_corpus_jsonl(&p, "internal:confidential")
                .unwrap_err()
                .contains("line 1")
        );
    }

    #[test]
    fn rows_under_another_label_are_skipped_and_counted() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(
            &dir,
            "c.jsonl",
            concat!(
                "{\"text\":\"secret\",\"family\":\"f\",\"label\":\"internal:confidential\"}\n",
                "{\"text\":\"public\",\"family\":\"g\",\"label\":\"public:public\"}\n"
            ),
        );
        let (docs, skipped) = read_corpus_jsonl(&p, "internal:confidential").unwrap();
        assert_eq!((docs.len(), skipped), (1, 1));
        assert_eq!(docs[0].family, "f");
    }

    #[test]
    fn anchors_that_are_also_negatives_are_refused() {
        let neg = vec![CalibrationSample {
            text: "Public Filing  Text".into(),
            family: "n".into(),
            label: None,
            kind: "long".into(),
        }];
        assert!(refuse_overlap(&["public filing text".into()], &neg).is_err());
        assert!(refuse_overlap(&["something else".into()], &neg).is_ok());
    }
}
