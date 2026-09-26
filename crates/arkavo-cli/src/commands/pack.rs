//! `arkavo pack index` — build a keyed reference index from a corpus (KP-009).
//!
//! Build-time, not runtime: this reads plaintext corpus material, so it runs
//! where that material already lives and produces something that no longer
//! contains it. What comes out is keyed digests plus labels, wrapped under the
//! classification of the most sensitive thing that went in.
//!
//! The tenant key arrives as a file. KAS-backed provisioning is Phase 5's pack
//! tooling; until then a `--key-file` is the honest interface, and
//! `MIN_SECRET_BYTES` is what stops it being a weak one. There is deliberately
//! no flag that builds without a key: KP-009's edge case is that an unavailable
//! key fails the build rather than falling back to unkeyed hashes, and an
//! unkeyed index is the dictionary the whole design exists to avoid.

use std::fs;
use std::path::{Path, PathBuf};

use arkavo_fingerprint::{
    EmbeddingPooling, EntryMeta, IndexKey, NearDuplicateIndex, ReferenceIndex,
};
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use arkavo_protocol::taxonomy::TaxonomyMap;

#[cfg(feature = "sentinel")]
use super::pack_semantic;

/// Files read as corpus material. Everything else is skipped rather than
/// guessed at: an index built from a binary's bytes is noise that costs
/// lookups.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "rst", "csv", "json", "yaml", "yml", "toml", "rs", "py", "go", "ts", "js", "java",
    "sql", "html", "xml",
];

pub fn execute(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("index") => build_index(&args[1..]),
        Some("seal") => super::pack_seal::run(&args[1..]),
        Some("verify") => super::pack_seal::verify(&args[1..]),
        Some("anchor") => super::pack_seal::anchor(&args[1..]),
        Some("help" | "--help" | "-h") | None => {
            print_help();
            Ok(())
        }
        Some(other) => Err(format!(
            "unknown pack subcommand '{other}'. Try `arkavo pack help`."
        )),
    }
}

fn print_help() {
    println!("Build and open sealed knowledge packs.\n");
    println!("Usage:");
    println!("  arkavo pack index --corpus <DIR> --key-file <PATH> --out <PATH> [options]");
    println!(
        "  arkavo pack seal --out <DIR> --signing-key <PATH> --pack-id <ID> [--component ...]"
    );
    println!("  arkavo pack verify --pack <DIR> --anchor <PATH>");
    println!("  arkavo pack anchor --signing-key <PATH> --out <PATH>\n");
    println!("Options:");
    println!(
        "  --corpus <DIR|FILE.jsonl>  Corpus dir, or JSONL rows {{text, family, label}} (family must never be corpus-derived text: it is stored in the sealed index and appears in audit evidence)"
    );
    println!("  --key-file <PATH>     Tenant index key material (>= 16 bytes)");
    println!("  --out <PATH>          Where to write the index");
    println!("  --taxonomy <PATH>     Taxonomy map (default: the embedded v1 map)");
    println!("  --index-id <NAME>     Separates indices under one tenant key");
    println!("  --category <NAME>     Category for corpus documents (default: internal)");
    println!("  --sensitivity <NAME>  Sensitivity for corpus documents (default: confidential)");
    println!(
        "  --family <NAME>       Source family recorded on matches (never corpus-derived text; refused with a JSONL corpus, whose rows carry their own family)"
    );
    println!("  --boilerplate <DIR>   Directory of material to suppress");
    println!(
        "  --embedder <GGUF>     Embedder model; builds the semantic tier (needs --embedder-source, --anchors, --calibrate-positives, --calibrate-negatives, --semantic-thresholds-out, --eval-evidence-out)"
    );
    println!("  --embedder-source <owner/repo/file>  Where a node fetches the embedder");
    println!("  --pooling <last|mean|cls>  Pooling, when the GGUF declares none");
    println!("  --anchors <PATH>      JSONL public anchors {{text, family, source}}");
    println!("  --calibrate-positives <PATH>  JSONL labelled positives for calibration");
    println!("  --calibrate-negatives <PATH>  JSONL negatives for calibration");
    println!("  --target-fpr <F>      Target false-positive rate (default: 0.01)");
    println!("  --semantic-thresholds-out <PATH>  Where to write the semantic calibration");
    println!("  --eval-evidence-out <PATH>  Where to write the semantic evaluation evidence");
    println!("\nSeal options:");
    println!("  --out <DIR>           Where to write the pack");
    println!("  --signing-key <PATH>  Organization signing key (32 raw bytes)");
    println!("  --pack-id <ID>        Identity of the pack being built");
    println!("  --taxonomy-version <V>  Taxonomy map version the pack was derived against");
    println!("  --tokenizer <NAME>    Tokenizer identity");
    println!(
        "  --thresholds [<TIER>:]<PATH>  Calibration table JSON for a tier (sentinel|semantic;"
    );
    println!("                        default sentinel), repeatable; one sentinel table is bound");
    println!("                        bare, more than one tier as an object keyed by tier");
    println!("  --component <PATH>:<ROLE>[:<CEILING>]  A component and its role");
    println!("  --eval-evidence <PATH>  Evaluation evidence file bound into the pack and digested");
    println!("  --parent <ID>:<DIGEST>  Parent pack lineage (default: root)");
    println!("\nVerify options:");
    println!("  --pack <DIR>          Pack directory to verify");
    println!("  --anchor <PATH>       Organization anchor public key (32 raw bytes)");
}

