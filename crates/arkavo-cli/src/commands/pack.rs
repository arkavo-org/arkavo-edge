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
use arkavo_protocol::data_classification::{DataCategory, SensitivityLevel};
use arkavo_protocol::taxonomy::TaxonomyMap;

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
    println!("  --corpus <DIR>        Directory of corpus material to index");
    println!("  --key-file <PATH>     Tenant index key material (>= 16 bytes)");
    println!("  --out <PATH>          Where to write the index");
    println!("  --taxonomy <PATH>     Taxonomy map (default: the embedded v1 map)");
    println!("  --index-id <NAME>     Separates indices under one tenant key");
    println!("  --category <NAME>     Category for corpus documents (default: internal)");
    println!("  --sensitivity <NAME>  Sensitivity for corpus documents (default: confidential)");
    println!("  --family <NAME>       Source family recorded on matches");
    println!("  --boilerplate <DIR>   Directory of material to suppress");
    println!("\nSeal options:");
    println!("  --out <DIR>           Where to write the pack");
    println!("  --signing-key <PATH>  Organization signing key (32 raw bytes)");
    println!("  --pack-id <ID>        Identity of the pack being built");
    println!("  --taxonomy-version <V>  Taxonomy map version the pack was derived against");
    println!("  --tokenizer <NAME>    Tokenizer identity");
    println!("  --thresholds <PATH>   Calibration table JSON bound into the manifest");
    println!("  --component <PATH>:<ROLE>[:<CEILING>]  A component and its role");
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
    family: String,
    boilerplate: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Options, String> {
    let mut corpus = None;
    let mut key_file = None;
    let mut out = None;
    let mut taxonomy = None;
    let mut index_id = "default".to_string();
    let mut category = DataCategory::Internal;
    let mut sensitivity = SensitivityLevel::Confidential;
    let mut family = "corpus".to_string();
    let mut boilerplate = None;

    let mut i = 0;
    while i < args.len() {
        let take = |i: usize, what: &str| -> Result<String, String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{what} requires a value"))
        };
        match args[i].as_str() {
            "--corpus" => corpus = Some(PathBuf::from(take(i, "--corpus")?)),
            "--key-file" => key_file = Some(PathBuf::from(take(i, "--key-file")?)),
            "--out" => out = Some(PathBuf::from(take(i, "--out")?)),
            "--taxonomy" => taxonomy = Some(PathBuf::from(take(i, "--taxonomy")?)),
            "--index-id" => index_id = take(i, "--index-id")?,
            "--category" => category = parse_category(&take(i, "--category")?)?,
            "--sensitivity" => sensitivity = parse_sensitivity(&take(i, "--sensitivity")?)?,
            "--family" => family = take(i, "--family")?,
            "--boilerplate" => boilerplate = Some(PathBuf::from(take(i, "--boilerplate")?)),
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 2;
    }

    Ok(Options {
        corpus: corpus.ok_or("--corpus is required")?,
        key_file: key_file.ok_or("--key-file is required")?,
        out: out.ok_or("--out is required")?,
        taxonomy,
        index_id,
        category,
        sensitivity,
        family,
        boilerplate,
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
    let mut documents = 0usize;
    let mut near_documents = 0usize;
    for path in text_files(&options.corpus)? {
        let text = match fs::read_to_string(&path) {
            Ok(text) => text,
            // Unreadable or non-UTF-8 files are reported and skipped: silently
            // dropping corpus material makes the index quietly incomplete.
            Err(e) => {
                eprintln!("skipping {}: {e}", path.display());
                continue;
            }
        };
        builder.add_document(
            &key,
            &text,
            options.category,
            options.sensitivity,
            &options.family,
        );
        // The near-duplicate tier refuses documents too short for a stable
        // fingerprint. That is not a failure to index them: the exact tier
        // covers that size well, and a fingerprint over a handful of shingles
        // would only ever match itself.
        if near.add_document(
            &key,
            &text,
            EntryMeta {
                category: options.category,
                sensitivity: options.sensitivity,
                source_family: options.family.clone(),
            },
        ) {
            near_documents += 1;
        }
        documents += 1;
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

    let index = builder.build();
    let near = near.build();
    let wrap = taxonomy.clearance_requirement(index.max_sensitivity());

    // Both tiers travel as one component: they are built from one corpus under
    // one tenant key, and shipping them separately is how they drift apart.
    let indexes = arkavo_knowledge_pack::PackIndexes {
        reference: index,
        near: Some(near),
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
        "Note: this output is plaintext. `arkavo pack seal --component {}:index` \
         wraps it before distribution; treat it as classified at the level above \
         until it is sealed.",
        options.out.display()
    );
    Ok(())
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
}
