//! End-to-end `arkavo pack index` semantic pass, against a real embedder.
//!
//! Skipped unless `ARKAVO_TEST_EMBED_MODEL` points at a `.gguf` file, so an
//! ordinary `cargo test` run stays hermetic (the repo's `ARKAVO_TEST_MODEL`
//! convention, see `crates/arkavo-gguf-tdf/tests/integration.rs`).
//!
//! Run with:
//!   ARKAVO_TEST_EMBED_MODEL=models/tinystories/stories15M.gguf \
//!     cargo test -p arkavo-cli --features sentinel --test pack_semantic_build
//!
//! `stories15M` declares no pooling type in its GGUF metadata, so every build
//! here passes `--pooling last` explicitly; Qwen3-Embedding declares `last`
//! pooling too, so the same flag works unmodified against either model named
//! by the env var.
//!
//! Calibrating against paraphrased positives is not guaranteed to reach
//! recall on a tiny embedder: whether it does depends on how well that model
//! separates the synthetic corpus from the synthetic negatives, which is not
//! something this test controls. Both outcomes are asserted below, not
//! skipped: either the build succeeds and the output files are checked, or
//! `calibrate` refuses by naming the unreachable threshold. A second case
//! with positives that repeat the corpus text verbatim is calibrated
//! separately, and that one must succeed unconditionally, since the
//! embedder's cosine of a vector against itself is as strong a signal as any
//! embedder can produce.

#![cfg(feature = "sentinel")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use arkavo_cli::commands::pack;
use arkavo_cli::sentinel_embedder::{LlamaEmbedder, sha256_file};
use arkavo_cli::sentinel_wiring::SentinelRuntime;
use arkavo_crypto::AgentKeypair;
use arkavo_fingerprint::{
    Embedder, EmbeddingPooling, IndexKey, SemanticCalibration, SemanticEvalEvidence, label_key,
};
use arkavo_gguf_tdf::PreResolvedKey;
use arkavo_knowledge_pack::{PackIndexes, verify_pack};
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};

fn model_path() -> Option<PathBuf> {
    let path = std::env::var_os("ARKAVO_TEST_EMBED_MODEL")?;
    let path = PathBuf::from(path);
    if path.exists() {
        Some(path)
    } else {
        eprintln!(
            "ARKAVO_TEST_EMBED_MODEL points at a missing file: {}",
            path.display()
        );
        None
    }
}

fn label() -> String {
    label_key(DataCategory::Internal, SensitivityLevel::Confidential)
}

const CORPUS: [(&str, &str); 3] = [
    (
        "the northwind acquisition closes in march with a hidden indemnity clause",
        "board",
    ),
    (
        "quarterly payroll ledger entries for the finance team show a large bonus pool",
        "finance",
    ),
    (
        "the new product roadmap for project falcon remains unannounced until launch",
        "product",
    ),
];

fn write_jsonl(path: &Path, rows: &[serde_json::Value]) {
    use std::fmt::Write as _;
    let mut body = String::new();
    for row in rows {
        writeln!(body, "{row}").unwrap();
    }
    std::fs::write(path, body).unwrap();
}

/// A 32-byte tenant key, generated from process/time entropy rather than a
/// literal so no credential-shaped byte string lands in the fixture source.
fn write_key(dir: &Path) -> PathBuf {
    let mut state = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .subsec_nanos() as u64
        ^ (std::process::id() as u64) << 32;
    let mut bytes = [0u8; 32];
    for b in &mut bytes {
        state = state
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1);
        *b = (state >> 33) as u8;
    }
    let path = dir.join("key.bin");
    std::fs::write(&path, bytes).unwrap();
    path
}

fn write_corpus(path: &Path) {
    let rows: Vec<_> = CORPUS
        .iter()
        .map(|(text, family)| serde_json::json!({"text": text, "family": family, "label": label()}))
        .collect();
    write_jsonl(path, &rows);
}

fn write_anchors(path: &Path) {
    let anchors = [
        "public filing about quarterly revenue and product labels for retail customers",
        "oxycodone prescribing information warns of addiction abuse and misuse",
        "the weather forecast calls for rain later this week across the region",
        "a recipe for sourdough bread requires flour water salt and time",
        "general information about how public libraries organize their catalogs",
    ];
    let rows: Vec<_> = anchors
        .iter()
        .map(|text| serde_json::json!({"text": text, "family": "anchor", "source": "public-web"}))
        .collect();
    write_jsonl(path, &rows);
}

/// 300 negatives across 12 families: enough fitting negatives to resolve the
/// default 1% target false-positive rate (needs >= 100), with room to spare.
fn write_negatives(path: &Path) {
    let rows: Vec<_> = (0..300)
        .map(|i| {
            serde_json::json!({
                "text": format!("please help me plan a birthday party number {i}"),
                "family": format!("neg{}", i % 12),
                "kind": if i % 3 == 0 { "short" } else { "long" },
            })
        })
        .collect();
    write_jsonl(path, &rows);
}

