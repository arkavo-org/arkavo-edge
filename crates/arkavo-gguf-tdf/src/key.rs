//! Payload-key wrap and rewrap boundaries.
//!
//! The profile keeps KAS behind two narrow traits so the packer and the
//! reader can be tested against a mock without a production KAS, and so the
//! crate does not force a KAS client on callers that supply their own.

use crate::error::GgufTdfError;
use opentdf::TdfManifest;

/// A payload key wrapped to a KAS public key.
#[derive(Debug, Clone)]
pub struct WrappedKey {
    /// Absolute KAS base URL, with no trailing `/v1/rewrap`.
    pub kas_url: String,
    /// KAS key id, for rotation. Recorded when the KAS reports one.
    pub kid: Option<String>,
    /// Base64 of the wrapped payload key.
    pub wrapped_key: String,
}

/// Wraps a freshly generated payload key for the archive being written.
pub trait PayloadKeyWrapper {
    /// Wraps `payload_key` (32 bytes) to the KAS public key.
    ///
    /// Implementations must not log, persist, or transmit the plaintext key.
    fn wrap(&self, payload_key: &[u8; 32]) -> Result<WrappedKey, GgufTdfError>;
}

/// Recovers the payload key for an archive being read.
pub trait PayloadKeyUnwrapper {
    /// Performs the OpenTDF KAS rewrap for `manifest` and returns the 32-byte
    /// payload key.
    ///
    /// Every failure — network, authn, authz, policy deny, binding mismatch —
    /// must surface as [`GgufTdfError::KasDenied`] so the caller fails closed
    /// rather than reaching for a sibling plaintext model.
    fn unwrap_key(&self, manifest: &TdfManifest) -> Result<[u8; 32], GgufTdfError>;
}

/// A payload key already recovered from KAS by the caller.
///
/// KAS rewrap is asynchronous and this crate's read path is not, so callers
/// perform the round-trip with their own runtime and hand the result here.
/// The key is zeroized when this value is dropped.
pub struct PreResolvedKey(zeroize::Zeroizing<[u8; 32]>);

impl PreResolvedKey {
    pub fn new(payload_key: [u8; 32]) -> Self {
        Self(zeroize::Zeroizing::new(payload_key))
    }
}

impl PayloadKeyUnwrapper for PreResolvedKey {
    fn unwrap_key(&self, _manifest: &TdfManifest) -> Result<[u8; 32], GgufTdfError> {
        Ok(*self.0)
    }
}

/// Wraps the payload key to a KAS public key with RSA-OAEP, the only wrap
/// this profile defines in v1.
///
/// The caller fetches the KAS public key (an async round-trip) and supplies
/// the PEM plus the key id it came from.
#[cfg(feature = "kas")]
pub struct RsaOaepWrapper {
    kas_url: String,
    kid: Option<String>,
    public_key_pem: String,
}

#[cfg(feature = "kas")]
impl RsaOaepWrapper {
    pub fn new(
        kas_url: impl Into<String>,
        kid: Option<String>,
        public_key_pem: impl Into<String>,
    ) -> Self {
        Self {
            kas_url: kas_url.into(),
            kid,
            public_key_pem: public_key_pem.into(),
        }
    }
}

#[cfg(feature = "kas")]
impl PayloadKeyWrapper for RsaOaepWrapper {
    fn wrap(&self, payload_key: &[u8; 32]) -> Result<WrappedKey, GgufTdfError> {
        let wrapped = opentdf::wrap_key_with_rsa_oaep(payload_key, &self.public_key_pem)
            .map_err(|e| GgufTdfError::KasDenied(format!("cannot wrap to the KAS key: {e}")))?;
        Ok(WrappedKey {
            kas_url: self.kas_url.clone(),
            kid: self.kid.clone(),
            wrapped_key: wrapped,
        })
    }
}
