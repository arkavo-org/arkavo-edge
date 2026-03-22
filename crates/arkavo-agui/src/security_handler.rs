//! Security, TDF audit, and data plane handler for AG-UI.
//!
//! Reports mesh-wide security posture by aggregating KAS status from
//! connected agents. Each agent is its own KAS — this handler tracks
//! which agents have KAS enabled and their encryption capabilities.

use crate::types::AgUiEvent;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use tokio::sync::{RwLock, mpsc};

/// Per-agent KAS info discovered via polling.
#[derive(Debug, Clone)]
pub struct AgentKasInfo {
    pub enabled: bool,
    pub public_key: String,
    pub key_id: String,
    pub algorithm: String,
}

/// Tracks mesh-wide TDF audit state and data plane activity for the UI.
pub struct SecurityHandler {
    /// Per-agent KAS status (populated by polling agents)
    agent_kas: Arc<RwLock<HashMap<String, AgentKasInfo>>>,
    /// Preflight from local AGENTS.md
    preflight_enabled: bool,
    preflight_policy_count: u32,
    audit_count: Arc<AtomicU64>,
    // Per-agent Iroh status (populated by polling agents)
    agent_iroh: Arc<RwLock<HashMap<String, bool>>>,
    // Data plane tracking
    iroh_active: AtomicBool,
    shares_sent: Arc<AtomicU64>,
    shares_received: Arc<AtomicU64>,
    bytes_staged: Arc<AtomicU64>,
    bytes_fetched: Arc<AtomicU64>,
    pending_offers: Arc<AtomicU32>,
}

