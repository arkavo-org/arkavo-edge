//! Wrapping a pack component that is not a GGUF (KP-001, KP-009).
//!
//! The reference index is a component like any other and must be wrapped like
//! one — a keyed index left in the clear is still a list of what the corpus
//! contains, keyed or not, and its labels say how sensitive each entry is.
//!
//! `gguf-tdf` cannot carry it: that profile's whole point is random access into
//! a virtual GGUF, which an index is not. So this is the small case — one
//! segment, one key — expressed with the same primitives and, more importantly,
//! the same [`PayloadKeyWrapper`] and [`PayloadKeyUnwrapper`] indirection, so
//! key release goes through the KAS in production and through a pre-resolved
//! key in a test without either path being special.

use arkavo_gguf_tdf::{GgufTdfError, PayloadKeyUnwrapper, PayloadKeyWrapper};
use base64::Engine as _;
use opentdf::manifest::KeyAccessExt;
use opentdf::{TdfEncryption, TdfManifest};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

/// Largest component this envelope will open.
///
/// The ciphertext is decrypted into one buffer, so this is a real allocation
/// bound and the archive is attacker-supplied. 256 MiB is far past any index
/// and far short of a denial of service.
pub const MAX_BLOB_BYTES: usize = 256 << 20;

/// A single-segment wrapped component.
#[derive(Debug, Serialize, Deserialize)]
pub struct SealedBlob {
    pub manifest: TdfManifest,
    /// Base64 of the one encrypted segment, IV and tag included.
    pub ciphertext: String,
}

/// Wrap `plaintext` under `attributes`, releasing the key through `wrapper`.
pub fn seal_blob(
    plaintext: &[u8],
    wrapper: &dyn PayloadKeyWrapper,
    attributes: &[String],
    mime_type: &str,
) -> Result<SealedBlob, GgufTdfError> {
    let mut payload_key = Zeroizing::new([0u8; 32]);
    use rand::RngCore as _;
    rand::thread_rng().fill_bytes(payload_key.as_mut());

    let encryption = TdfEncryption::with_payload_key(payload_key.as_ref())
        .map_err(|e| GgufTdfError::Crypto(format!("payload key rejected: {e}")))?;
    let encrypted = encryption
        .encrypt_segment(plaintext)
        .map_err(|e| GgufTdfError::Crypto(format!("segment encrypt failed: {e}")))?;

    let wrapped = wrapper.wrap(&payload_key)?;
    let mut manifest = TdfManifest::new("0.payload".to_string(), wrapped.kas_url.clone());
    manifest.payload.mime_type = Some(mime_type.to_string());

    let policy_json = policy_document(attributes)?;
    manifest.set_policy_raw(&policy_json);

    let key_access = manifest
        .encryption_information
        .key_access
        .first_mut()
        .ok_or_else(|| GgufTdfError::BadIndex("manifest has no keyAccess entry".to_string()))?;
    key_access.wrapped_key.clone_from(&wrapped.wrapped_key);
    key_access.kid.clone_from(&wrapped.kid);
    key_access
        .generate_policy_binding_raw(&policy_json, payload_key.as_ref())
        .map_err(GgufTdfError::BadIndex)?;

    Ok(SealedBlob {
        manifest,
        ciphertext: base64::engine::general_purpose::STANDARD.encode(&encrypted.bytes),
    })
}

/// Recover the plaintext, evaluating attributes before the key is released.
///
/// The key comes back from `unwrapper`, which is where the KAS evaluates the
/// policy embedded in the manifest. Nothing here decides entitlement; if the
/// key does not arrive, nothing is decrypted.
pub fn open_blob(
    blob: &SealedBlob,
    unwrapper: &dyn PayloadKeyUnwrapper,
) -> Result<Zeroizing<Vec<u8>>, GgufTdfError> {
    let ciphertext = base64::engine::general_purpose::STANDARD
        .decode(&blob.ciphertext)
        .map_err(|e| GgufTdfError::BadIndex(format!("ciphertext is not base64: {e}")))?;
    // AES-GCM adds a 12-byte IV and a 16-byte tag, so the plaintext is shorter
    // than the ciphertext; checking the larger number is the conservative side.
    if ciphertext.len() > MAX_BLOB_BYTES {
        return Err(GgufTdfError::BadIndex(format!(
            "component is {} bytes, over the {MAX_BLOB_BYTES} byte cap",
            ciphertext.len()
        )));
    }

    let payload_key = Zeroizing::new(unwrapper.unwrap_key(&blob.manifest)?);
    let encryption = TdfEncryption::with_payload_key(payload_key.as_ref())
        .map_err(|e| GgufTdfError::Crypto(format!("payload key rejected: {e}")))?;

    let plain_len = ciphertext.len().checked_sub(28).ok_or_else(|| {
        GgufTdfError::BadIndex("ciphertext is shorter than its own IV and tag".into())
    })?;
    let mut plaintext = Zeroizing::new(vec![0u8; plain_len]);
    encryption
        .decrypt_segment_into(&ciphertext, &mut plaintext)
        .map_err(|e| GgufTdfError::Crypto(format!("segment decrypt failed: {e}")))?;
    Ok(plaintext)
}

fn policy_document(attributes: &[String]) -> Result<String, GgufTdfError> {
    let policy = serde_json::json!({
        "uuid": uuid::Uuid::new_v4().to_string(),
        "body": {
            "dataAttributes": attributes
                .iter()
                .map(|a| serde_json::json!({ "attribute": a }))
                .collect::<Vec<_>>(),
            "dissem": Vec::<String>::new(),
        }
    });
    serde_json::to_string(&policy)
        .map_err(|e| GgufTdfError::BadIndex(format!("cannot serialize policy: {e}")))
}
