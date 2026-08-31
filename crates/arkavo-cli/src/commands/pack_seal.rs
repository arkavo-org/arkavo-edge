//! `arkavo pack seal` and `arkavo pack verify` (KP-002, KP-003, KP-005).
//!
//! Sealing is a build-time step run where the components already live. What
//! comes out is a directory whose manifest names every component and its
//! digest, signed once over the manifest bytes.
//!
//! Verification takes a *resolved* anchor public key as a file. There is no
//! `did:webvh` resolver in this workspace, and a verify command that fetched
//! its own trust root would be choosing what to trust — so the operator
//! supplies it and the absence of one is a refusal, not a warning.

use std::path::{Path, PathBuf};

use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use arkavo_gguf_tdf::{Classification, ComponentRole};
use arkavo_knowledge_pack::{Lineage, PackBuilder, verify_pack};

pub fn run(args: &[String]) -> Result<(), String> {
    let options = SealOptions::parse(args)?;

    let key_bytes = std::fs::read(&options.signing_key).map_err(|e| {
        format!(
            "cannot read the signing key {}: {e}",
            options.signing_key.display()
        )
    })?;
    let key = AgentKeypair::from_bytes(&key_bytes)
        .map_err(|e| format!("the signing key is unusable: {e}"))?;

    let mut builder = PackBuilder::new(
        &options.pack_id,
        &options.taxonomy_version,
        &options.tokenizer,
    )
    .with_lineage(options.lineage.clone());
    if let Some(digest) = &options.corpus_digest {
        builder = builder.with_corpus_digest(digest);
    }
    if let Some(path) = &options.thresholds {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let value: serde_json::Value = serde_json::from_str(&text)
            .map_err(|e| format!("the calibration table is not valid JSON: {e}"))?;
        builder = builder.with_thresholds(value);
    }
    if let Some(digest) = &options.eval_evidence {
        builder = builder.with_eval_evidence(digest);
    }

    for component in &options.components {
        if matches!(component.role, ComponentRole::Index) {
            refuse_plaintext_index(&component.path)?;
        }
        builder
            .add_component(&component.path, component.role.clone(), component.ceiling)
            .map_err(|e| format!("{e}"))?;
    }

    let ceiling = builder.ceiling();
    let manifest = builder
        .build(&options.out, &key)
        .map_err(|e| format!("{e}"))?;

    println!(
        "Sealed pack {} with {} component(s)",
        manifest.pack_id,
        manifest.components.len()
    );
    for component in &manifest.components {
        println!(
            "  {:<28} {:<9} {}",
            component.file,
            component.role.as_str(),
            component.effective_ceiling().as_str()
        );
    }
    println!("Pack ceiling: {}", ceiling.as_str());
    println!("Wrote {}", options.out.display());
    Ok(())
}

pub fn verify(args: &[String]) -> Result<(), String> {
    let mut pack: Option<PathBuf> = None;
    let mut anchor: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let value = || -> Result<String, String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} requires a value", args[i]))
        };
        match args[i].as_str() {
            "--pack" => pack = Some(PathBuf::from(value()?)),
            "--anchor" => anchor = Some(PathBuf::from(value()?)),
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 2;
    }
    let pack = pack.ok_or("--pack is required")?;
    // KP-003: no anchor, no trust. There is deliberately no flag that verifies
    // structure while skipping the signature — that is trust on first use with
    // extra steps.
    let anchor_path = anchor
        .ok_or("--anchor is required; a pack cannot be trusted without an organization anchor")?;
    let anchor_bytes = std::fs::read(&anchor_path)
        .map_err(|e| format!("cannot read the anchor {}: {e}", anchor_path.display()))?;
    let anchor = AgentPublicKey::from_bytes(&anchor_bytes)
        .map_err(|e| format!("the anchor key is unusable: {e}"))?;

    let verified = verify_pack(&pack, Some(&anchor)).map_err(|e| format!("{e}"))?;

    println!("Pack {} verified", verified.manifest.pack_id);
    println!("Taxonomy: {}", verified.manifest.taxonomy_version);
    println!("Ceiling:  {}", verified.manifest.ceiling().as_str());
    println!("Held:     {}", verified.present.join(", "));
    if !verified.absent.is_empty() {
        // Not a failure: a node holding part of a pack is the point of
        // wrapping components separately (KP-005).
        println!("Absent:   {}", verified.absent.join(", "));
    }
    Ok(())
}

