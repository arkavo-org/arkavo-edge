//! `arkavo model protect` — wrap a GGUF into a KAS-gated `.gguf.tdf`.
//!
//! Fetching the KAS public key is the one asynchronous step; the wrap itself
//! is synchronous and streams the source a segment at a time, so a 12 GiB
//! model never lands in memory.

use anyhow::{Context, Result, bail};
use arkavo_gguf_tdf::{DEFAULT_MAX_SEGMENT, ProtectOptions, RsaOaepWrapper, protect};
use std::path::{Path, PathBuf};

/// Default KAS when the caller does not name one.
const DEFAULT_KAS_URL: &str = "https://platform.arkavo.net";

/// Arguments for the protect subcommand.
pub struct ProtectArgs<'a> {
    pub path: &'a Path,
    pub output: Option<&'a Path>,
    pub kas_url: Option<&'a str>,
    pub max_segment: Option<u64>,
    pub attributes: &'a [String],
    pub delete_source: bool,
}

/// Default output path: `<source>.tdf`, giving `model.gguf.tdf`.
pub fn default_output(source: &Path) -> PathBuf {
    let mut name = source.as_os_str().to_os_string();
    name.push(".tdf");
    PathBuf::from(name)
}

pub async fn run(args: ProtectArgs<'_>) -> Result<()> {
    if !args.path.exists() {
        bail!("model not found: {}", args.path.display());
    }
    let source_name = args.path.to_string_lossy();
    if source_name.to_lowercase().ends_with(".gguf.tdf") {
        bail!("{source_name} is already protected");
    }

    let dest = args
        .output
        .map(Path::to_path_buf)
        .unwrap_or_else(|| default_output(args.path));
    if dest.exists() {
        bail!(
            "{} already exists; remove it or pass --output",
            dest.display()
        );
    }

    let kas_url = args.kas_url.unwrap_or(DEFAULT_KAS_URL);

    // The resolved URL can come from a remote well-known document.
    let config = opentdf::kas_discovery::OpentdfConfiguration::for_kas_connect(kas_url);
    let endpoints = opentdf::kas_discovery::KasEndpoints::from_config(&config)
        .with_context(|| format!("cannot resolve KAS endpoints for {kas_url}"))?;

    let http = reqwest::Client::new();
    let kas_key = opentdf::kas_key::fetch_kas_public_key_connect(&endpoints.public_key_url, &http)
        .await
        .with_context(|| {
            format!(
                "cannot fetch the KAS public key from {}",
                endpoints.public_key_url
            )
        })?;

    let wrapper = RsaOaepWrapper::new(kas_url, Some(kas_key.kid.clone()), kas_key.public_key);
    let opts = ProtectOptions {
        max_segment: args.max_segment.unwrap_or(DEFAULT_MAX_SEGMENT),
        attributes: args.attributes.to_vec(),
        ..Default::default()
    };

    println!("Protecting {} ...", args.path.display());
    let report = protect(args.path, &dest, &wrapper, &opts)
        .with_context(|| format!("cannot protect {}", args.path.display()))?;

    // Structural read-back (no KAS round-trip): a truncated or malformed
    // archive must be caught before the only plaintext copy can be removed.
    arkavo_gguf_tdf::GgufTdfArchive::open(&dest).with_context(|| {
        format!(
            "wrote {} but it failed to reopen; the source was not deleted",
            dest.display()
        )
    })?;

    println!("  wrote      {}", dest.display());
    println!("  segments   {}", report.segments);
    println!("  header     {} bytes", report.header_bytes);
    println!("  virtual    {} bytes", report.virtual_size);
    println!("  archive    {} bytes", report.archive_bytes);
    println!("  kas        {kas_url} (kid {})", kas_key.kid);
    if opts.attributes.is_empty() {
        println!("  policy     no data attributes; anyone the KAS admits can load this");
    } else {
        for attribute in &opts.attributes {
            println!("  attribute  {attribute}");
        }
    }

    if args.delete_source {
        std::fs::remove_file(args.path)
            .with_context(|| format!("cannot delete {}", args.path.display()))?;
        println!("  removed    {}", args.path.display());
    } else {
        println!(
            "  kept       {} (pass --delete-source to remove it)",
            args.path.display()
        );
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_kas_is_platform() {
        assert_eq!(DEFAULT_KAS_URL, "https://platform.arkavo.net");
    }

    #[test]
    fn default_output_appends_tdf_to_the_whole_name() {
        assert_eq!(
            default_output(Path::new("/models/Mixtral-8x7B-v0.1-KQ2.gguf")),
            PathBuf::from("/models/Mixtral-8x7B-v0.1-KQ2.gguf.tdf")
        );
        // The extension is appended, not replaced.
        assert_eq!(
            default_output(Path::new("model.gguf")),
            PathBuf::from("model.gguf.tdf")
        );
    }

    #[tokio::test]
    async fn refuses_a_missing_source() {
        let err = run(ProtectArgs {
            path: Path::new("/nonexistent/model.gguf"),
            output: None,
            kas_url: None,
            max_segment: None,
            attributes: &[],
            delete_source: false,
        })
        .await
        .expect_err("a missing model must not be wrapped");
        assert!(err.to_string().contains("model not found"), "got: {err}");
    }

    #[tokio::test]
    async fn refuses_an_already_protected_source() {
        let dir = tempfile::tempdir().unwrap();
        let already = dir.path().join("model.gguf.tdf");
        std::fs::write(&already, b"PK\x03\x04").unwrap();

        let err = run(ProtectArgs {
            path: &already,
            output: None,
            kas_url: None,
            max_segment: None,
            attributes: &[],
            delete_source: false,
        })
        .await
        .expect_err("double-wrapping must be refused");
        assert!(err.to_string().contains("already protected"), "got: {err}");
    }

    #[tokio::test]
    async fn refuses_to_overwrite_an_existing_archive() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("model.gguf");
        let dest = dir.path().join("model.gguf.tdf");
        std::fs::write(&source, b"GGUF").unwrap();
        std::fs::write(&dest, b"PK\x03\x04").unwrap();

        let err = run(ProtectArgs {
            path: &source,
            output: None,
            kas_url: None,
            max_segment: None,
            attributes: &[],
            delete_source: false,
        })
        .await
        .expect_err("an existing archive must not be clobbered");
        assert!(err.to_string().contains("already exists"), "got: {err}");
    }
}
