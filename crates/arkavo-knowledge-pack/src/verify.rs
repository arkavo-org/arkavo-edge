//! Opening a pack (KP-003, KP-004, KP-005).
//!
//! Order is the whole of it. The signature is checked over the manifest bytes
//! before the manifest is parsed, the digests are checked before any component
//! is used, and no key is requested until both have passed. A digest check that
//! ran after a key request would be a check on content the KAS had already
//! agreed to release.
//!
//! Absent is not tampered. An egress node legitimately holds the sentinel and
//! the index and not the adapters (KP-005), so a component the manifest lists
//! and the disk does not is recorded rather than treated as an attack. A
//! component that is present and does not match its digest is the attack.

use std::path::{Path, PathBuf};

use arkavo_crypto::AgentPublicKey;

use crate::manifest::{
    ManifestError, PACK_MANIFEST_FILE, PACK_SIGNATURE_FILE, PackManifest, digest_of,
};
use crate::sign::{SignatureError, decode_signature, verify_manifest};

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("cannot read {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error(transparent)]
    Signature(#[from] SignatureError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("component {file} does not match its manifest digest (expected {expected})")]
    DigestMismatch { file: String, expected: String },
}

/// A pack whose manifest verified, and what of it is actually here.
#[derive(Debug, Clone)]
pub struct VerifiedPack {
    pub root: PathBuf,
    pub manifest: PackManifest,
    /// Components present and matching their digest.
    pub present: Vec<String>,
    /// Components the manifest lists that this node does not hold.
    pub absent: Vec<String>,
}

impl VerifiedPack {
    pub fn holds(&self, file: &str) -> bool {
        self.present.iter().any(|f| f == file)
    }

    pub fn path(&self, file: &str) -> PathBuf {
        self.root.join(file)
    }

    /// What this node holds, for the audit record (KP-005).
    pub fn inventory(&self) -> String {
        format!(
            "pack {} holds [{}], missing [{}]",
            self.manifest.pack_id,
            self.present.join(", "),
            self.absent.join(", ")
        )
    }
}

/// Verify a pack directory against a resolved organization anchor.
///
/// `anchor` is the *resolved* public key. Resolving the organization's
/// `did:webvh` to it is not done here — there is no resolver in this workspace,
/// and a verifier that fetched its own trust root would be deciding what to
/// trust. `None` is refused (KP-003: no trust-on-first-use fallback).
pub fn verify_pack(
    root: &Path,
    anchor: Option<&AgentPublicKey>,
) -> Result<VerifiedPack, VerifyError> {
    let manifest_path = root.join(PACK_MANIFEST_FILE);
    let bytes = read(&manifest_path)?;
    let signature_text =
        String::from_utf8_lossy(&read(&root.join(PACK_SIGNATURE_FILE))?).into_owned();
    let signature = decode_signature(&signature_text)?;

    // Before parsing: a manifest that does not verify is not a manifest, and
    // parsing it first would mean acting on attacker-chosen structure.
    verify_manifest(&bytes, &signature, anchor)?;

    let manifest = PackManifest::from_bytes(&bytes)?;
    manifest.check()?;

    let mut present = Vec::new();
    let mut absent = Vec::new();
    for component in &manifest.components {
        let path = root.join(&component.file);
        match std::fs::read(&path) {
            Ok(content) => {
                if digest_of(&content) != component.digest {
                    return Err(VerifyError::DigestMismatch {
                        file: component.file.clone(),
                        expected: component.digest.clone(),
                    });
                }
                present.push(component.file.clone());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                absent.push(component.file.clone());
            }
            Err(e) => {
                return Err(VerifyError::Read { path, source: e });
            }
        }
    }

    Ok(VerifiedPack {
        root: root.to_path_buf(),
        manifest,
        present,
        absent,
    })
}

fn read(path: &Path) -> Result<Vec<u8>, VerifyError> {
    std::fs::read(path).map_err(|e| VerifyError::Read {
        path: path.to_path_buf(),
        source: e,
    })
}
