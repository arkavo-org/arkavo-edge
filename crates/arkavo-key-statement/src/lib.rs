//! Key statements: an agent's workload key vouching for a key the agent
//! uses for one job (INGRESS-006).
//!
//! The workload key is who the agent is (#766), but gossip, iroh and TDF each
//! need a key of their own: their protocols fix the algorithm, and the
//! workload key used directly in another protocol could be led to sign
//! something that protocol's peers accept. So the workload key signs a
//! statement naming the key, its job, when it is valid and its generation. A
//! peer accepts the key only through a current statement, and a later
//! generation retires an earlier key without changing who the agent is.
//!
//! A statement is a compact JWS (ES256). Its `kid` is the workload key's
//! `did:key`, so a peer needs no fetch to check it, and its `typ` keeps it
//! from being read as any other token the workload key signs.

use arkavo_crypto::{AgentPublicKey, P256VerifyingKey};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

/// The JWS `typ` of a key statement.
pub const MEDIA_TYPE: &str = "key-statement+jwt";

const ALGORITHM: &str = "ES256";

/// The job a subordinate key does. Each fixes the key's algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Purpose {
    /// Signing gossip: Ed25519.
    Gossip,
    /// The iroh endpoint key: Ed25519.
    Iroh,
    /// The key TDF key access wraps to: P-256.
    Tdf,
}

/// What a workload key says about one of its agent's keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyStatement {
    /// The workload key's `did:key`: the agent.
    #[serde(rename = "iss")]
    pub issuer: String,
    /// The subordinate key's `did:key`.
    #[serde(rename = "sub")]
    pub key: String,
    pub purpose: Purpose,
    /// Rises each time the agent replaces the key for this purpose.
    #[serde(rename = "gen")]
    pub generation: u64,
    /// Unix seconds from which the statement holds.
    #[serde(rename = "nbf")]
    pub not_before: u64,
    /// Unix seconds from which it no longer holds.
    #[serde(rename = "exp")]
    pub not_after: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StatementError {
    #[error("not a key statement: {0}")]
    Malformed(String),
    #[error("the key statement's signature does not verify: {0}")]
    Signature(String),
    #[error("the key statement names the wrong kind of key: {0}")]
    Key(String),
    #[error("the key statement holds from {not_before} until {not_after}, not at {now}")]
    OutsideWindow {
        not_before: u64,
        not_after: u64,
        now: u64,
    },
    #[error("generation {presented} of {issuer}'s {purpose:?} key is older than generation {seen}")]
    Superseded {
        issuer: String,
        purpose: Purpose,
        seen: u64,
        presented: u64,
    },
    #[error("{issuer} named two different {purpose:?} keys for generation {generation}")]
    Conflicting {
        issuer: String,
        purpose: Purpose,
        generation: u64,
    },
    #[error("signing the key statement failed: {0}")]
    Signer(String),
}

/// Sign `statement` with the workload key that `key_id` names. `sign`
/// returns ECDSA P-256 SHA-256 as `r || s`.
pub fn issue<E: std::fmt::Display>(
    statement: &KeyStatement,
    key_id: &str,
    sign: impl FnOnce(&[u8]) -> Result<[u8; 64], E>,
) -> Result<String, StatementError> {
    let (_, did) = P256VerifyingKey::from_did_key_id(key_id)
        .map_err(|e| StatementError::Malformed(format!("{key_id}: {e}")))?;
    if did != statement.issuer {
        return Err(StatementError::Malformed(format!(
            "{key_id} is not a key of {}",
            statement.issuer
        )));
    }
    check_contents(statement)?;
    let header = serde_json::json!({ "alg": ALGORITHM, "typ": MEDIA_TYPE, "kid": key_id });
    let payload =
        serde_json::to_vec(statement).map_err(|e| StatementError::Malformed(e.to_string()))?;
    let input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(payload)
    );
    let signature = sign(input.as_bytes()).map_err(|e| StatementError::Signer(e.to_string()))?;
    Ok(format!("{input}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

/// The statement `compact` makes, if its workload key signed it and it holds
/// at `now` (Unix seconds).
pub fn verify(compact: &str, now: u64) -> Result<KeyStatement, StatementError> {
    let malformed = |why: &str| StatementError::Malformed(why.to_string());
    let mut parts = compact.split('.');
    let (Some(protected), Some(payload), Some(signature), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(malformed("not a compact JWS"));
    };

    let header: Value = decode(protected)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| malformed("the header is not base64url JSON"))?;
    if header.get("typ").and_then(Value::as_str) != Some(MEDIA_TYPE) {
        return Err(malformed("the header's typ is not a key statement's"));
    }
    if header.get("alg").and_then(Value::as_str) != Some(ALGORITHM) {
        return Err(malformed("the statement is not ES256"));
    }
    // An extension this verifier does not know could change what the
    // signature means (RFC 7515 §4.1.11).
    if header.get("crit").is_some() {
        return Err(malformed(
            "the header asks for extensions this verifier lacks",
        ));
    }
    let key_id = header
        .get("kid")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("the header names no key"))?;
    let (workload_key, did) = P256VerifyingKey::from_did_key_id(key_id)
        .map_err(|e| StatementError::Signature(format!("{key_id}: {e}")))?;

    // JWS ES256 is the 64-byte r || s; DER is not a JWS signature.
    let signature = decode(signature)
        .filter(|bytes| bytes.len() == 64)
        .ok_or_else(|| StatementError::Signature("not a 64-byte ES256 value".to_string()))?;
    workload_key
        .verify(format!("{protected}.{payload}").as_bytes(), &signature)
        .map_err(|_| StatementError::Signature(format!("not signed by {did}")))?;

    let statement: KeyStatement = decode(payload)
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .ok_or_else(|| malformed("the payload is not a key statement"))?;
    if statement.issuer != did {
        return Err(StatementError::Signature(format!(
            "signed by {did} but issued in the name of {}",
            statement.issuer
        )));
    }
    check_contents(&statement)?;
    if now < statement.not_before || now >= statement.not_after {
        return Err(StatementError::OutsideWindow {
            not_before: statement.not_before,
            not_after: statement.not_after,
            now,
        });
    }
    Ok(statement)
}

