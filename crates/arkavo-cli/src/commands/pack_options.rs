//! CLI flag parsing and validation for `arkavo pack index`.
//!
//! Split out from `pack.rs`, which owns dispatch and the actual build:
//! turning nineteen flags into a validated `Options` is one responsibility
//! on its own, and keeping it here is what lets `pack.rs` stay a size where
//! the build itself is still readable at a glance.

use std::path::{Path, PathBuf};

use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};

#[cfg(feature = "sentinel")]
use super::pack_semantic::SemanticOptions;
#[cfg(feature = "sentinel")]
use arkavo_fingerprint::EmbeddingPooling;

#[derive(Debug)]
pub(crate) struct Options {
    pub(crate) corpus: PathBuf,
    pub(crate) key_file: PathBuf,
    pub(crate) out: PathBuf,
    pub(crate) taxonomy: Option<PathBuf>,
    pub(crate) index_id: String,
    pub(crate) category: DataCategory,
    pub(crate) sensitivity: SensitivityLevel,
    pub(crate) family: Option<String>,
    pub(crate) boilerplate: Option<PathBuf>,
    /// `None` unless `--embedder` was given; when it was, every companion
    /// flag `SemanticOptions` needs was already confirmed present here, so
    /// nothing downstream re-validates an `Option` that cannot be `None`.
    #[cfg(feature = "sentinel")]
    pub(crate) embedder: Option<SemanticOptions>,
}

/// A `.jsonl` corpus carries its own per-row family and label; a directory
/// corpus does not, which is what makes `--family` meaningful for one and
/// nonsensical for the other.
pub(crate) fn is_jsonl_corpus(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
}

#[cfg(feature = "sentinel")]
fn parse_pooling(name: &str) -> Result<EmbeddingPooling, String> {
    match name.to_ascii_lowercase().as_str() {
        "last" => Ok(EmbeddingPooling::Last),
        "mean" => Ok(EmbeddingPooling::Mean),
        "cls" => Ok(EmbeddingPooling::Cls),
        other => Err(format!("unknown pooling '{other}'")),
    }
}

#[cfg(feature = "sentinel")]
fn parse_target_fpr(raw: &str) -> Result<f32, String> {
    raw.parse()
        .map_err(|e| format!("--target-fpr '{raw}' is not a number: {e}"))
}

pub(crate) fn parse(args: &[String]) -> Result<Options, String> {
    let mut corpus = None;
    let mut key_file = None;
    let mut out = None;
    let mut taxonomy = None;
    let mut index_id = "default".to_string();
    let mut category = DataCategory::Internal;
    let mut sensitivity = SensitivityLevel::Confidential;
    let mut family = None;
    let mut boilerplate = None;
    #[cfg(feature = "sentinel")]
    let mut embedder = None;
    #[cfg(feature = "sentinel")]
    let mut embedder_source = None;
    #[cfg(feature = "sentinel")]
    let mut pooling = None;
    #[cfg(feature = "sentinel")]
    let mut anchors = None;
    #[cfg(feature = "sentinel")]
    let mut calibrate_positives = None;
    #[cfg(feature = "sentinel")]
    let mut calibrate_negatives = None;
    #[cfg(feature = "sentinel")]
    let mut target_fpr = 0.01f32;
    #[cfg(feature = "sentinel")]
    let mut semantic_thresholds_out = None;
    #[cfg(feature = "sentinel")]
    let mut eval_evidence_out = None;

    let mut i = 0;
    while i < args.len() {
        let take = |i: usize, what: &str| -> Result<String, String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{what} requires a value"))
        };
        let path =
            |i: usize, what: &str| -> Result<PathBuf, String> { take(i, what).map(PathBuf::from) };
        match args[i].as_str() {
            "--corpus" => corpus = Some(path(i, "--corpus")?),
            "--key-file" => key_file = Some(path(i, "--key-file")?),
            "--out" => out = Some(path(i, "--out")?),
            "--taxonomy" => taxonomy = Some(path(i, "--taxonomy")?),
            "--index-id" => index_id = take(i, "--index-id")?,
            "--category" => category = parse_category(&take(i, "--category")?)?,
            "--sensitivity" => sensitivity = parse_sensitivity(&take(i, "--sensitivity")?)?,
            "--family" => family = Some(take(i, "--family")?),
            "--boilerplate" => boilerplate = Some(path(i, "--boilerplate")?),
            // The semantic pass needs llama.cpp linked in, which only happens
            // under `sentinel`. Refused right here, before any companion is
            // even looked for: a `knowledge-pack`-only build has no use for
            // them, so it should not ask for six flags it can never act on.
            #[cfg(not(feature = "sentinel"))]
            "--embedder" => {
                let _ = take(i, "--embedder")?;
                return Err("this build was compiled without the sentinel feature".to_string());
            }
            #[cfg(feature = "sentinel")]
            "--embedder" => embedder = Some(path(i, "--embedder")?),
            #[cfg(feature = "sentinel")]
            "--embedder-source" => embedder_source = Some(take(i, "--embedder-source")?),
            #[cfg(feature = "sentinel")]
            "--pooling" => pooling = Some(parse_pooling(&take(i, "--pooling")?)?),
            #[cfg(feature = "sentinel")]
            "--anchors" => anchors = Some(path(i, "--anchors")?),
            #[cfg(feature = "sentinel")]
            "--calibrate-positives" => {
                calibrate_positives = Some(path(i, "--calibrate-positives")?);
            }
            #[cfg(feature = "sentinel")]
            "--calibrate-negatives" => {
                calibrate_negatives = Some(path(i, "--calibrate-negatives")?);
            }
            #[cfg(feature = "sentinel")]
            "--target-fpr" => target_fpr = parse_target_fpr(&take(i, "--target-fpr")?)?,
            #[cfg(feature = "sentinel")]
            "--semantic-thresholds-out" => {
                semantic_thresholds_out = Some(path(i, "--semantic-thresholds-out")?);
            }
            #[cfg(feature = "sentinel")]
            "--eval-evidence-out" => eval_evidence_out = Some(path(i, "--eval-evidence-out")?),
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 2;
    }

    let corpus = corpus.ok_or("--corpus is required")?;
    if family.is_some() && is_jsonl_corpus(&corpus) {
        return Err(
            "--family cannot be used with a JSONL corpus; its rows carry their own family"
                .to_string(),
        );
    }

    #[cfg(feature = "sentinel")]
    let embedder = embedder
        .map(|embedder| {
            Ok::<_, String>(SemanticOptions {
                embedder,
                embedder_source: embedder_source.ok_or("--embedder requires --embedder-source")?,
                pooling,
                anchors: anchors.ok_or("--embedder requires --anchors")?,
                calibrate_positives: calibrate_positives
                    .ok_or("--embedder requires --calibrate-positives")?,
                calibrate_negatives: calibrate_negatives
                    .ok_or("--embedder requires --calibrate-negatives")?,
                target_fpr,
                semantic_thresholds_out: semantic_thresholds_out
                    .ok_or("--embedder requires --semantic-thresholds-out")?,
                eval_evidence_out: eval_evidence_out
                    .ok_or("--embedder requires --eval-evidence-out")?,
            })
        })
        .transpose()?;

    Ok(Options {
        corpus,
        key_file: key_file.ok_or("--key-file is required")?,
        out: out.ok_or("--out is required")?,
        taxonomy,
        index_id,
        category,
        sensitivity,
        family,
        boilerplate,
        #[cfg(feature = "sentinel")]
        embedder,
    })
}

