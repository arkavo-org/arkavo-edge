//! Loading a KAS-protected `.gguf.tdf` model through the callback loader.
//!
//! This module owns path detection and the bridge from a decrypted virtual
//! GGUF to `LlamaModel::from_callback`. It deliberately contains no KAS,
//! OAuth, or TDF-transport code: the caller performs the rewrap in the
//! runtime it already owns and passes the resulting payload key in.

use crate::{Error, Result};
use arkavo_gguf_tdf::{GgufTdfArchive, PreResolvedKey};
use arkavo_llama_cpp::LlamaModel;
use std::path::Path;

/// File extension identifying a protected model.
pub const PROTECTED_EXTENSION: &str = ".gguf.tdf";

/// Whether `path` names a protected model artifact.
pub fn is_protected_model_path(path: &str) -> bool {
    path.to_lowercase().ends_with(PROTECTED_EXTENSION)
}

/// Whether `path` is an mmproj sidecar, which has no callback-capable load.
fn is_mmproj(path: &str) -> bool {
    Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.to_lowercase().starts_with("mmproj-"))
}

/// Loads a protected model with an already-recovered payload key.
///
/// The archive is opened and structurally validated, the header is decrypted
/// and bound to the plaintext index, and the root signature is checked before
/// any weight byte is served. llama.cpp then reads the virtual GGUF through a
/// callback and never sees the zip, the ciphertext, or the key.
///
/// No plaintext GGUF is written to disk or to an anonymous whole-file mapping.
pub fn load_with_payload_key(model_path: &str, payload_key: [u8; 32]) -> Result<LlamaModel> {
    if is_mmproj(model_path) {
        return Err(Error::Config(format!(
            "GGUFTDF_MTMD_UNSUPPORTED: {model_path} is an mmproj sidecar; \
             multimodal projectors need a callback-capable mtmd API"
        )));
    }

    let archive = GgufTdfArchive::open(Path::new(model_path))
        .map_err(|e| Error::Config(format!("Failed to open protected model {model_path}: {e}")))?;
    let virtual_size = archive.virtual_size();

    let mut virtual_gguf = archive
        .unlock(&PreResolvedKey::new(payload_key))
        .map_err(|e| Error::Config(format!("Failed to unlock {model_path}: {e}")))?;

    // `from_callback` does not retain the closure, so the borrow ends when the
    // load returns and the decrypted state drops with `virtual_gguf`.
    let model = LlamaModel::from_callback(virtual_size, |offset, buf| {
        virtual_gguf.read_at(offset, buf)
    });

    match model {
        Ok(model) => Ok(model),
        Err(load_error) => {
            // Surface the profile's own error when the load stopped because a
            // segment failed to authenticate, rather than the generic
            // "failed to load model" the executor reports.
            if let Some(cause) = virtual_gguf.error() {
                return Err(Error::Config(format!(
                    "Failed to load protected model {model_path}: {cause}"
                )));
            }
            Err(Error::Config(format!(
                "Failed to load protected model {model_path}: {load_error}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `expect_err` needs `Debug` on the success type; `LlamaModel` wraps a
    /// raw pointer and does not implement it.
    fn expect_err(result: Result<LlamaModel>, what: &str) -> Error {
        match result {
            Ok(_) => panic!("{what}"),
            Err(err) => err,
        }
    }

    #[test]
    fn detects_protected_paths_case_insensitively() {
        assert!(is_protected_model_path("model.gguf.tdf"));
        assert!(is_protected_model_path("/tmp/Model.GGUF.TDF"));
        assert!(!is_protected_model_path("model.gguf"));
        assert!(!is_protected_model_path("model.tdf"));
        assert!(!is_protected_model_path("notes.txt"));
    }

    #[test]
    fn mmproj_sidecars_are_refused_until_mtmd_takes_a_callback() {
        let err = expect_err(
            load_with_payload_key("/tmp/mmproj-vision.gguf.tdf", [0u8; 32]),
            "mmproj must be refused",
        );
        assert!(
            err.to_string().contains("GGUFTDF_MTMD_UNSUPPORTED"),
            "got: {err}"
        );
    }

    #[test]
    fn a_missing_archive_fails_closed() {
        let err = expect_err(
            load_with_payload_key("/nonexistent/model.gguf.tdf", [0u8; 32]),
            "a missing archive must not load",
        );
        assert!(err.to_string().contains("Failed to open"), "got: {err}");
    }
}