/// KP-009: an index leaves the build wrapped or not at all.
///
/// A keyed index is not safe to distribute in the clear. Keying stops an
/// attacker computing entries for guesses; it does nothing about the labels
/// sitting next to each entry, which say how sensitive the corpus is and how
/// much of it there is. Sealing is refused rather than warned about, because a
/// warning at build time is a warning nobody sees at distribution time.
fn refuse_plaintext_index(path: &Path) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    if serde_json::from_slice::<arkavo_knowledge_pack::SealedBlob>(&bytes).is_ok() {
        return Ok(());
    }
    Err(format!(
        "{} is a plaintext index. An index component must be wrapped before it is \
         sealed into a pack; a keyed index still carries the labels that say how \
         sensitive its corpus is.",
        path.display()
    ))
}

struct Component {
    path: PathBuf,
    role: ComponentRole,
    ceiling: Option<Classification>,
}

struct SealOptions {
    out: PathBuf,
    signing_key: PathBuf,
    pack_id: String,
    taxonomy_version: String,
    tokenizer: String,
    thresholds: Option<PathBuf>,
    corpus_digest: Option<String>,
    eval_evidence: Option<String>,
    lineage: Lineage,
    components: Vec<Component>,
}

impl SealOptions {
    fn parse(args: &[String]) -> Result<Self, String> {
        let mut out = None;
        let mut signing_key = None;
        let mut pack_id = None;
        let mut taxonomy_version = "1.0.0".to_string();
        let mut tokenizer = String::new();
        let mut thresholds = None;
        let mut corpus_digest = None;
        let mut eval_evidence = None;
        let mut lineage = Lineage::Root;
        let mut components = Vec::new();

        let mut i = 0;
        while i < args.len() {
            let value = || -> Result<String, String> {
                args.get(i + 1)
                    .cloned()
                    .ok_or_else(|| format!("{} requires a value", args[i]))
            };
            match args[i].as_str() {
                "--out" => out = Some(PathBuf::from(value()?)),
                "--signing-key" => signing_key = Some(PathBuf::from(value()?)),
                "--pack-id" => pack_id = Some(value()?),
                "--taxonomy-version" => taxonomy_version = value()?,
                "--tokenizer" => tokenizer = value()?,
                "--thresholds" => thresholds = Some(PathBuf::from(value()?)),
                "--corpus-digest" => corpus_digest = Some(value()?),
                "--eval-evidence" => eval_evidence = Some(value()?),
                "--parent" => lineage = parse_parent(&value()?)?,
                "--component" => components.push(parse_component(&value()?)?),
                other => return Err(format!("unknown option '{other}'")),
            }
            i += 2;
        }

        Ok(Self {
            out: out.ok_or("--out is required")?,
            signing_key: signing_key.ok_or("--signing-key is required")?,
            pack_id: pack_id.ok_or("--pack-id is required")?,
            taxonomy_version,
            tokenizer,
            thresholds,
            corpus_digest,
            eval_evidence,
            lineage,
            components,
        })
    }
}

/// `<path>:<role>[:<ceiling>]`, where an adapter's role is `adapter/<compartment>`.
///
/// Parsed from the right so a Windows drive letter is part of the path, not a
/// role named `\packs\index.tdf`.
fn parse_component(spec: &str) -> Result<Component, String> {
    let mut parts = spec.rsplitn(3, ':');
    let last = parts
        .next()
        .ok_or_else(|| format!("component '{spec}' must be <path>:<role>[:<ceiling>]"))?;
    let middle = parts.next();
    let first = parts.next();
    let (path, role_name, ceiling) = match (first, middle) {
        (Some(path), Some(role_name)) => (path, role_name, Some(last)),
        (None, Some(path)) => (path, last, None),
        (_, None) => {
            return Err(format!(
                "component '{spec}' must be <path>:<role>[:<ceiling>]"
            ));
        }
    };
    let role = match role_name {
        "sentinel" => ComponentRole::Sentinel,
        "index" => ComponentRole::Index,
        "model" => ComponentRole::Model,
        adapter if adapter.starts_with("adapter/") => ComponentRole::Adapter {
            compartment: adapter["adapter/".len()..].to_string(),
        },
        // An adapter with no compartment would be an adapter nobody can be
        // entitled to specifically, which is not a thing a pack should hold.
        "adapter" => {
            return Err("an adapter must name its compartment: adapter/<compartment>".to_string());
        }
        other => return Err(format!("unknown component role '{other}'")),
    };
    Ok(Component {
        path: PathBuf::from(path),
        role,
        ceiling: ceiling.map(parse_ceiling).transpose()?,
    })
}

