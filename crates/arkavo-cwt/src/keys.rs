//! The published COSE key set and its cache.
//!
//! `/.well-known/cose-keys` serves a CBOR array of COSE_Key maps (not JWKS
//! JSON). Each entry carries the raw 32-byte RFC 7638 thumbprint as its `kid`,
//! which is the same value the CWT's protected header names; matching is plain
//! byte equality, never a recomputed thumbprint.

use crate::{Claims, CwtError, VerifyOptions, verify::verify};
use coset::{CborSerializable, CoseKey, CoseKeySet, Label};
use p256::EncodedPoint;
use p256::ecdsa::VerifyingKey;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

const EC2_CRV: i64 = -1;
const EC2_X: i64 = -2;
const EC2_Y: i64 = -3;

/// The ES256 verification keys the issuer currently publishes.
pub struct KeySet {
    keys: Vec<(Vec<u8>, VerifyingKey)>,
}

impl KeySet {
    /// Parse a `/.well-known/cose-keys` body.
    ///
    /// Entries that are not EC2 P-256 keys, or that carry no `kid`, are skipped
    /// rather than rejected: the issuer may publish keys for algorithms this
    /// verifier does not handle, and one such entry must not blind it to the
    /// ES256 keys alongside it.
    pub fn from_cbor(bytes: &[u8]) -> Result<Self, CwtError> {
        let set = CoseKeySet::from_slice(bytes).map_err(|e| CwtError::KeySet(e.to_string()))?;
        let mut keys = Vec::with_capacity(set.0.len());
        for key in &set.0 {
            if key.key_id.is_empty() || !is_p256(key) {
                continue;
            }
            keys.push((key.key_id.clone(), verifying_key(key)?));
        }
        if keys.is_empty() {
            return Err(CwtError::KeySet("no usable ES256 P-256 keys".into()));
        }
        Ok(Self { keys })
    }

    /// Fetch and parse the key set published at `url`.
    pub async fn fetch(url: &str) -> Result<Self, CwtError> {
        let body = reqwest::get(url)
            .await
            .map_err(|e| CwtError::Fetch(e.to_string()))?
            .error_for_status()
            .map_err(|e| CwtError::Fetch(e.to_string()))?
            .bytes()
            .await
            .map_err(|e| CwtError::Fetch(e.to_string()))?;
        Self::from_cbor(&body)
    }

    pub(crate) fn get(&self, kid: &[u8]) -> Option<&VerifyingKey> {
        self.keys
            .iter()
            .find(|(candidate, _)| candidate == kid)
            .map(|(_, key)| key)
    }
}

fn is_p256(key: &CoseKey) -> bool {
    key.kty == coset::KeyType::Assigned(coset::iana::KeyType::EC2)
        && param(key, EC2_CRV).and_then(ciborium::Value::as_integer)
            == Some((coset::iana::EllipticCurve::P_256 as i64).into())
}

fn param(key: &CoseKey, label: i64) -> Option<&ciborium::Value> {
    key.params
        .iter()
        .find(|(candidate, _)| *candidate == Label::Int(label))
        .map(|(_, value)| value)
}

fn verifying_key(key: &CoseKey) -> Result<VerifyingKey, CwtError> {
    let coordinate = |label: i64, name: &str| {
        param(key, label)
            .and_then(ciborium::Value::as_bytes)
            .ok_or_else(|| CwtError::KeySet(format!("COSE key is missing its {name} coordinate")))
    };
    let point = EncodedPoint::from_affine_coordinates(
        coordinate(EC2_X, "x")?.as_slice().into(),
        coordinate(EC2_Y, "y")?.as_slice().into(),
        false,
    );
    VerifyingKey::from_encoded_point(&point)
        .map_err(|e| CwtError::KeySet(format!("COSE key is not a valid P-256 point: {e}")))
}

struct Cached {
    keys: Arc<KeySet>,
    fetched_at: Instant,
}

/// A [`KeySet`] that re-fetches when a token names a `kid` it does not hold,
/// but no more than once per `ttl` — so a token signed by a key that will never
/// be published cannot turn into a fetch per verification.
pub struct CachedKeySet {
    url: String,
    ttl: Duration,
    cached: RwLock<Option<Cached>>,
}

impl CachedKeySet {
    pub fn new(url: impl Into<String>, ttl: Duration) -> Self {
        Self {
            url: url.into(),
            ttl,
            cached: RwLock::new(None),
        }
    }

    /// Verify `token` against the cached key set, refreshing once if the token's
    /// `kid` is unknown and the cache is at least `ttl` old.
    pub async fn verify(&self, token: &str, opts: &VerifyOptions<'_>) -> Result<Claims, CwtError> {
        let current = self.snapshot();
        let (keys, fetched_at) = match current {
            Some(cached) => cached,
            None => (self.refresh().await?, Instant::now()),
        };

        match verify(token, &keys, opts) {
            Err(CwtError::UnknownKid(kid)) => {
                if fetched_at.elapsed() < self.ttl {
                    return Err(CwtError::UnknownKid(kid));
                }
                let refreshed = self.refresh().await?;
                verify(token, &refreshed, opts)
            }
            other => other,
        }
    }

    fn snapshot(&self) -> Option<(Arc<KeySet>, Instant)> {
        let guard = self
            .cached
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .as_ref()
            .map(|cached| (cached.keys.clone(), cached.fetched_at))
    }

    async fn refresh(&self) -> Result<Arc<KeySet>, CwtError> {
        let keys = Arc::new(KeySet::fetch(&self.url).await?);
        let mut guard = self
            .cached
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *guard = Some(Cached {
            keys: keys.clone(),
            fetched_at: Instant::now(),
        });
        Ok(keys)
    }
}