#[derive(Debug)]
struct Options {
    corpus: PathBuf,
    key_file: PathBuf,
    out: PathBuf,
    taxonomy: Option<PathBuf>,
    index_id: String,
    category: DataCategory,
    sensitivity: SensitivityLevel,
    family: Option<String>,
    boilerplate: Option<PathBuf>,
    /// `None` unless `--embedder` was given; when it was, every companion
    /// flag it requires was already checked present, so this holds them
    /// unwrapped rather than as `Option`s a second consumer has to re-check.
    embedder: Option<EmbedderFlags>,
}

#[derive(Debug)]
struct EmbedderFlags {
    embedder: PathBuf,
    embedder_source: String,
    pooling: Option<EmbeddingPooling>,
    anchors: PathBuf,
    calibrate_positives: PathBuf,
    calibrate_negatives: PathBuf,
    target_fpr: f32,
    semantic_thresholds_out: PathBuf,
    eval_evidence_out: PathBuf,
}

/// A `.jsonl` corpus carries its own per-row family and label; a directory
/// corpus does not, which is what makes `--family` meaningful for one and
/// nonsensical for the other.
fn is_jsonl_corpus(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
}

fn parse_pooling(name: &str) -> Result<EmbeddingPooling, String> {
    match name.to_ascii_lowercase().as_str() {
        "last" => Ok(EmbeddingPooling::Last),
        "mean" => Ok(EmbeddingPooling::Mean),
        "cls" => Ok(EmbeddingPooling::Cls),
        other => Err(format!("unknown pooling '{other}'")),
    }
}