fn parse_ceiling(name: &str) -> Result<Classification, String> {
    match name.to_ascii_lowercase().as_str() {
        "public" => Ok(Classification::Public),
        "internal" => Ok(Classification::Internal),
        "confidential" => Ok(Classification::Confidential),
        "restricted" => Ok(Classification::Restricted),
        other => Err(format!("unknown classification '{other}'")),
    }
}

fn parse_parent(spec: &str) -> Result<Lineage, String> {
    let (pack_id, digest) = spec
        .split_once(':')
        .ok_or_else(|| format!("parent '{spec}' must be <pack-id>:<manifest-digest>"))?;
    Ok(Lineage::Parent {
        pack_id: pack_id.to_string(),
        manifest_digest: digest.to_string(),
    })
}

/// `arkavo pack anchor` — derive the verifying key from a signing key.
///
/// Without this there is no way to produce the file `verify --anchor` wants,
/// which would leave the signed-pack path unusable end to end. It writes the
/// public half only; the signing key never leaves the file it came from.
pub fn anchor(args: &[String]) -> Result<(), String> {
    let mut signing_key: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let value = || -> Result<String, String> {
            args.get(i + 1)
                .cloned()
                .ok_or_else(|| format!("{} requires a value", args[i]))
        };
        match args[i].as_str() {
            "--signing-key" => signing_key = Some(PathBuf::from(value()?)),
            "--out" => out = Some(PathBuf::from(value()?)),
            other => return Err(format!("unknown option '{other}'")),
        }
        i += 2;
    }
    let signing_key = signing_key.ok_or("--signing-key is required")?;
    let out = out.ok_or("--out is required")?;

    let bytes = std::fs::read(&signing_key)
        .map_err(|e| format!("cannot read {}: {e}", signing_key.display()))?;
    let key = AgentKeypair::from_bytes(&bytes)
        .map_err(|e| format!("the signing key is unusable: {e}"))?;
    write_public(&out, &key)?;
    println!("Wrote the anchor to {}", out.display());
    println!("did:key: {}", key.public_key().to_did_key());
    Ok(())
}

fn write_public(out: &Path, key: &AgentKeypair) -> Result<(), String> {
    std::fs::write(out, key.public_key().to_bytes())
        .map_err(|e| format!("cannot write {}: {e}", out.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_adapter_component_names_its_compartment() {
        let component = parse_component("a.gguf.tdf:adapter/legal:confidential").expect("parse");

        assert_eq!(component.role.compartment(), Some("legal"));
        assert_eq!(component.ceiling, Some(Classification::Confidential));
    }

    #[test]
    fn an_adapter_without_a_compartment_is_refused() {
        // An adapter nobody can be entitled to specifically is not a component.
        assert!(parse_component("a.gguf.tdf:adapter").is_err());
    }

    #[test]
    fn a_component_may_omit_its_ceiling() {
        let component = parse_component("index.tdf:index").expect("parse");

        assert_eq!(component.ceiling, None);
    }

    #[test]
    fn a_windows_path_is_the_path_not_a_role() {
        // split(':') from the left takes the drive letter as the path.
        let component = parse_component(r"C:\packs\index.tdf:index:confidential").expect("parse");

        assert_eq!(component.path.to_string_lossy(), r"C:\packs\index.tdf");
        assert!(matches!(component.role, ComponentRole::Index));
        assert_eq!(component.ceiling, Some(Classification::Confidential));
    }

    #[test]
    fn an_unknown_role_is_refused_rather_than_guessed() {
        assert!(parse_component("x.tdf:weights").is_err());
        assert!(parse_component("x.tdf").is_err());
    }

    #[test]
    fn lineage_defaults_to_root_and_a_parent_needs_a_digest() {
        // A named parent with no digest is a claim, not lineage.
        assert!(parse_parent("pack-0").is_err());
        assert!(matches!(
            parse_parent("pack-0:abc").expect("parse"),
            Lineage::Parent { .. }
        ));
    }

    #[test]
    fn verify_refuses_without_an_anchor() {
        let err = verify(&["--pack".into(), "/tmp/x".into()]).unwrap_err();

        assert!(err.contains("--anchor"), "{err}");
    }
}
