//! The published COSE key set and its cache.
//!
//! `/.well-known/cose-keys` serves a CBOR array of COSE_Key maps (not JWKS
//! JSON). Each entry carries the raw 32-byte RFC 7638 thumbprint as its `kid`,
//! which is the same value the CWT's protected header names; matching is plain
//! byte equality, never a recomputed thumbprint.

use crate::{Claims, CwtError, VerifyOptions, verify::verify};
use coset::{CborSerializable, CoseKeySet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

/// The ES256 verification keys the issuer currently publishes.
pub struct KeySet {
    keys: Vec<(Vec<u8>, crate::VerifyingKey)>,
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
            if key.key_id.is_empty() {
                continue;
            }
            if let Ok(crate::VerifyingKey::P256(vk)) = crate::VerifyingKey::from_cose_key(key) {
                keys.push((key.key_id.clone(), crate::VerifyingKey::P256(vk)));
            }
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

    pub(crate) fn get(&self, kid: &[u8]) -> Option<&crate::VerifyingKey> {
        self.keys
            .iter()
            .find(|(candidate, _)| candidate == kid)
            .map(|(_, key)| key)
    }
}

struct Cached {
    keys: Arc<KeySet>,
    fetched_at: Instant,
}

/// A [`KeySet`] that re-fetches when a token names a `kid` it does not hold.
///
/// Refetching happens no more than once per `ttl`, so a token signed by a key
/// that will never be published cannot turn into a fetch per verification.
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
        drop(guard);
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coset::CoseKeyBuilder;
    use p256::ecdsa::SigningKey;

    /// A structurally valid COSE_Key whose `x` (or `y`) coordinate is 31 bytes
    /// — the shape a leading-zero-stripped coordinate produces — must be
    /// rejected through the crate's public parse path, not panic the process.
    /// `generic-array`'s `From<&[T]>` panics on this input; the fix routes the
    /// length check through `CwtError::KeySet` instead.
    fn cose_key_with(x: Vec<u8>, y: Vec<u8>) -> Vec<u8> {
        let key = CoseKeyBuilder::new_ec2_pub_key(coset::iana::EllipticCurve::P_256, x, y)
            .algorithm(coset::iana::Algorithm::ES256)
            .key_id(vec![0xAA; 32])
            .build();
        CoseKeySet(vec![key]).to_vec().expect("encode key set")
    }

    fn valid_coordinates() -> (Vec<u8>, Vec<u8>) {
        let vk = *SigningKey::random(&mut rand::rngs::OsRng).verifying_key();
        let point = vk.to_encoded_point(false);
        (
            point.x().expect("x coordinate").to_vec(),
            point.y().expect("y coordinate").to_vec(),
        )
    }

    #[test]
    fn from_cbor_rejects_31_byte_x_coordinate_without_panicking() {
        let (x, y) = valid_coordinates();
        let bytes = cose_key_with(x[..31].to_vec(), y);

        match KeySet::from_cbor(&bytes) {
            Err(CwtError::KeySet(_)) => {}
            Err(other) => panic!("expected CwtError::KeySet, got {other:?}"),
            Ok(_) => panic!("31-byte x must be rejected, not accepted"),
        }
    }

    #[test]
    fn from_cbor_rejects_31_byte_y_coordinate_without_panicking() {
        let (x, y) = valid_coordinates();
        let bytes = cose_key_with(x, y[..31].to_vec());

        match KeySet::from_cbor(&bytes) {
            Err(CwtError::KeySet(_)) => {}
            Err(other) => panic!("expected CwtError::KeySet, got {other:?}"),
            Ok(_) => panic!("31-byte y must be rejected, not accepted"),
        }
    }
}
