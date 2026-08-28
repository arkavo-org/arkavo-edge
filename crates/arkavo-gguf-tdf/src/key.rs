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
