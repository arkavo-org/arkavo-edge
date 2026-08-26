//! Union CWT verifier (draft-arkavo-authzen-cwt-00): stricter of authnz-rs and catalog.

use crate::cwt_decode::parse_claims;
use crate::cwt_subject::DecodedClaims;
use crate::error::AuthorizationError;
use ciborium::value::Value;
use coset::{AsCborValue, CborSerializable};
use p256::ecdsa::signature::Verifier;
use p256::ecdsa::{Signature, VerifyingKey};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::info;

const CWT_TAG_PREFIX: [u8; 2] = [0xD8, 0x3D];
const SKEW_SECS: i64 = 60;
const KEY_REFRESH_MIN_INTERVAL: Duration = Duration::from_secs(60);
const KEY_FETCH_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, thiserror::Error)]
pub enum CwtError {
    #[error("malformed token")]
    Malformed,
    #[error("unsupported algorithm (ES256 required)")]
    Algorithm,
    #[error("unknown key id")]
    UnknownKid,
    #[error("signature verification failed")]
    Signature,
    #[error("token expired")]
    Expired,
    #[error("token not yet valid")]
    NotYetValid,
    #[error("required claim missing: {0}")]
    MissingClaim(&'static str),
    #[error("issuer mismatch")]
    Issuer,
    #[error("audience mismatch")]
    Audience,
    #[error("duplicate claim key")]
    DuplicateKey,
    #[error("key set unavailable: {0}")]
    KeySet(String),
}

impl From<CwtError> for AuthorizationError {
    fn from(e: CwtError) -> Self {
        AuthorizationError::InvalidToken(e.to_string())
    }
}

struct KeyCache {
    keys: HashMap<Vec<u8>, VerifyingKey>,
    last_fetch: Option<Instant>,
}

pub struct CwtVerifier {
    cose_keys_url: Option<String>,
    expected_iss: Option<String>,
    expected_audiences: Vec<String>,
    http: reqwest::Client,
    cache: RwLock<KeyCache>,
}

fn bounded_http_client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(KEY_FETCH_TIMEOUT)
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

impl CwtVerifier {
    pub fn new(cose_keys_url: String, expected_iss: Option<String>) -> Self {
        Self {
            cose_keys_url: Some(cose_keys_url),
            expected_iss,
            expected_audiences: Vec::new(),
            http: bounded_http_client(),
            cache: RwLock::new(KeyCache {
                keys: HashMap::new(),
                last_fetch: None,
            }),
        }
    }

    pub fn with_static_keys(keys: Vec<(Vec<u8>, VerifyingKey)>) -> Self {
        Self {
            cose_keys_url: None,
            expected_iss: None,
            expected_audiences: Vec::new(),
            http: bounded_http_client(),
            cache: RwLock::new(KeyCache {
                keys: keys.into_iter().collect(),
                last_fetch: None,
            }),
        }
    }

    #[must_use]
    pub fn with_expected_issuer(mut self, iss: String) -> Self {
        self.expected_iss = Some(iss);
        self
    }

    #[must_use]
    pub fn with_audiences(mut self, audiences: Vec<String>) -> Self {
        self.expected_audiences = audiences;
        self
    }

    pub async fn verify(&self, token_b64: &str) -> Result<DecodedClaims, CwtError> {
        self.verify_at(token_b64, chrono::Utc::now().timestamp())
            .await
    }

    pub async fn verify_at(&self, token_b64: &str, now: i64) -> Result<DecodedClaims, CwtError> {
        use base64::Engine;
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(token_b64.trim())
            .map_err(|_| CwtError::Malformed)?;
        let inner = bytes
            .strip_prefix(&CWT_TAG_PREFIX[..])
            .ok_or(CwtError::Malformed)?;
        let sign1 = coset::CoseSign1::from_slice(inner).map_err(|_| CwtError::Malformed)?;

        match sign1.protected.header.alg {
            Some(coset::Algorithm::Assigned(coset::iana::Algorithm::ES256)) => {}
            _ => return Err(CwtError::Algorithm),
        }
        let kid = sign1.protected.header.key_id.clone();
        if kid.is_empty() {
            return Err(CwtError::UnknownKid);
        }

        let key = match self.lookup(&kid).await {
            Some(k) => k,
            None => {
                self.refresh_keys().await?;
                self.lookup(&kid).await.ok_or(CwtError::UnknownKid)?
            }
        };

        sign1
            .verify_signature(b"", |sig, data| {
                let sig = Signature::from_slice(sig).map_err(|_| ())?;
                key.verify(data, &sig).map_err(|_| ())
            })
            .map_err(|_| CwtError::Signature)?;

        let payload = sign1.payload.as_deref().ok_or(CwtError::Malformed)?;
        let claims = parse_claims(payload)?;

        if claims.iat > claims.exp {
            return Err(CwtError::Malformed);
        }
        let exp = i64::try_from(claims.exp).map_err(|_| CwtError::Malformed)?;
        let iat = i64::try_from(claims.iat).map_err(|_| CwtError::Malformed)?;
        if exp <= now - SKEW_SECS {
            return Err(CwtError::Expired);
        }
        if iat > now + SKEW_SECS {
            return Err(CwtError::NotYetValid);
        }
        if let Some(expected) = &self.expected_iss
            && &claims.iss != expected
        {
            return Err(CwtError::Issuer);
        }
        if !self.expected_audiences.is_empty() {
            let ok = self
                .expected_audiences
                .iter()
                .any(|a| crate::cwt_subject::aud_contains(&claims.aud, a));
            if !ok {
                return Err(CwtError::Audience);
            }
        }
        Ok(claims)
    }

