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

/// Attribute FQNs the embedded policy requires.
///
/// Plaintext on the manifest by design: a reader has to be able to see what a
/// component demands before deciding whether to ask for its key.
pub fn embedded_attributes(manifest: &TdfManifest) -> Result<Vec<String>, GgufTdfError> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&manifest.encryption_information.policy)
        .map_err(|e| GgufTdfError::BadIndex(format!("embedded policy is not base64: {e}")))?;
    let policy: serde_json::Value = serde_json::from_slice(&raw)
        .map_err(|e| GgufTdfError::BadIndex(format!("embedded policy is not JSON: {e}")))?;
    Ok(policy
        .get("body")
        .and_then(|b| b.get("dataAttributes"))
        .and_then(|a| a.as_array())
        .map(|attributes| {
            attributes
                .iter()
                .filter_map(|a| a.get("attribute").and_then(|v| v.as_str()))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}

/// Recover the plaintext, checking the embedded policy before asking for a key.
pub fn open_blob(
    blob: &SealedBlob,
    unwrapper: &dyn PayloadKeyUnwrapper,
) -> Result<Zeroizing<Vec<u8>>, GgufTdfError> {
    open_blob_requiring(blob, unwrapper, &[])
}

/// Recover the plaintext, refusing before any key request if the component was
/// not wrapped under the attributes the caller expects (KP-003).
///
/// This is a pre-flight check, not an authorization decision — the KAS still
/// decides whether this node may have the key. It exists to separate "this
/// component is not what I was told it was" from "the KAS denied me", and to
/// make the refusal happen before a round-trip rather than after one. A
/// component whose policy is missing an expected attribute is one anybody
/// entitled to less could open, which is a misconfiguration worth catching at
/// the reader rather than trusting the wrapper to have got right.
pub fn open_blob_requiring(
    blob: &SealedBlob,
    unwrapper: &dyn PayloadKeyUnwrapper,
    required: &[String],
) -> Result<Zeroizing<Vec<u8>>, GgufTdfError> {
    if !required.is_empty() {
        let found = embedded_attributes(&blob.manifest)?;
        let missing: Vec<&String> = required.iter().filter(|r| !found.contains(r)).collect();
        if !missing.is_empty() {
            return Err(GgufTdfError::KasDenied(format!(
                "component policy is missing {missing:?}; refusing before a key request"
            )));
        }
    }
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
