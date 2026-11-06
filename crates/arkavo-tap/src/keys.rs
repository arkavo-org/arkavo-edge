//! Trusted key records and the resolver interface.
//!
//! A key is trusted only as a record from a configured trust authority: who it belongs to,
//! what it may be used for, until when, and where the record came from. A key ID is opaque,
//! and caches key on authority and environment as well, so a reused ID never lets one actor's
//! cached key stand in for another's.

use crate::error::{RejectKind, Rejection};
use crate::profile::Purpose;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};
use std::fmt;
use std::future::Future;

/// The environment a trust authority vouches for. Development roots never vouch for
/// production.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Environment {
    Local,
    Staging,
    Production,
}

impl Environment {
    pub fn name(self) -> &'static str {
        match self {
            Environment::Local => "local",
            Environment::Staging => "staging",
            Environment::Production => "production",
        }
    }

    pub fn parse(name: &str) -> Result<Self, String> {
        match name {
            "local" => Ok(Environment::Local),
            "staging" => Ok(Environment::Staging),
            "production" => Ok(Environment::Production),
            other => Err(format!("unknown environment {other:?}")),
        }
    }
}

/// A trust authority and the environment it speaks for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TrustAuthority {
    pub name: String,
    pub environment: Environment,
}

impl fmt::Display for TrustAuthority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.name, self.environment.name())
    }
}

/// Whether the key may still be used. A revocation is authoritative.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Active,
    Revoked,
}

/// Who the key speaks for, as the trust authority verified it. An agent-supplied name is a
/// claim and never appears here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorBinding {
    /// The agent provider the key is registered to.
    pub provider: String,
    /// The runtime instance, when the authority verified that this key belongs to one.
    pub runtime: Option<String>,
}

impl ActorBinding {
    /// Whether a runtime instance, not only a provider, is identified.
    pub fn runtime_identified(&self) -> bool {
        self.runtime.is_some()
    }
}

/// A key as a trust authority vouches for it.
#[derive(Clone)]
pub struct KeyRecord {
    pub authority: TrustAuthority,
    pub key_id: String,
    pub binding: ActorBinding,
    public_key: VerifyingKey,
    pub status: KeyStatus,
    pub not_before: i64,
    pub not_after: i64,
    /// Purposes the key may declare.
    pub purposes: Vec<Purpose>,
    pub retrieved_at: i64,
    /// After this time the record must be fetched again before it is used.
    pub refresh_by: i64,
    /// Where the record came from, for the decision record.
    pub provenance: String,
}

/// What a trust authority says about one key, before the key bytes are checked.
#[derive(Debug, Clone)]
pub struct KeyFields<'a> {
    pub authority: TrustAuthority,
    pub key_id: String,
    pub binding: ActorBinding,
    pub public_key: &'a [u8],
    pub status: KeyStatus,
    pub not_before: i64,
    pub not_after: i64,
    pub purposes: Vec<Purpose>,
    pub retrieved_at: i64,
    pub refresh_by: i64,
    pub provenance: String,
}

impl fmt::Debug for KeyRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KeyRecord")
            .field("authority", &self.authority)
            .field("key_id", &self.key_id)
            .field("thumbprint", &self.thumbprint())
            .field("binding", &self.binding)
            .field("status", &self.status)
            .field("not_before", &self.not_before)
            .field("not_after", &self.not_after)
            .field("refresh_by", &self.refresh_by)
            .finish_non_exhaustive()
    }
}

impl KeyRecord {
    /// An Ed25519 key record. Only Ed25519 keys are supported.
    pub fn ed25519(fields: KeyFields<'_>) -> Result<Self, String> {
        let bytes: [u8; 32] = fields
            .public_key
            .try_into()
            .map_err(|_| "an Ed25519 public key is 32 bytes".to_owned())?;
        let public_key =
            VerifyingKey::from_bytes(&bytes).map_err(|e| format!("not an Ed25519 key: {e}"))?;
        if public_key.is_weak() {
            return Err("a small-order Ed25519 key is refused".into());
        }
        Ok(KeyRecord {
            authority: fields.authority,
            key_id: fields.key_id,
            binding: fields.binding,
            public_key,
            status: fields.status,
            not_before: fields.not_before,
            not_after: fields.not_after,
            purposes: fields.purposes,
            retrieved_at: fields.retrieved_at,
            refresh_by: fields.refresh_by,
            provenance: fields.provenance,
        })
    }