/// 20 positives across 4 families, worded differently from the corpus text —
/// whether a tiny embedder judges these close enough to the corpus to clear
/// a calibrated threshold is exactly the question this fixture is for.
fn write_paraphrase_positives(path: &Path) {
    let paraphrases = [
        "there is a hidden indemnity clause tied to the northwind deal closing next month",
        "the finance department's payroll ledger reveals a substantial bonus fund this quarter",
        "an unreleased roadmap for the falcon project is being kept under wraps before launch",
    ];
    let rows: Vec<_> = (0..20)
        .map(|i| {
            serde_json::json!({
                "text": format!("{} ({i})", paraphrases[i % paraphrases.len()]),
                "family": format!("pos{}", i % 4),
                "label": label(),
                "kind": if i % 2 == 0 { "rewrite" } else { "translation" },
            })
        })
        .collect();
    write_jsonl(path, &rows);
}

/// 20 positives across 4 families that repeat corpus sentences verbatim, so
/// recall is reachable by any embedder: a vector's cosine against itself is
/// the strongest score that embedder can produce.
fn write_verbatim_positives(path: &Path) {
    let rows: Vec<_> = (0..20)
        .map(|i| {
            let (text, _family) = CORPUS[i % CORPUS.len()];
            serde_json::json!({
                "text": text,
                "family": format!("pos{}", i % 4),
                "label": label(),
                "kind": "verbatim",
            })
        })
        .collect();
    write_jsonl(path, &rows);
}

struct Outputs {
    _dir: tempfile::TempDir,
    index_out: PathBuf,
    thresholds_out: PathBuf,
    evidence_out: PathBuf,
}

