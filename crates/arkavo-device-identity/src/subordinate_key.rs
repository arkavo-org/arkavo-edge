//! The keys an agent uses for one job each, vouched for by its workload key
//! (INGRESS-006).
//!
//! Each lives in the agent's directory beside its workload key, so a peer
//! that bound the key once binds the same key after a restart. A
//! [`Voucher`] signs the statement peers check and renews it before it runs
//! out: an agent runs for months, a statement holds for a week.

use crate::workload_key::WorkloadKey;
use crate::{DeviceIdentityError, Result};
use arkavo_crypto::{AgentKeypair, P256SigningKeypair};
use arkavo_key_statement::KeyStatement;
pub use arkavo_key_statement::Purpose;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::Path;
use std::sync::{Arc, Mutex};

const FORMAT: &str = "arkavo-subordinate-key/1";

/// How long a statement holds.
const LIFETIME: u64 = 7 * 24 * 60 * 60;
/// A statement this close to running out is replaced before it is shown.
const RENEW_WITHIN: u64 = 24 * 60 * 60;
/// How far back a statement's window starts, for peers whose clocks run
/// behind this one's.
const CLOCK_SKEW: u64 = 5 * 60;

#[derive(Serialize, Deserialize)]
struct Record {
    format: String,
    purpose: Purpose,
    generation: u64,
    key: String,
}

/// One of the agent's keys for a single job.
pub struct SubordinateKey {
    purpose: Purpose,
    generation: u64,
    secret: Vec<u8>,
}

impl SubordinateKey {
    pub fn purpose(&self) -> Purpose {
        self.purpose
    }

    /// The time the key was made, in Unix seconds. A key made again after
    /// its file is lost outranks the one peers knew, with no counter to keep.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// The private key: an Ed25519 seed for gossip and iroh, a P-256 scalar
    /// for TDF.
    pub fn secret(&self) -> &[u8] {
        &self.secret
    }
}

impl fmt::Debug for SubordinateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubordinateKey")
            .field("purpose", &self.purpose)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

fn filename(purpose: Purpose) -> &'static str {
    match purpose {
        Purpose::Gossip => "gossip_key",
        Purpose::Iroh => "iroh_key",
        Purpose::Tdf => "tdf_key",
    }
}

/// The key the agent named `agent` uses for `purpose`, made on first use.
pub fn load_or_create(agent: &str, purpose: Purpose) -> Result<SubordinateKey> {
    load_or_create_at(
        &crate::agent_dir::agent_dir(agent)?.join(filename(purpose)),
        purpose,
    )
}

/// [`load_or_create`] for a key file at `path` instead of the agent's own
/// directory.
pub fn load_or_create_at(path: &Path, purpose: Purpose) -> Result<SubordinateKey> {
    if let Some(key) = load(path, purpose)? {
        return Ok(key);
    }
    if let Some(dir) = path.parent() {
        crate::private_file::create_private_dir(dir)?;
    }
    let secret = match purpose {
        Purpose::Gossip | Purpose::Iroh => AgentKeypair::generate().to_bytes(),
        Purpose::Tdf => P256SigningKeypair::generate().to_bytes(),
    };
    let key = SubordinateKey {
        purpose,
        generation: unix_now(),
        secret,
    };
    let record = serde_json::to_vec(&Record {
        format: FORMAT.to_string(),
        purpose,
        generation: key.generation,
        key: STANDARD.encode(&key.secret),
    })
    .map_err(|e| DeviceIdentityError::WorkloadKey(e.to_string()))?;
    if crate::private_file::create_private(path, &record)? {
        return Ok(key);
    }
    // Another process started at the same moment; its key is the agent's.
    load(path, purpose)?.ok_or_else(|| {
        DeviceIdentityError::WorkloadKey(format!("{} vanished as it was made", path.display()))
    })
}

fn load(path: &Path, purpose: Purpose) -> Result<Option<SubordinateKey>> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(DeviceIdentityError::Storage(format!(
                "Failed to read {}: {e}",
                path.display()
            )));
        }
    };
    let refuse = |why: &str| {
        DeviceIdentityError::WorkloadKey(format!(
            "{} {why}; move it aside and the agent makes a new one",
            path.display()
        ))
    };
    let record: Record = serde_json::from_slice(&bytes).map_err(|_| refuse("is not a key"))?;
    if record.format != FORMAT {
        return Err(refuse(&format!(
            "is a {} key, which this version does not read",
            record.format
        )));
    }
    if record.purpose != purpose {
        return Err(refuse(&format!("holds the {:?} key", record.purpose)));
    }
    let secret = STANDARD
        .decode(&record.key)
        .ok()
        .filter(|secret| secret.len() == 32)
        .ok_or_else(|| refuse("is damaged"))?;
    Ok(Some(SubordinateKey {
        purpose,
        generation: record.generation,
        secret,
    }))
}

/// Signs, and renews before it runs out, the statement in which the workload
/// key vouches for one subordinate key.
pub struct Voucher {
    workload: Arc<WorkloadKey>,
    purpose: Purpose,
    key: String,
    generation: u64,
    current: Mutex<Option<(String, u64)>>,
}

impl Voucher {
    /// `key` is the subordinate key's `did:key`.
    pub fn new(workload: Arc<WorkloadKey>, subordinate: &SubordinateKey, key: String) -> Self {
        Self {
            workload,
            purpose: subordinate.purpose,
            key,
            generation: subordinate.generation,
            current: Mutex::new(None),
        }
    }

    /// The statement to show a peer now. Signing may reach the Secure
    /// Enclave, so call it off async workers.
    pub fn statement(&self) -> Result<String> {
        self.statement_at(unix_now())
    }

    fn statement_at(&self, now: u64) -> Result<String> {
        let mut current = self
            .current
            .lock()
            .map_err(|_| DeviceIdentityError::WorkloadKey("a statement signer failed".into()))?;
        if let Some((compact, not_after)) = current.as_ref()
            && now.saturating_add(RENEW_WITHIN) < *not_after
        {
            return Ok(compact.clone());
        }
        let statement = KeyStatement {
            issuer: self.workload.did().to_string(),
            key: self.key.clone(),
            purpose: self.purpose,
            generation: self.generation,
            not_before: now.saturating_sub(CLOCK_SKEW),
            not_after: now.saturating_add(LIFETIME),
        };
        let compact = arkavo_key_statement::issue(&statement, &self.workload.key_id(), |input| {
            self.workload.sign(input)
        })
        .map_err(|e| DeviceIdentityError::WorkloadKey(e.to_string()))?;
        *current = Some((compact.clone(), statement.not_after));
        Ok(compact)
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}

#[cfg(test)]
mod tests;
