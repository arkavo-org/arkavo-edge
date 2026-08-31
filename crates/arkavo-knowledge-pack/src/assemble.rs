//! Building a pack (KP-001, KP-002, KP-006).
//!
//! Assembly is where the invariants are enforced, because it is the last moment
//! at which they can be. Once a manifest is signed, a contradiction inside it
//! is a signed contradiction: two adapters claiming one compartment, a
//! component with no digest, a ceiling lower than the corpus it came from. The
//! builder refuses those rather than recording them.

use std::path::{Path, PathBuf};

use arkavo_crypto::AgentKeypair;
use arkavo_gguf_tdf::{Classification, ComponentRole};

use crate::manifest::{
    ComponentRecord, Lineage, ManifestError, PACK_FORMAT_VERSION, PACK_MANIFEST_FILE,
    PACK_SIGNATURE_FILE, PackManifest,
};
use crate::sign::{encode_signature, sign_manifest};

#[derive(Debug, thiserror::Error)]
pub enum AssembleError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("cannot read component {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot write {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

/// Accumulates components and produces a signed pack.
pub struct PackBuilder {
    manifest: PackManifest,
    /// Source paths, parallel to `manifest.components`, copied at build time.
    sources: Vec<PathBuf>,
}

impl PackBuilder {
    pub fn new(
        pack_id: impl Into<String>,
        taxonomy_version: impl Into<String>,
        tokenizer: impl Into<String>,
    ) -> Self {
        Self {
            manifest: PackManifest {
                format_version: PACK_FORMAT_VERSION.to_string(),
                pack_id: pack_id.into(),
                taxonomy_version: taxonomy_version.into(),
                corpus_snapshot_digest: String::new(),
                tokenizer: tokenizer.into(),
                components: Vec::new(),
                thresholds: serde_json::Value::Null,
                lineage: Lineage::Root,
                eval_evidence_digest: None,
            },
            sources: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_corpus_digest(mut self, digest: impl Into<String>) -> Self {
        self.manifest.corpus_snapshot_digest = digest.into();
        self
    }

    #[must_use]
    pub fn with_thresholds(mut self, thresholds: serde_json::Value) -> Self {
        self.manifest.thresholds = thresholds;
        self
    }

    #[must_use]
    pub fn with_lineage(mut self, lineage: Lineage) -> Self {
        self.manifest.lineage = lineage;
        self
    }

    #[must_use]
    pub fn with_eval_evidence(mut self, digest: impl Into<String>) -> Self {
        self.manifest.eval_evidence_digest = Some(digest.into());
        self
    }

    /// Add a component from a file, digesting its bytes as written.
    pub fn add_component(
        &mut self,
        source: &Path,
        role: ComponentRole,
        ceiling: Option<Classification>,
    ) -> Result<(), AssembleError> {
        let bytes = std::fs::read(source).map_err(|e| AssembleError::Read {
            path: source.to_path_buf(),
            source: e,
        })?;
        let file = source
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "component".to_string());
        let mut record = ComponentRecord::new(file, role, &bytes);
        record.classification_ceiling = ceiling;
        self.manifest.components.push(record);
        self.sources.push(source.to_path_buf());
        Ok(())
    }

    /// The ceiling this pack would carry: the high-water mark of its parts.
    pub fn ceiling(&self) -> Classification {
        self.manifest.ceiling()
    }

    /// Write the pack, signing the manifest bytes exactly as written.
    pub fn build(self, dest: &Path, key: &AgentKeypair) -> Result<PackManifest, AssembleError> {
        self.manifest.check()?;
        std::fs::create_dir_all(dest).map_err(|e| AssembleError::Write {
            path: dest.to_path_buf(),
            source: e,
        })?;

        for (record, source) in self.manifest.components.iter().zip(&self.sources) {
            let target = dest.join(&record.file);
            if target != *source {
                std::fs::copy(source, &target).map_err(|e| AssembleError::Write {
                    path: target,
                    source: e,
                })?;
            }
        }

        // Signed after every component is in place, so a signature never
        // describes a pack that was not fully written.
        let bytes = self.manifest.to_bytes();
        let signature = sign_manifest(&bytes, key);
        write(&dest.join(PACK_MANIFEST_FILE), &bytes)?;
        write(
            &dest.join(PACK_SIGNATURE_FILE),
            encode_signature(&signature).as_bytes(),
        )?;
        Ok(self.manifest)
    }
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), AssembleError> {
    std::fs::write(path, bytes).map_err(|e| AssembleError::Write {
        path: path.to_path_buf(),
        source: e,
    })
}