    async fn lookup(&self, kid: &[u8]) -> Option<VerifyingKey> {
        self.cache.read().await.keys.get(kid).copied()
    }

    async fn refresh_keys(&self) -> Result<(), CwtError> {
        let Some(url) = &self.cose_keys_url else {
            return Ok(());
        };
        {
            let mut cache = self.cache.write().await;
            if let Some(last) = cache.last_fetch
                && last.elapsed() < KEY_REFRESH_MIN_INTERVAL
            {
                return Ok(());
            }
            cache.last_fetch = Some(Instant::now());
        }
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| CwtError::KeySet(format!("GET {url}: {e}")))?;
        if !resp.status().is_success() {
            return Err(CwtError::KeySet(format!(
                "GET {url}: HTTP {}",
                resp.status()
            )));
        }
        let body = resp
            .bytes()
            .await
            .map_err(|e| CwtError::KeySet(format!("read body: {e}")))?;
        let keys = parse_cose_key_set(&body)?;
        info!(count = keys.len(), "Refreshed COSE key set from IdP");
        self.cache.write().await.keys = keys.into_iter().collect();
        Ok(())
    }
}

fn parse_cose_key_set(bytes: &[u8]) -> Result<Vec<(Vec<u8>, VerifyingKey)>, CwtError> {
    let value: Value = ciborium::de::from_reader(bytes).map_err(|_| CwtError::Malformed)?;
    let Value::Array(entries) = value else {
        return Err(CwtError::KeySet("key set is not a CBOR array".into()));
    };
    let mut out = Vec::new();
    for entry in entries {
        let Ok(key) = coset::CoseKey::from_cbor_value(entry) else {
            continue;
        };
        if key.key_id.is_empty() {
            continue;
        }
        if let Ok(vk) = p256_from_cose_key(&key) {
            out.push((key.key_id.clone(), vk));
        }
    }
    if out.is_empty() {
        return Err(CwtError::KeySet("no usable P-256 keys in key set".into()));
    }
    Ok(out)
}

fn p256_from_cose_key(key: &coset::CoseKey) -> Result<VerifyingKey, CwtError> {
    use coset::iana::{Ec2KeyParameter, EnumI64};
    if key.kty != coset::KeyType::Assigned(coset::iana::KeyType::EC2) {
        return Err(CwtError::Malformed);
    }
    let mut x = None;
    let mut y = None;
    let mut crv_ok = false;
    for (label, value) in &key.params {
        match label {
            coset::Label::Int(l) if *l == Ec2KeyParameter::Crv as i64 => {
                crv_ok = matches!(
                    value,
                    Value::Integer(i)
                        if i128::from(*i) == i128::from(coset::iana::EllipticCurve::P_256.to_i64())
                );
            }
            coset::Label::Int(l) if *l == Ec2KeyParameter::X as i64 => {
                if let Value::Bytes(b) = value {
                    x = Some(b.as_slice());
                }
            }
            coset::Label::Int(l) if *l == Ec2KeyParameter::Y as i64 => {
                if let Value::Bytes(b) = value {
                    y = Some(b.as_slice());
                }
            }
            _ => {}
        }
    }
    let (x, y) = (x.ok_or(CwtError::Malformed)?, y.ok_or(CwtError::Malformed)?);
    if !crv_ok || x.len() != 32 || y.len() != 32 {
        return Err(CwtError::Malformed);
    }
    let mut sec1 = Vec::with_capacity(65);
    sec1.push(0x04);
    sec1.extend_from_slice(x);
    sec1.extend_from_slice(y);
    VerifyingKey::from_sec1_bytes(&sec1).map_err(|_| CwtError::Malformed)
}

#[cfg(test)]
#[allow(unreachable_pub)]
pub(crate) mod test_support {
    use super::*;
    use crate::cwt_subject::Aud;
    use coset::{CoseSign1Builder, HeaderBuilder, iana};
    use p256::ecdsa::{SigningKey, signature::Signer};

