//! Loading the classifier through the protected weights path (SENT-005).
//!
//! The DLP model is protected by the mechanism it enforces. That is not
//! symmetry for its own sake: a sentinel is a distillation of the corpus it was
//! trained to recognize, so shipping it in the clear hands an attacker a
//! queryable copy of exactly what the indices are keyed to hide.
//!
//! Everything structural is checked before a key is asked for, the attributes
//! are evaluated by the KAS before the payload key is released, and the
//! plaintext weights are served through a virtual reader that never writes them
//! to disk.

use std::path::Path;

use arkavo_gguf_tdf::{GgufTdfArchive, PayloadKeyUnwrapper, VirtualGguf};

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("the sentinel archive is unusable: {0}")]
    Archive(#[from] arkavo_gguf_tdf::GgufTdfError),
    #[error("not entitled to the sentinel component: {0}")]
    NotEntitled(String),
}

/// Open the sentinel's weights, evaluating entitlement before any key release.
///
/// A node that is not entitled gets [`LoadError::NotEntitled`] rather than a
/// panic or a silent fallback: SENT-005's edge case is that the cascade carries
/// on without this tier and the gap is recorded, which the caller can only do
/// if the refusal comes back as a value.
pub fn open_sentinel(
    path: &Path,
    unwrapper: &dyn PayloadKeyUnwrapper,
) -> Result<VirtualGguf, LoadError> {
    // Structure first: a malformed archive is rejected before a KAS round-trip,
    // so a broken file cannot be used to make the KAS do work.
    let archive = GgufTdfArchive::open(path)?;
    // `unlock` is where the attributes are evaluated and the payload key is
    // released, rewrapped and zeroized. Nothing here holds the key.
    archive.unlock(unwrapper).map_err(|e| match e {
        arkavo_gguf_tdf::GgufTdfError::KasDenied(reason) => LoadError::NotEntitled(reason),
        other => LoadError::Archive(other),
    })
}