    /// The RFC 7638 JWK thumbprint of the public key: the identity a replay claim and a
    /// purchase grant are bound to.
    pub fn thumbprint(&self) -> String {
        thumbprint(self.public_key.as_bytes())
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.public_key.to_bytes()
    }

    /// Verifies an Ed25519 signature, refusing non-canonical encodings.
    pub fn verify(&self, message: &[u8], signature: &[u8]) -> Result<(), Rejection> {
        let signature = Signature::from_slice(signature).map_err(|_| {
            Rejection::new(RejectKind::InvalidSignature, "not an Ed25519 signature")
        })?;
        self.public_key
            .verify_strict(message, &signature)
            .map_err(|_| Rejection::new(RejectKind::InvalidSignature, "signature does not verify"))
    }
}

/// The RFC 7638 thumbprint of an Ed25519 public key.
pub fn thumbprint(public_key: &[u8; 32]) -> String {
    let jwk = format!(
        r#"{{"crv":"Ed25519","kty":"OKP","x":"{}"}}"#,
        URL_SAFE_NO_PAD.encode(public_key)
    );
    URL_SAFE_NO_PAD.encode(Sha256::digest(jwk.as_bytes()))
}

/// Why a key could not be resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// The authority has no such key, or the lookup is not one it answers.
    Unknown,
    /// The authority could not be asked, or its answer was unusable: fail closed.
    Unavailable(String),
}

/// Looks keys up through configured trust authorities only. A key URL a caller supplies is
/// never a trust anchor.
pub trait TrustedKeyResolver: Send + Sync {
    fn resolve(
        &self,
        authority: &TrustAuthority,
        key_id: &str,
    ) -> impl Future<Output = Result<KeyRecord, ResolveError>> + Send;
}

/// Lower-case hex of `bytes`.
pub(crate) fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut s, b| {
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Key IDs are opaque, but bounded: they appear in URLs and logs.
pub fn key_id_is_valid(key_id: &str) -> bool {
    (1..=128).contains(&key_id.len())
        && key_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authority() -> TrustAuthority {
        TrustAuthority {
            name: "arkavo-local-demo".into(),
            environment: Environment::Local,
        }
    }

    #[test]
    fn rfc_7638_thumbprint_of_the_rfc_8037_example_key() {
        // RFC 8037 appendix A.3: the thumbprint of its Ed25519 example key.
        let x = URL_SAFE_NO_PAD
            .decode("11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo")
            .unwrap();
        assert_eq!(
            thumbprint(&x.try_into().unwrap()),
            "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k"
        );
    }

    #[test]
    fn records_refuse_bad_keys_and_redact_debug() {
        let binding = ActorBinding {
            provider: "arkavo".into(),
            runtime: None,
        };
        let make = |key: &[u8]| {
            KeyRecord::ed25519(KeyFields {
                authority: authority(),
                key_id: "k1".into(),
                binding: binding.clone(),
                public_key: key,
                status: KeyStatus::Active,
                not_before: 0,
                not_after: 10,
                purposes: vec![Purpose::Browse],
                retrieved_at: 0,
                refresh_by: 5,
                provenance: "test".into(),
            })
        };
        assert!(make(&[0u8; 31]).is_err());
        // The identity point has small order.
        let mut identity = [0u8; 32];
        identity[0] = 1;
        assert!(make(&identity).is_err());
        let key = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let record = make(key.verifying_key().as_bytes()).unwrap();
        assert!(!record.binding.runtime_identified());
        assert_eq!(record.public_key(), key.verifying_key().to_bytes());
        let debug = format!("{record:?}");
        assert!(debug.contains("thumbprint"));
        use ed25519_dalek::Signer as _;
        let sig = key.sign(b"msg").to_bytes();
        record.verify(b"msg", &sig).unwrap();
        assert_eq!(
            record.verify(b"other", &sig).unwrap_err().kind,
            RejectKind::InvalidSignature
        );
        assert!(record.verify(b"msg", &sig[..63]).is_err());
    }

    #[test]
    fn environments_and_key_ids() {
        for e in [
            Environment::Local,
            Environment::Staging,
            Environment::Production,
        ] {
            assert_eq!(Environment::parse(e.name()), Ok(e));
        }
        assert!(Environment::parse("prod").is_err());
        assert_eq!(authority().to_string(), "arkavo-local-demo/local");
        assert!(key_id_is_valid("arkavo-demo-01"));
        assert!(!key_id_is_valid(""));
        assert!(!key_id_is_valid("../keys"));
        assert!(!key_id_is_valid(&"a".repeat(129)));
    }
}