fn parse_target_fpr(raw: &str) -> Result<f32, String> {
    raw.parse()
        .map_err(|e| format!("--target-fpr '{raw}' is not a number: {e}"))
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut corpus = None;
    let mut key_file = None;
    let mut out = None;
    let mut taxonomy = None;
    let mut index_id = "default".to_string();
    let mut category = DataCategory::Internal;
    let mut sensitivity = SensitivityLevel::Confidential;
    let mut family = None;
    let mut boilerplate = None;
    let mut embedder = None;
    let mut embedder_source = None;
    let mut pooling = None;
    let mut anchors = None;
    let mut calibrate_positives = None;
    let mut calibrate_negatives = None;
    let mut target_fpr = 0.01f32;
    let mut semantic_thresholds_out = None;
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
            "--embedder" => embedder = Some(path(i, "--embedder")?),
            "--embedder-source" => embedder_source = Some(take(i, "--embedder-source")?),
            "--pooling" => pooling = Some(parse_pooling(&take(i, "--pooling")?)?),
            "--anchors" => anchors = Some(path(i, "--anchors")?),
            "--calibrate-positives" => {
                calibrate_positives = Some(path(i, "--calibrate-positives")?);
            }
            "--calibrate-negatives" => {
                calibrate_negatives = Some(path(i, "--calibrate-negatives")?);
            }
            "--target-fpr" => target_fpr = parse_target_fpr(&take(i, "--target-fpr")?)?,
            "--semantic-thresholds-out" => {
                semantic_thresholds_out = Some(path(i, "--semantic-thresholds-out")?);
            }
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
    let embedder = embedder
        .map(|embedder| {
            Ok::<_, String>(EmbedderFlags {
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

fn build_index(args: &[String]) -> Result<(), String> {
    let options = parse(args)?;

    // The semantic pass needs llama.cpp linked in, which only happens under
    // `sentinel`; a `knowledge-pack`-only build must refuse the flag rather
    // than silently building an index with no semantic section.
    #[cfg(not(feature = "sentinel"))]
    if let Some(flags) = &options.embedder {
        return Err(format!(
            "this build was compiled without the sentinel feature; cannot build the semantic \
             tier from {} (source {}, pooling {:?}, anchors {}, positives {}, negatives {}, \
             target-fpr {}, thresholds-out {}, evidence-out {})",
            flags.embedder.display(),
            flags.embedder_source,
            flags.pooling,
            flags.anchors.display(),
            flags.calibrate_positives.display(),
            flags.calibrate_negatives.display(),
            flags.target_fpr,
            flags.semantic_thresholds_out.display(),
            flags.eval_evidence_out.display(),
        ));
    }

    let taxonomy = match &options.taxonomy {
        Some(path) => {
            let json = fs::read_to_string(path)
                .map_err(|e| format!("cannot read taxonomy {}: {e}", path.display()))?;
            TaxonomyMap::from_json(&json).map_err(|e| format!("taxonomy is unusable: {e}"))?
        }
        None => TaxonomyMap::v1().clone(),
    };

    // KP-009 edge case: no key, no index. There is no unkeyed fallback.
    let secret = fs::read(&options.key_file).map_err(|e| {
        format!(
            "cannot read tenant key {}: {e}. The index cannot be built without one.",
            options.key_file.display()
        )
    })?;
    let key = IndexKey::derive(&secret, &options.index_id)
        .map_err(|e| format!("tenant key is unusable: {e}"))?;

    let mut builder = ReferenceIndex::builder(&key, taxonomy.version());
    let mut near = NearDuplicateIndex::builder(&key, taxonomy.version());
    let mut near_documents = 0usize;

    let (docs, skipped) = read_documents(&options)?;
    for (text, family) in &docs {
        builder.add_document(&key, text, options.category, options.sensitivity, family);
        // The near-duplicate tier refuses documents too short for a stable
        // fingerprint. That is not a failure to index them: the exact tier
        // covers that size well, and a fingerprint over a handful of shingles
        // would only ever match itself.
        if near.add_document(
            &key,
            text,
            EntryMeta {
                category: options.category,
                sensitivity: options.sensitivity,
                source_family: family.clone(),
            },
        ) {
            near_documents += 1;
        }
    }
    let documents = docs.len();
    if skipped > 0 {
        println!("Skipped {skipped} corpus rows recorded under another label");
    }

    let mut boilerplate_files = 0usize;
    if let Some(dir) = &options.boilerplate {
        for path in text_files(dir)? {
            if let Ok(text) = fs::read_to_string(&path) {
                builder.add_boilerplate(&key, &text);
                boilerplate_files += 1;
            }
        }
    }

    #[cfg(feature = "sentinel")]
    let semantic = build_semantic_section(&options, &docs, &taxonomy)?;
    #[cfg(not(feature = "sentinel"))]
    let semantic: Option<arkavo_fingerprint::SemanticIndex> = None;

    let index = builder.build();
    let near = near.build();
    let wrap = taxonomy.clearance_requirement(index.max_sensitivity());

    // All three tiers travel as one component: they are built from one corpus
    // under one tenant key, and shipping them separately is how they drift
    // apart.
    let indexes = arkavo_knowledge_pack::PackIndexes {
        reference: index,
        near: Some(near),
        semantic,
    };
    let encoded =
        serde_json::to_vec(&indexes).map_err(|e| format!("cannot serialize the index: {e}"))?;
    fs::write(&options.out, &encoded)
        .map_err(|e| format!("cannot write {}: {e}", options.out.display()))?;
    let index = &indexes.reference;

    println!(
        "Indexed {documents} documents, {} entries ({near_documents} near-duplicate signatures)",
        index.len()
    );
    if boilerplate_files > 0 {
        println!(
            "Suppressed {} shingles from {boilerplate_files} boilerplate files",
            index.suppression().len()
        );
    }
    println!("Classification: {:?}", index.max_sensitivity());
    match &wrap {
        Some(attribute) => println!("Wrap under: {}={}", attribute.fqn, attribute.value),
        None => println!("Wrap under: no clearance required (public)"),
    }
    println!("Wrote {}", options.out.display());
    println!(
        "Note: this output is plaintext. Wrap it before `arkavo pack seal` — \
         seal copies a SealedBlob and refuses a plaintext index, because the \
         labels beside each entry say how sensitive the corpus is. Treat it as \
         classified at the level above until it is wrapped."
    );
    Ok(())
}

/// Read corpus documents as (text, family) pairs. A `.jsonl` corpus supplies
/// its own family per row and may skip rows filed under another label; a
/// directory corpus applies `--family` (default `"corpus"`) to every file.
fn read_documents(options: &Options) -> Result<(Vec<(String, String)>, usize), String> {
    if is_jsonl_corpus(&options.corpus) {
        return read_jsonl_documents(options);
    }
    let family = options
        .family
        .clone()
        .unwrap_or_else(|| "corpus".to_string());
    let mut docs = Vec::new();
    for path in text_files(&options.corpus)? {
        match fs::read_to_string(&path) {
            Ok(text) => docs.push((text, family.clone())),
            // Unreadable or non-UTF-8 files are reported and skipped: silently
            // dropping corpus material makes the index quietly incomplete.
            Err(e) => eprintln!("skipping {}: {e}", path.display()),
        }
    }
    Ok((docs, 0))
}

#[cfg(feature = "sentinel")]
fn read_jsonl_documents(options: &Options) -> Result<(Vec<(String, String)>, usize), String> {
    let label = arkavo_fingerprint::label_key(options.category, options.sensitivity);
    let (docs, skipped) = pack_semantic::read_corpus_jsonl(&options.corpus, &label)?;
    Ok((
        docs.into_iter().map(|d| (d.text, d.family)).collect(),
        skipped,
    ))
}

#[cfg(not(feature = "sentinel"))]
fn read_jsonl_documents(_options: &Options) -> Result<(Vec<(String, String)>, usize), String> {
    Err("this build was compiled without the sentinel feature".to_string())
}

/// Build the semantic section, when `--embedder` was given. `EmbedderFlags`
/// already holds every companion unwrapped, since `parse` refused to build
/// one without them.
#[cfg(feature = "sentinel")]
fn build_semantic_section(
    options: &Options,
    docs: &[(String, String)],
    taxonomy: &TaxonomyMap,
) -> Result<Option<arkavo_fingerprint::SemanticIndex>, String> {
    let Some(flags) = &options.embedder else {
        return Ok(None);
    };
    let semantic_options = pack_semantic::SemanticOptions {
        embedder: flags.embedder.clone(),
        embedder_source: flags.embedder_source.clone(),
        pooling: flags.pooling,
        anchors: flags.anchors.clone(),
        calibrate_positives: flags.calibrate_positives.clone(),
        calibrate_negatives: flags.calibrate_negatives.clone(),
        target_fpr: flags.target_fpr,
        semantic_thresholds_out: flags.semantic_thresholds_out.clone(),
        eval_evidence_out: flags.eval_evidence_out.clone(),
    };
    let documents: Vec<pack_semantic::CorpusDoc> = docs
        .iter()
        .map(|(text, family)| pack_semantic::CorpusDoc {
            text: text.clone(),
            family: family.clone(),
        })
        .collect();
    let index = pack_semantic::run(
        &semantic_options,
        &documents,
        options.category,
        options.sensitivity,
        taxonomy,
    )?;
    Ok(Some(index))
}

/// Corpus files, deepest-first order irrelevant — the index is a set.
fn text_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|e| format!("cannot read directory {}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if is_text(&path) {
                found.push(path);
            }
        }
    }
    Ok(found)
}

fn is_text(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| TEXT_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn only_text_extensions_are_indexed() {
        assert!(is_text(Path::new("notes.md")));
        assert!(is_text(Path::new("a/b/report.TXT")));
        assert!(!is_text(Path::new("model.gguf")));
        assert!(!is_text(Path::new("archive.tar.gz")));
    }

    #[test]
    fn there_is_no_option_that_builds_without_a_key() {
        // KP-009: an unavailable key fails the build. A flag that skipped
        // keying would reintroduce the dictionary the design exists to avoid.
        let err = parse(&["--no-key".into(), "x".into()]).unwrap_err();

        assert!(err.contains("unknown option"), "{err}");
    }

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

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
}
