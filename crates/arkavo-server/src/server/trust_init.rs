//! Persistent MCP-T trust service initialization.
//!
//! The previous wiring constructed `TrustService` with an ephemeral keypair
//! and an in-memory event log on every server start. That made the L1 endpoint
//! "theater": signatures never verified across a bounce, and `trust/history`
//! lost every event when the process restarted.
//!
//! This module loads the persistent device keypair (the same anchor used by
//! `arkavo-device-identity` everywhere else in the codebase) and opens a
//! SQLite-backed `TrustStore` under the agent data directory, then hands back
//! a fully-wired `TrustService` ready to be shared across the JSON-RPC impl
//! and the background sync task.

use std::sync::Arc;

use arkavo_crypto::AgentKeypair;
use arkavo_device_identity::keypair as device_keypair_store;
use arkavo_memory::MemoryStorage;
use arkavo_trust::{SharedTrustService, TrustService, TrustStore};
use chrono::{DateTime, Utc};
use tracing::{info, warn};

const TRUST_SCORE_MAX_AGE_SECS: u32 = 3600;
const TRUST_DB_FILENAME: &str = "trust.db";

/// Result of trust service initialization. The `created_at` is the device
/// keypair file's mtime — used as the agent's birth time for the MCP-T
/// tenure dimension so tenure persists across process restarts. Falls back
/// to `Utc::now()` if the file mtime is unavailable (rare; means we just
/// created the keypair this tick, in which case tenure starts at zero).
pub(super) struct TrustInit {
    pub service: SharedTrustService,
    pub created_at: DateTime<Utc>,
}

/// Load the persistent agent keypair (creating it on first run) and open the
/// trust event store. Returns `None` on any failure so the caller can leave
/// the trust endpoint advertised-but-unimplemented rather than silently
/// substituting an ephemeral identity.
pub(super) async fn init_trust_service() -> Option<TrustInit> {
    let keypair = match load_or_create_device_keypair() {
        Ok(kp) => kp,
        Err(e) => {
            warn!("MCP-T trust service disabled: keypair load failed: {e}");
            return None;
        }
    };

    let store = match open_trust_store().await {
        Ok(store) => store,
        Err(e) => {
            warn!("MCP-T trust service disabled: store open failed: {e}");
            return None;
        }
    };

    let created_at = device_keypair_store::created_at()
        .ok()
        .flatten()
        .map(DateTime::<Utc>::from)
        .unwrap_or_else(Utc::now);

    let did = keypair.public_key().to_did_key();
    info!("MCP-T trust service initialized with persistent identity {did}");

    Some(TrustInit {
        service: Arc::new(TrustService::with_store(
            keypair,
            TRUST_SCORE_MAX_AGE_SECS,
            store,
        )),
        created_at,
    })
}

fn load_or_create_device_keypair() -> Result<AgentKeypair, String> {
    let bytes = match device_keypair_store::get_keypair().map_err(|e| e.to_string())? {
        Some(b) => b,
        None => {
            let new_kp = AgentKeypair::generate();
            let new_bytes = new_kp.to_bytes();
            device_keypair_store::store_keypair(&new_bytes).map_err(|e| e.to_string())?;
            new_bytes
        }
    };
    AgentKeypair::from_bytes(&bytes).map_err(|e| e.to_string())
}

async fn open_trust_store() -> Result<TrustStore, String> {
    let data_dir = MemoryStorage::get_data_directory()
        .map_err(|e| format!("agent data directory unavailable: {e}"))?;
    let db_path = data_dir.join(TRUST_DB_FILENAME);
    TrustStore::new(&db_path)
        .await
        .map_err(|e| format!("trust store open failed at {}: {e}", db_path.display()))
}