fn run_build(model: &Path, positives_path: &Path) -> (Result<(), String>, Outputs) {
    let dir = tempfile::tempdir().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    let anchors = dir.path().join("anchors.jsonl");
    let negatives = dir.path().join("negatives.jsonl");
    write_corpus(&corpus);
    write_anchors(&anchors);
    write_negatives(&negatives);
    let key = write_key(dir.path());
    let index_out = dir.path().join("index.json");
    let thresholds_out = dir.path().join("thresholds.json");
    let evidence_out = dir.path().join("evidence.json");

    let args: Vec<String> = [
        "index",
        "--corpus",
        corpus.to_str().unwrap(),
        "--key-file",
        key.to_str().unwrap(),
        "--out",
        index_out.to_str().unwrap(),
        "--category",
        "internal",
        "--sensitivity",
        "confidential",
        "--embedder",
        model.to_str().unwrap(),
        "--embedder-source",
        "test/embed/model.gguf",
        "--pooling",
        "last",
        "--anchors",
        anchors.to_str().unwrap(),
        "--calibrate-positives",
        positives_path.to_str().unwrap(),
        "--calibrate-negatives",
        negatives.to_str().unwrap(),
        "--semantic-thresholds-out",
        thresholds_out.to_str().unwrap(),
        "--eval-evidence-out",
        evidence_out.to_str().unwrap(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let result = pack::execute(&args);
    (
        result,
        Outputs {
            _dir: dir,
            index_out,
            thresholds_out,
            evidence_out,
        },
    )
}

fn assert_outputs_parse(model: &Path, outputs: &Outputs) {
    let index_json = std::fs::read_to_string(&outputs.index_out).unwrap();
    let indexes: PackIndexes = serde_json::from_str(&index_json).unwrap();
    assert!(
        indexes.semantic.is_some(),
        "the index has no semantic section"
    );

    let thresholds_json = std::fs::read_to_string(&outputs.thresholds_out).unwrap();
    let calibration: SemanticCalibration = serde_json::from_str(&thresholds_json).unwrap();
    assert_eq!(calibration.embedder.sha256, sha256_file(model).unwrap());

    let evidence_json = std::fs::read_to_string(&outputs.evidence_out).unwrap();
    let _evidence: SemanticEvalEvidence = serde_json::from_str(&evidence_json).unwrap();
}

#[test]
fn paraphrase_positives_either_calibrate_or_name_the_unreachable_threshold() {
    let Some(model) = model_path() else {
        eprintln!("skipping: set ARKAVO_TEST_EMBED_MODEL");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let positives = dir.path().join("positives.jsonl");
    write_paraphrase_positives(&positives);

    let (result, outputs) = run_build(&model, &positives);
    match result {
        Ok(()) => assert_outputs_parse(&model, &outputs),
        Err(err) => assert!(
            err.contains("threshold") || err.contains("catches") || err.contains("fitting"),
            "expected an unreachable-threshold style error from `calibrate`, got: {err}"
        ),
    }
}

#[test]
fn verbatim_positives_calibrate_successfully() {
    let Some(model) = model_path() else {
        eprintln!("skipping: set ARKAVO_TEST_EMBED_MODEL");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let positives = dir.path().join("positives.jsonl");
    write_verbatim_positives(&positives);

    let (result, outputs) = run_build(&model, &positives);
    if let Err(e) = result {
        panic!("verbatim positives repeat the indexed text; calibration must reach recall: {e}");
    }
    assert_outputs_parse(&model, &outputs);
}

/// The full CLI path a real deployment takes: `pack index` (with a semantic
/// section) → `pack wrap` (local key custody) → `pack seal` → `verify_pack` →
/// `SentinelRuntime::from_pack` with the recorded key and a real embedder.
///
/// Every earlier test in this file calls `pack::execute(["index", ..])`
/// directly; nothing before this task turned that plaintext index into
/// something `pack seal` would accept, so this is the first test that walks
/// the whole producing side of `chat --pack` end to end. Verbatim positives
/// are used (not paraphrases) so calibration succeeds unconditionally — this
/// test is about the wrap/seal/load wiring, not about whether a tiny embedder
/// generalizes.
#[test]
fn pack_index_wrap_seal_and_load_round_trip_through_sentinel_runtime() {
    let Some(model) = model_path() else {
        eprintln!("skipping: set ARKAVO_TEST_EMBED_MODEL");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let corpus = dir.path().join("corpus.jsonl");
    let anchors = dir.path().join("anchors.jsonl");
    let negatives = dir.path().join("negatives.jsonl");
    let positives = dir.path().join("positives.jsonl");
    write_corpus(&corpus);
    write_anchors(&anchors);
    write_negatives(&negatives);
    write_verbatim_positives(&positives);
    let tenant_key = write_key(dir.path());

    let index_out = dir.path().join("index.json");
    let thresholds_out = dir.path().join("thresholds.json");
    let evidence_out = dir.path().join("evidence.json");
    let index_args: Vec<String> = [
        "index",
        "--corpus",
        corpus.to_str().unwrap(),
        "--key-file",
        tenant_key.to_str().unwrap(),
        "--out",
        index_out.to_str().unwrap(),
        "--category",
        "internal",
        "--sensitivity",
        "confidential",
        "--embedder",
        model.to_str().unwrap(),
        "--embedder-source",
        "test/embed/model.gguf",
        "--pooling",
        "last",
        "--anchors",
        anchors.to_str().unwrap(),
        "--calibrate-positives",
        positives.to_str().unwrap(),
        "--calibrate-negatives",
        negatives.to_str().unwrap(),
        "--semantic-thresholds-out",
        thresholds_out.to_str().unwrap(),
        "--eval-evidence-out",
        evidence_out.to_str().unwrap(),
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    pack::execute(&index_args).expect("pack index");

    let wrapped_out = dir.path().join("index.json.tdf");
    let payload_key_out = dir.path().join("payload.key");
    pack::execute(&[
        "wrap".to_string(),
        "--in".to_string(),
        index_out.to_str().unwrap().to_string(),
        "--out".to_string(),
        wrapped_out.to_str().unwrap().to_string(),
        "--payload-key-out".to_string(),
        payload_key_out.to_str().unwrap().to_string(),
    ])
    .expect("pack wrap");

    let signing_key = AgentKeypair::generate();
    let signing_key_path = dir.path().join("signing.key");
    std::fs::write(&signing_key_path, signing_key.to_bytes()).unwrap();
    let pack_dir = dir.path().join("pack");
    pack::execute(&[
        "seal".to_string(),
        "--out".to_string(),
        pack_dir.to_str().unwrap().to_string(),
        "--signing-key".to_string(),
        signing_key_path.to_str().unwrap().to_string(),
        "--pack-id".to_string(),
        "semantic-e2e".to_string(),
        "--component".to_string(),
        format!("{}:index:confidential", wrapped_out.to_str().unwrap()),
        "--thresholds".to_string(),
        format!("semantic:{}", thresholds_out.to_str().unwrap()),
    ])
    .expect("pack seal");

    let verified = verify_pack(&pack_dir, Some(&signing_key.public_key())).expect("verify_pack");

    let secret = std::fs::read(&tenant_key).unwrap();
    let index_key = Arc::new(IndexKey::derive(&secret, "default").expect("derive tenant key"));
    let payload_key_bytes = std::fs::read(&payload_key_out).unwrap();
    assert_eq!(
        payload_key_bytes.len(),
        32,
        "the payload key must be raw bytes"
    );
    let mut payload_key = [0u8; 32];
    payload_key.copy_from_slice(&payload_key_bytes);

    let embedder: Arc<dyn Embedder> =
        Arc::new(LlamaEmbedder::load(&model, EmbeddingPooling::Last).expect("load embedder"));

    let runtime = SentinelRuntime::from_pack(
        &verified,
        Some(&index_key),
        &PreResolvedKey::new(payload_key),
        Some(embedder),
    )
    .expect("provision the sentinel runtime from the sealed pack");

    assert_eq!(runtime.ceiling, SensitivityLevel::Confidential);
    assert!(
        runtime.cascade().tier_names().contains(&"semantic"),
        "{:?}",
        runtime.cascade().tier_names()
    );
    // The manifest carried only a semantic thresholds table — no sentinel
    // table was sealed — so the sentinel calibration must read back absent
    // rather than fabricated.
    assert!(runtime.calibration.is_none());
}