impl KeyStatement {
    /// The subordinate key as an Ed25519 key, for the purposes that use one.
    pub fn ed25519_key(&self) -> Result<AgentPublicKey, StatementError> {
        match self.purpose {
            Purpose::Gossip | Purpose::Iroh => AgentPublicKey::from_did_key(&self.key)
                .map_err(|e| StatementError::Key(format!("{}: {e}", self.key))),
            Purpose::Tdf => Err(StatementError::Key(format!(
                "a {:?} key is not Ed25519",
                self.purpose
            ))),
        }
    }

    /// The subordinate key as a P-256 key, for the purposes that use one.
    pub fn p256_key(&self) -> Result<P256VerifyingKey, StatementError> {
        match self.purpose {
            Purpose::Tdf => P256VerifyingKey::from_did_key(&self.key)
                .map_err(|e| StatementError::Key(format!("{}: {e}", self.key))),
            Purpose::Gossip | Purpose::Iroh => Err(StatementError::Key(format!(
                "a {:?} key is not P-256",
                self.purpose
            ))),
        }
    }
}

/// What a statement must say, whoever signed it and whenever it is read.
fn check_contents(statement: &KeyStatement) -> Result<(), StatementError> {
    // The canonical did:key, so one key has one name in every registry.
    let canonical = match statement.purpose {
        Purpose::Gossip | Purpose::Iroh => statement.ed25519_key()?.to_did_key(),
        Purpose::Tdf => statement.p256_key()?.to_did_key(),
    };
    if canonical != statement.key {
        return Err(StatementError::Key(format!(
            "{} is not the canonical did:key of its key",
            statement.key
        )));
    }
    if statement.key == statement.issuer {
        return Err(StatementError::Key(
            "the workload key does not stand in for its own subordinate".to_string(),
        ));
    }
    if statement.not_before >= statement.not_after {
        return Err(StatementError::Malformed(
            "the statement's window is empty".to_string(),
        ));
    }
    Ok(())
}

fn decode(part: &str) -> Option<Vec<u8>> {
    URL_SAFE_NO_PAD.decode(part).ok()
}

/// The latest generation of each agent's key for each purpose this peer has
/// accepted.
///
/// A statement for an older generation is refused, so a key the agent has
/// replaced cannot be brought back; a newer one replaces the old key.
#[derive(Debug, Default)]
pub struct Generations {
    latest: HashMap<(String, Purpose), (u64, String)>,
}

impl Generations {
    /// Record `statement` unless it is older than one already accepted, or
    /// names a different key for the same generation.
    pub fn admit(&mut self, statement: &KeyStatement) -> Result<(), StatementError> {
        match self
            .latest
            .entry((statement.issuer.clone(), statement.purpose))
        {
            Entry::Vacant(entry) => {
                entry.insert((statement.generation, statement.key.clone()));
            }
            Entry::Occupied(mut entry) => {
                let (seen, key) = entry.get();
                if statement.generation < *seen {
                    return Err(StatementError::Superseded {
                        issuer: statement.issuer.clone(),
                        purpose: statement.purpose,
                        seen: *seen,
                        presented: statement.generation,
                    });
                }
                if statement.generation == *seen && *key != statement.key {
                    return Err(StatementError::Conflicting {
                        issuer: statement.issuer.clone(),
                        purpose: statement.purpose,
                        generation: statement.generation,
                    });
                }
                entry.insert((statement.generation, statement.key.clone()));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