fn parse_category(name: &str) -> Result<DataCategory, String> {
    match name.to_ascii_lowercase().as_str() {
        "pii" => Ok(DataCategory::Pii),
        "credentials" => Ok(DataCategory::Credentials),
        "financial" => Ok(DataCategory::Financial),
        "healthcare" => Ok(DataCategory::Healthcare),
        "internal" => Ok(DataCategory::Internal),
        "public" => Ok(DataCategory::Public),
        other => Err(format!("unknown category '{other}'")),
    }
}

fn parse_sensitivity(name: &str) -> Result<SensitivityLevel, String> {
    match name.to_ascii_lowercase().as_str() {
        "public" => Ok(SensitivityLevel::Public),
        "internal" => Ok(SensitivityLevel::Internal),
        "confidential" => Ok(SensitivityLevel::Confidential),
        "restricted" => Ok(SensitivityLevel::Restricted),
        other => Err(format!("unknown sensitivity '{other}'")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn category_and_sensitivity_names_are_case_insensitive() {
        assert_eq!(parse_category("PII").unwrap(), DataCategory::Pii);
        assert_eq!(
            parse_sensitivity("Restricted").unwrap(),
            SensitivityLevel::Restricted
        );
    }

    #[test]
    fn an_unknown_label_is_refused_rather_than_defaulted() {
        // Defaulting would silently classify a corpus at the wrong level.
        assert!(parse_category("nonsense").is_err());
        assert!(parse_sensitivity("secret-ish").is_err());
    }

    #[test]
    fn the_required_options_are_required() {
        let err = parse(&["--corpus".into(), "/tmp/x".into()]).unwrap_err();

        assert!(err.contains("--key-file"), "{err}");
    }

    #[test]
    fn there_is_no_option_that_builds_without_a_key() {
        // KP-009: an unavailable key fails the build. A flag that skipped
        // keying would reintroduce the dictionary the design exists to avoid.
        let err = parse(&["--no-key".into(), "x".into()]).unwrap_err();

        assert!(err.contains("unknown option"), "{err}");
    }

    #[test]
    fn family_with_a_jsonl_corpus_is_refused() {
        let err = parse(&args(&[
            "--corpus",
            "c.jsonl",
            "--key-file",
            "k",
            "--out",
            "o",
            "--family",
            "x",
        ]))
        .unwrap_err();
        assert!(err.contains("family"), "{err}");
    }

    #[cfg(feature = "sentinel")]
    #[test]
    fn embedder_without_its_companions_is_refused() {
        let err = parse(&args(&[
            "--corpus",
            "c",
            "--key-file",
            "k",
            "--out",
            "o",
            "--embedder",
            "m.gguf",
        ]))
        .unwrap_err();
        assert!(err.contains("--embedder-source"));
    }

    #[cfg(not(feature = "sentinel"))]
    #[test]
    fn embedder_is_refused_without_the_sentinel_feature() {
        let err = parse(&args(&[
            "--corpus",
            "c",
            "--key-file",
            "k",
            "--out",
            "o",
            "--embedder",
            "m.gguf",
        ]))
        .unwrap_err();
        assert!(err.contains("sentinel feature"), "{err}");
    }
}