impl Default for SecurityHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityHandler {
    pub fn new() -> Self {
        Self {
            agent_kas: Arc::new(RwLock::new(HashMap::new())),
            preflight_enabled: false,
            preflight_policy_count: 0,
            audit_count: Arc::new(AtomicU64::new(0)),
            agent_iroh: Arc::new(RwLock::new(HashMap::new())),
            iroh_active: AtomicBool::new(false),
            shares_sent: Arc::new(AtomicU64::new(0)),
            shares_received: Arc::new(AtomicU64::new(0)),
            bytes_staged: Arc::new(AtomicU64::new(0)),
            bytes_fetched: Arc::new(AtomicU64::new(0)),
            pending_offers: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Configure preflight from AGENTS.md parsed config.
    pub fn configure_from_agents_md(&mut self) {
        let config = arkavo_router::load_agent_config().unwrap_or_default();

        if let Some(ref pf) = config.preflight {
            self.preflight_enabled = true;
            self.preflight_policy_count = pf.policies.len() as u32;
        }
    }

    /// Update KAS info for a specific agent (called during mesh polling).
    pub async fn update_agent_kas(&self, agent_id: &str, info: AgentKasInfo) {
        self.agent_kas
            .write()
            .await
            .insert(agent_id.to_string(), info);
    }

    /// Mark an agent as not having KAS (e.g., poll failed).
    pub async fn mark_agent_no_kas(&self, agent_id: &str) {
        self.agent_kas.write().await.remove(agent_id);
    }

    /// Get the audit counter for external incrementing.
    pub fn audit_counter(&self) -> Arc<AtomicU64> {
        self.audit_count.clone()
    }

    /// Record that an audit encryption happened.
    pub fn record_audit(&self) {
        self.audit_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Update Iroh status for a specific agent and recalculate mesh-wide flag.
    pub async fn update_agent_iroh(&self, agent_id: &str, active: bool) {
        self.agent_iroh
            .write()
            .await
            .insert(agent_id.to_string(), active);
        // Any agent with Iroh means the mesh data plane is active
        let any_active = self.agent_iroh.read().await.values().any(|&v| v);
        self.iroh_active.store(any_active, Ordering::Relaxed);
    }

    /// Set whether any Iroh P2P node is active in the mesh.
    pub fn set_iroh_active(&self, active: bool) {
        self.iroh_active.store(active, Ordering::Relaxed);
    }

    /// Record a TDF share sent via Iroh.
    pub fn record_share_sent(&self, bytes: u64) {
        self.shares_sent.fetch_add(1, Ordering::Relaxed);
        self.bytes_staged.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Record a TDF share received via Iroh.
    pub fn record_share_received(&self, bytes: u64) {
        self.shares_received.fetch_add(1, Ordering::Relaxed);
        self.bytes_fetched.fetch_add(bytes, Ordering::Relaxed);
    }

    /// Update the pending offer count.
    pub fn set_pending_offers(&self, count: u32) {
        self.pending_offers.store(count, Ordering::Relaxed);
    }

    /// Handle security and data plane UI events.
    pub async fn handle_event(
        &self,
        event: &AgUiEvent,
        tx: &mpsc::Sender<AgUiEvent>,
    ) -> anyhow::Result<()> {
        match event {
            AgUiEvent::GetSecurityStatus => {
                let kas_map = self.agent_kas.read().await;
                let iroh_map = self.agent_iroh.read().await;
                let kas_count = kas_map.len();
                let any_kas_enabled = !kas_map.is_empty();

                // Aggregate: show first agent's info, count of KAS-enabled agents
                let (agent_id, key_id, algorithm) = if let Some((id, info)) = kas_map.iter().next()
                {
                    (id.clone(), info.key_id.clone(), info.algorithm.clone())
                } else {
                    (String::new(), String::new(), "AES-256-GCM".to_string())
                };

                let kas_summary = if kas_count > 1 {
                    format!("{agent_id} (+{} more)", kas_count - 1)
                } else {
                    agent_id
                };

                // Build per-agent security info from KAS + Iroh maps
                let mut agent_ids: std::collections::HashSet<String> =
                    std::collections::HashSet::new();
                agent_ids.extend(kas_map.keys().cloned());
                agent_ids.extend(iroh_map.keys().cloned());
                let agents: Vec<crate::types::AgentSecurityInfo> = agent_ids
                    .into_iter()
                    .map(|id| {
                        let kas = kas_map.get(&id);
                        let iroh = iroh_map.get(&id).copied().unwrap_or(false);
                        crate::types::AgentSecurityInfo {
                            id: id.clone(),
                            kas_enabled: kas.is_some(),
                            key_id: kas.map(|k| k.key_id.clone()).unwrap_or_default(),
                            algorithm: kas
                                .map(|k| k.algorithm.clone())
                                .unwrap_or_else(|| "N/A".to_string()),
                            iroh_active: iroh,
                        }
                    })
                    .collect();
                drop(kas_map);
                drop(iroh_map);

                let response = AgUiEvent::SecurityStatusUpdate {
                    kas_enabled: any_kas_enabled,
                    kas_url: if any_kas_enabled {
                        format!("{kas_count} agent(s) with KAS")
                    } else {
                        String::new()
                    },
                    agent_id: kas_summary,
                    key_id,
                    encryption_algorithm: algorithm,
                    audit_count: self.audit_count.load(Ordering::Relaxed),
                    preflight_enabled: self.preflight_enabled,
                    preflight_policies: self.preflight_policy_count,
                    agents,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                tx.send(response).await?;
            }
            AgUiEvent::GetDataPlaneStatus => {
                let response = AgUiEvent::DataPlaneStatusUpdate {
                    iroh_active: self.iroh_active.load(Ordering::Relaxed),
                    total_shares_sent: self.shares_sent.load(Ordering::Relaxed),
                    total_shares_received: self.shares_received.load(Ordering::Relaxed),
                    total_bytes_staged: self.bytes_staged.load(Ordering::Relaxed),
                    total_bytes_fetched: self.bytes_fetched.load(Ordering::Relaxed),
                    pending_offers: self.pending_offers.load(Ordering::Relaxed),
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                tx.send(response).await?;
            }
            _ => {}
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_security_handler() {
        let handler = SecurityHandler::new();
        assert_eq!(handler.audit_count.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn record_audit_increments() {
        let handler = SecurityHandler::new();
        handler.record_audit();
        handler.record_audit();
        assert_eq!(handler.audit_count.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn audit_counter_shared() {
        let handler = SecurityHandler::new();
        let counter = handler.audit_counter();
        handler.record_audit();
        assert_eq!(counter.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn data_plane_tracking() {
        let handler = SecurityHandler::new();

        handler.record_share_sent(4096);
        handler.record_share_sent(8192);
        handler.record_share_received(2048);

        assert_eq!(handler.shares_sent.load(Ordering::Relaxed), 2);
        assert_eq!(handler.bytes_staged.load(Ordering::Relaxed), 12288);
        assert_eq!(handler.shares_received.load(Ordering::Relaxed), 1);
        assert_eq!(handler.bytes_fetched.load(Ordering::Relaxed), 2048);
    }

    #[test]
    fn pending_offers_tracking() {
        let handler = SecurityHandler::new();
        handler.set_pending_offers(3);
        assert_eq!(handler.pending_offers.load(Ordering::Relaxed), 3);
    }

    #[tokio::test]
    async fn handle_get_security_status_no_agents() {
        let handler = SecurityHandler::new();

        let (tx, mut rx) = mpsc::channel(10);
        handler
            .handle_event(&AgUiEvent::GetSecurityStatus, &tx)
            .await
            .unwrap();

        let response = rx.recv().await.unwrap();
        if let AgUiEvent::SecurityStatusUpdate {
            kas_enabled,
            kas_url,
            ..
        } = response
        {
            assert!(!kas_enabled);
            assert!(kas_url.is_empty());
        } else {
            panic!("Expected SecurityStatusUpdate");
        }
    }

    #[tokio::test]
    async fn handle_get_security_status_with_agents() {
        let handler = SecurityHandler::new();
        handler
            .update_agent_kas(
                "agent-1",
                AgentKasInfo {
                    enabled: true,
                    public_key: "BK5x...".to_string(),
                    key_id: "kas-key-1".to_string(),
                    algorithm: "ec:secp256r1".to_string(),
                },
            )
            .await;

        let (tx, mut rx) = mpsc::channel(10);
        handler
            .handle_event(&AgUiEvent::GetSecurityStatus, &tx)
            .await
            .unwrap();

        let response = rx.recv().await.unwrap();
        if let AgUiEvent::SecurityStatusUpdate {
            kas_enabled,
            kas_url,
            agent_id,
            encryption_algorithm,
            ..
        } = response
        {
            assert!(kas_enabled);
            assert_eq!(kas_url, "1 agent(s) with KAS");
            assert_eq!(agent_id, "agent-1");
            assert_eq!(encryption_algorithm, "ec:secp256r1");
        } else {
            panic!("Expected SecurityStatusUpdate");
        }
    }

    #[tokio::test]
    async fn handle_get_data_plane_status() {
        let handler = SecurityHandler::new();
        handler.set_iroh_active(true);
        handler.record_share_sent(1024);
        handler.record_share_received(512);
        handler.set_pending_offers(2);

        let (tx, mut rx) = mpsc::channel(10);
        handler
            .handle_event(&AgUiEvent::GetDataPlaneStatus, &tx)
            .await
            .unwrap();

        let response = rx.recv().await.unwrap();
        if let AgUiEvent::DataPlaneStatusUpdate {
            iroh_active,
            total_shares_sent,
            total_shares_received,
            total_bytes_staged,
            total_bytes_fetched,
            pending_offers,
            ..
        } = response
        {
            assert!(iroh_active);
            assert_eq!(total_shares_sent, 1);
            assert_eq!(total_shares_received, 1);
            assert_eq!(total_bytes_staged, 1024);
            assert_eq!(total_bytes_fetched, 512);
            assert_eq!(pending_offers, 2);
        } else {
            panic!("Expected DataPlaneStatusUpdate");
        }
    }
}