    #[allow(clippy::too_many_arguments)]
    pub fn mint(
        key: &SigningKey,
        kid: &[u8],
        iss: &str,
        sub: &str,
        aud: Aud,
        iat: i64,
        exp: i64,
        extras: &[(&str, Value)],
        tagged: bool,
        with_alg: bool,
        with_kid: bool,
    ) -> String {
        use base64::Engine;
        let mut entries: Vec<(Value, Value)> = vec![
            (Value::Integer(1.into()), Value::Text(iss.into())),
            (Value::Integer(2.into()), Value::Text(sub.into())),
            (
                Value::Integer(3.into()),
                match &aud {
                    Aud::One(s) => Value::Text(s.clone()),
                    Aud::Many(v) => {
                        Value::Array(v.iter().map(|s| Value::Text(s.clone())).collect())
                    }
                },
            ),
            (Value::Integer(4.into()), Value::Integer(exp.into())),
            (Value::Integer(6.into()), Value::Integer(iat.into())),
            (Value::Integer(7.into()), Value::Bytes(vec![0u8; 16])),
        ];
        for (k, v) in extras {
            entries.push((Value::Text((*k).into()), v.clone()));
        }
        let mut payload = Vec::new();
        ciborium::ser::into_writer(&Value::Map(entries), &mut payload).unwrap();

        let mut hb = HeaderBuilder::new();
        if with_alg {
            hb = hb.algorithm(iana::Algorithm::ES256);
        }
        if with_kid {
            hb = hb.key_id(kid.to_vec());
        }
        let sign1 = CoseSign1Builder::new()
            .protected(hb.build())
            .payload(payload)
            .create_signature(b"", |to_sign| {
                let sig: Signature = key.sign(to_sign);
                sig.to_bytes().to_vec()
            })
            .build();
        let inner = sign1.to_vec().unwrap();
        let mut out = Vec::new();
        if tagged {
            out.extend_from_slice(&CWT_TAG_PREFIX);
        }
        out.extend_from_slice(&inner);
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(out)
    }

    pub fn keypair() -> (SigningKey, VerifyingKey) {
        let sk = SigningKey::from_slice(&[0x17; 32]).expect("valid scalar");
        let vk = *sk.verifying_key();
        (sk, vk)
    }

    pub fn pe_token(sk: &SigningKey, kid: &[u8], now: i64) -> String {
        mint(
            sk,
            kid,
            "https://identity.arkavo.net",
            "arkavo:550e8400-e29b-41d4-a716-446655440000",
            Aud::One("https://mcp.arkavo.net".into()),
            now,
            now + 3600,
            &[],
            true,
            true,
            true,
        )
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::test_support::{keypair, mint, pe_token};
    use super::*;
    use crate::cwt_subject::{Aud, token_map};

    const NOW: i64 = 1_900_000_000;

    fn verifier(kid: &[u8], vk: VerifyingKey) -> CwtVerifier {
        CwtVerifier::with_static_keys(vec![(kid.to_vec(), vk)])
            .with_expected_issuer("https://identity.arkavo.net".into())
            .with_audiences(vec!["https://mcp.arkavo.net".into(), "arkavo".into()])
    }

    #[tokio::test]
    async fn verify_roundtrip_omits_cnf() {
        let (sk, vk) = keypair();
        let token = pe_token(&sk, b"kid-1", NOW);
        let claims = verifier(b"kid-1", vk).verify_at(&token, NOW).await.unwrap();
        assert_eq!(claims.sub, "arkavo:550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(token_map(&claims).get("cnf"), None);
    }

    #[tokio::test]
    async fn rejects_untagged_and_expired() {
        let (sk, vk) = keypair();
        let untagged = mint(
            &sk,
            b"kid-1",
            "https://identity.arkavo.net",
            "s",
            Aud::One("https://mcp.arkavo.net".into()),
            NOW,
            NOW + 10,
            &[],
            false,
            true,
            true,
        );
        assert!(matches!(
            verifier(b"kid-1", vk)
                .verify_at(&untagged, NOW)
                .await
                .unwrap_err(),
            CwtError::Malformed
        ));
        let expired = mint(
            &sk,
            b"kid-1",
            "https://identity.arkavo.net",
            "s",
            Aud::One("https://mcp.arkavo.net".into()),
            NOW - 120,
            NOW - 60,
            &[],
            true,
            true,
            true,
        );
        assert!(matches!(
            verifier(b"kid-1", vk)
                .verify_at(&expired, NOW)
                .await
                .unwrap_err(),
            CwtError::Expired
        ));
    }

    #[tokio::test]
    async fn rejects_missing_alg_and_duplicate_keys() {
        let (sk, vk) = keypair();
        let no_alg = mint(
            &sk,
            b"kid-1",
            "https://identity.arkavo.net",
            "s",
            Aud::One("https://mcp.arkavo.net".into()),
            NOW,
            NOW + 10,
            &[],
            true,
            false,
            true,
        );
        assert!(matches!(
            verifier(b"kid-1", vk)
                .verify_at(&no_alg, NOW)
                .await
                .unwrap_err(),
            CwtError::Algorithm
        ));

        let mut payload = Vec::new();
        let entries = vec![
            (Value::Integer(1.into()), Value::Text("iss".into())),
            (Value::Integer(1.into()), Value::Text("iss2".into())),
        ];
        ciborium::ser::into_writer(&Value::Map(entries), &mut payload).unwrap();
        assert!(matches!(
            crate::cwt_decode::parse_claims(&payload),
            Err(CwtError::DuplicateKey)
        ));
    }
}
