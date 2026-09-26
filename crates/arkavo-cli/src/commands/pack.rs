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

use arkavo_fingerprint::{EntryMeta, IndexKey, NearDuplicateIndex, ReferenceIndex};
use arkavo_protocol::taxonomy::TaxonomyMap;

use super::pack_options::{self, Options};
#[cfg(feature = "sentinel")]
use super::pack_semantic;

/// Files read as corpus material. Everything else is skipped rather than
/// guessed at: an index built from a binary's bytes is noise that costs
/// lookups.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "md", "rst", "csv", "json", "yaml", "yml", "toml", "rs", "py", "go", "ts", "js", "java",
    "sql", "html", "xml",
];

/// One corpus document ready for indexing: text plus the family id its own
/// JSONL row (or the corpus-wide `--family` flag, for a directory corpus)
/// recorded it under.
///
/// Shared by the keyed tiers built here and the semantic tier in
/// `pack_semantic`, so a document is read once and carried as one value
/// rather than unpacked into a tuple and rebuilt on each side.
#[derive(Debug)]
pub struct CorpusDoc {
    pub text: String,
    pub family: String,
}

pub fn execute(args: &[String]) -> Result<(), String> {
    match args.first().map(String::as_str) {
        Some("index") => build_index(&args[1..]),
        Some("wrap") => super::pack_wrap::run(&args[1..]),
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
        "  arkavo pack wrap --in <PATH> --out <PATH> --payload-key-out <PATH> [--taxonomy <PATH>]"
    );
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
    println!("\nWrap options:");
    println!("  --in <PATH>              Plaintext index from `arkavo pack index`");
    println!("  --out <PATH>             Where to write the wrapped index");
    println!(
        "  --payload-key-out <PATH> Where to write the 32-byte payload key (never overwritten; \
         keep it like a secret)"
    );
    println!("  --taxonomy <PATH>        Taxonomy map (default: the embedded v1 map)");
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

fn build_index(args: &[String]) -> Result<(), String> {
    let options = pack_options::parse(args)?;

    let taxonomy = match &options.taxonomy {
        Some(path) => {
            let json = fs::read_to_string(path)
                .map_err(|e| format!("cannot read taxonomy {}: {e}", path.display()))?;
            TaxonomyMap::from_json(&json).map_err(|e| format!("taxonomy is unusable: {e}"))?
        }
        None => TaxonomyMap::v1().clone(),
    };

    // Every tier this command can build classifies under `options.category`,
    // so an undefined category is refused here, once, rather than only when
    // the semantic tier happens to be requested.
    if taxonomy.policy_for(options.category).is_none() {
        return Err(format!(
            "label is not defined by taxonomy {}",
            taxonomy.version()
        ));
    }

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
    for doc in &docs {
        builder.add_document(
            &key,
            &doc.text,
            options.category,
            options.sensitivity,
            &doc.family,
        );
        // The near-duplicate tier refuses documents too short for a stable
        // fingerprint. That is not a failure to index them: the exact tier
        // covers that size well, and a fingerprint over a handful of shingles
        // would only ever match itself.
        if near.add_document(
            &key,
            &doc.text,
            EntryMeta {
                category: options.category,
                sensitivity: options.sensitivity,
                source_family: doc.family.clone(),
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

/// Read corpus documents. A `.jsonl` corpus supplies its own family per row
/// and may skip rows filed under another label; a directory corpus applies
/// `--family` (default `"corpus"`) to every file.
fn read_documents(options: &Options) -> Result<(Vec<CorpusDoc>, usize), String> {
    if pack_options::is_jsonl_corpus(&options.corpus) {
        return read_jsonl_documents(options);
    }
    let family = options
        .family
        .clone()
        .unwrap_or_else(|| "corpus".to_string());
    let mut docs = Vec::new();
    for path in text_files(&options.corpus)? {
        match fs::read_to_string(&path) {
            Ok(text) => docs.push(CorpusDoc {
                text,
                family: family.clone(),
            }),
            // Unreadable or non-UTF-8 files are reported and skipped: silently
            // dropping corpus material makes the index quietly incomplete.
            Err(e) => eprintln!("skipping {}: {e}", path.display()),
        }
    }
    Ok((docs, 0))
}

#[cfg(feature = "sentinel")]
fn read_jsonl_documents(options: &Options) -> Result<(Vec<CorpusDoc>, usize), String> {
    let label = arkavo_fingerprint::label_key(options.category, options.sensitivity);
    pack_semantic::read_corpus_jsonl(&options.corpus, &label)
}

#[cfg(not(feature = "sentinel"))]
fn read_jsonl_documents(_options: &Options) -> Result<(Vec<CorpusDoc>, usize), String> {
    Err("this build was compiled without the sentinel feature".to_string())
}

/// Build the semantic section, when `--embedder` was given.
#[cfg(feature = "sentinel")]
fn build_semantic_section(
    options: &Options,
    docs: &[CorpusDoc],
    taxonomy: &TaxonomyMap,
) -> Result<Option<arkavo_fingerprint::SemanticIndex>, String> {
    let Some(semantic_options) = &options.embedder else {
        return Ok(None);
    };
    let index = pack_semantic::run(
        semantic_options,
        docs,
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
    fn only_text_extensions_are_indexed() {
        assert!(is_text(Path::new("notes.md")));
        assert!(is_text(Path::new("a/b/report.TXT")));
        assert!(!is_text(Path::new("model.gguf")));
        assert!(!is_text(Path::new("archive.tar.gz")));
    }

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    /// The taxonomy check runs in `build_index` itself, right after the
    /// taxonomy loads, so it covers every corpus this command can build —
    /// not only a build that also happens to pass `--embedder`.
    #[test]
    fn a_category_missing_from_a_custom_taxonomy_is_refused_before_any_tier_builds() {
        let dir = tempfile::tempdir().unwrap();
        let taxonomy_path = dir.path().join("taxonomy.json");
        // A taxonomy with only a "public" label: DataCategory::Internal (the
        // default `--category`) is not defined by it.
        std::fs::write(
            &taxonomy_path,
            r#"{"version":"1.0.0","namespace":"https://attr.example.com/","labels":[
                {"label":"public","category":"Public","sensitivity":"Public","unentitled":"redact"}
            ]}"#,
        )
        .unwrap();
        let corpus_dir = dir.path().join("corpus");
        std::fs::create_dir(&corpus_dir).unwrap();
        std::fs::write(corpus_dir.join("a.txt"), "hello world").unwrap();
        let key_path = dir.path().join("key.bin");
        std::fs::write(&key_path, [7u8; 32]).unwrap();
        let out_path = dir.path().join("index.json");

        let err = build_index(&args(&[
            "--corpus",
            corpus_dir.to_str().unwrap(),
            "--key-file",
            key_path.to_str().unwrap(),
            "--out",
            out_path.to_str().unwrap(),
            "--taxonomy",
            taxonomy_path.to_str().unwrap(),
        ]))
        .unwrap_err();
        assert!(err.contains("not defined by taxonomy"), "{err}");
        assert!(!out_path.exists(), "no index should have been written");
    }
}
