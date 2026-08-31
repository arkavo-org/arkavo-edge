//! The manifest that binds a pack (KP-002, KP-006, KP-014).
//!
//! A pack is several separately wrapped components. Nothing about that
//! arrangement is trustworthy on its own — an attacker can add a component,
//! swap one, or drop one, and each component's own TDF says nothing about the
//! others. The manifest is what makes the set a set: it names every component
//! and its digest, and one signature over the manifest covers all of them.
//!
//! Everything a runtime must not be free to choose locally lives here:
//! calibrated thresholds, the taxonomy version they were fitted against, and
//! each component's classification ceiling. A value the operator can edit is a
//! value the operator can lower.

use std::collections::BTreeSet;

use arkavo_gguf_tdf::{Classification, ComponentRole};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Format version of the manifest itself, independent of any component's.
pub const PACK_FORMAT_VERSION: &str = "1";

/// File name of the manifest inside a pack.
pub const PACK_MANIFEST_FILE: &str = "manifest.json";

/// File name of the detached signature over the manifest bytes.
pub const PACK_SIGNATURE_FILE: &str = "manifest.sig";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest is not valid JSON: {0}")]
    Malformed(String),
    #[error("manifest format version {0} is not supported (expected {PACK_FORMAT_VERSION})")]
    UnsupportedVersion(String),
    #[error("compartment {0} is served by more than one adapter")]
    DuplicateCompartment(String),
    #[error("the pack lists more than one {0} component")]
    DuplicateRole(String),
    #[error("two components would be written as {0}")]
    DuplicateFile(String),
    #[error("component {0} has no digest")]
    MissingDigest(String),
    #[error("pack lists no components")]
    Empty,
}

/// Hex-encoded SHA-256 of some bytes, the form every digest in a manifest takes.
pub fn digest_of(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// One component, as the manifest sees it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentRecord {
    /// File name within the pack. Identity for lookup only — the component's
    /// role comes from `role`, never from parsing this.
    pub file: String,
    #[serde(flatten)]
    pub role: ComponentRole,
    /// Hex SHA-256 of the component's bytes as written.
    pub digest: String,
    /// How far output derived from this component may travel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification_ceiling: Option<Classification>,
}

impl ComponentRecord {
    pub fn new(file: impl Into<String>, role: ComponentRole, bytes: &[u8]) -> Self {
        Self {
            file: file.into(),
            role,
            digest: digest_of(bytes),
            classification_ceiling: None,
        }
    }

    #[must_use]
    pub fn with_ceiling(mut self, ceiling: Classification) -> Self {
        self.classification_ceiling = Some(ceiling);
        self
    }

    /// The ceiling to apply when reasoning about this component.
    ///
    /// A component with none recorded is treated as the most restrictive, not
    /// the least (KP-008): an absent ceiling means nobody said, and nobody
    /// saying is not permission.
    pub fn effective_ceiling(&self) -> Classification {
        self.classification_ceiling
            .unwrap_or(Classification::Restricted)
    }
}

/// Where a pack came from.
///
/// A root pack says so explicitly rather than omitting the field (KP-002 edge
/// case). An absent field cannot be told apart from a field someone removed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Lineage {
    Root,
    Parent {
        pack_id: String,
        /// Digest of the parent's manifest bytes, so lineage is verifiable
        /// rather than a name anyone can claim.
        manifest_digest: String,
    },
}

/// What the pack is and what is in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackManifest {
    pub format_version: String,
    pub pack_id: String,
    /// Taxonomy-map version every attribute and threshold was derived against.
    pub taxonomy_version: String,
    /// Digest of the corpus snapshot the pack was built from.
    pub corpus_snapshot_digest: String,
    /// Tokenizer identity, so a detector is never paired with a tokenizer it
    /// was not trained against.
    pub tokenizer: String,
    pub components: Vec<ComponentRecord>,
    /// Calibrated per-label thresholds, carried as the calibration table's own
    /// wire form. Opaque here on purpose: the manifest binds and signs them;
    /// interpreting them is the runtime's job, and a manifest that could
    /// interpret them would need to agree with the runtime's version of the
    /// type forever.
    pub thresholds: serde_json::Value,
    pub lineage: Lineage,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eval_evidence_digest: Option<String>,
}

impl PackManifest {
    /// The pack's own ceiling: the highest any component carries (KP-006).
    pub fn ceiling(&self) -> Classification {
        self.components
            .iter()
            .map(ComponentRecord::effective_ceiling)
            .fold(Classification::Public, Classification::high_water)
    }

    pub fn component(&self, file: &str) -> Option<&ComponentRecord> {
        self.components.iter().find(|c| c.file == file)
    }

    pub fn adapters(&self) -> impl Iterator<Item = &ComponentRecord> {
        self.components
            .iter()
            .filter(|c| matches!(c.role, ComponentRole::Adapter { .. }))
    }

    pub fn role(&self, role: &ComponentRole) -> Option<&ComponentRecord> {
        self.components.iter().find(|c| &c.role == role)
    }

    /// Everything that has to hold before a manifest is worth signing.
    ///
    /// Checked at assembly rather than at load: a pack that fails this was
    /// built wrong, and signing it would put a signature on a contradiction.
    pub fn check(&self) -> Result<(), ManifestError> {
        if self.components.is_empty() {
            return Err(ManifestError::Empty);
        }
        let mut compartments: BTreeSet<&str> = BTreeSet::new();
        let mut singletons: BTreeSet<&str> = BTreeSet::new();
        let mut files: BTreeSet<&str> = BTreeSet::new();
        for component in &self.components {
            if component.digest.is_empty() {
                return Err(ManifestError::MissingDigest(component.file.clone()));
            }
            if !files.insert(component.file.as_str()) {
                return Err(ManifestError::DuplicateFile(component.file.clone()));
            }
            match component.role.compartment() {
                Some(compartment) => {
                    if !compartments.insert(compartment) {
                        return Err(ManifestError::DuplicateCompartment(compartment.to_string()));
                    }
                }
                // A second sentinel or index is not additive: lookup is by
                // role, so one of them would be silently ignored — and which
                // one depends on manifest order, which is not a security
                // property anybody should be relying on.
                None => {
                    if !singletons.insert(component.role.as_str()) {
                        return Err(ManifestError::DuplicateRole(
                            component.role.as_str().to_string(),
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    /// Serialize to the exact bytes that get signed and written.
    ///
    /// Pretty-printed and stable: the signature covers these bytes, so the
    /// only safe reading of a manifest is the one that reads the file rather
    /// than re-serializing a parsed struct.
    ///
    /// # Panics
    ///
    /// If a field cannot be serialized. Signing a fallback empty buffer would
    /// produce a pack whose signature verifies over the wrong bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes =
            serde_json::to_vec_pretty(self).expect("PackManifest fields are all serializable");
        bytes.push(b'\n');
        bytes
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ManifestError> {
        let manifest: Self =
            serde_json::from_slice(bytes).map_err(|e| ManifestError::Malformed(e.to_string()))?;
        if manifest.format_version != PACK_FORMAT_VERSION {
            return Err(ManifestError::UnsupportedVersion(manifest.format_version));
        }
        Ok(manifest)
    }
}
