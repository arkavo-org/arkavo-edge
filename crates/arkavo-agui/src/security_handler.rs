//! Security, TDF audit, and data plane handler for AG-UI.
//!
//! Provides security status, TDF encryption audit events, data plane
//! activity, and policy information to the web UI. Each agent is its
//! own KAS -- this handler reports on the local agent's encryption
//! posture and Iroh P2P transport activity.

use crate::types::AgUiEvent;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use tokio::sync::mpsc;

/// Tracks TDF audit state and data plane activity for the UI.
pub struct SecurityHandler {
    kas_enabled: bool,
    kas_url: String,
    agent_id: String,
    key_id: String,
    preflight_enabled: bool,
    preflight_policy_count: u32,
    audit_count: Arc<AtomicU64>,
    // Data plane tracking
    iroh_active: bool,
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
            kas_enabled: false,
            kas_url: String::new(),
            agent_id: String::new(),
            key_id: String::new(),
            preflight_enabled: false,
            preflight_policy_count: 0,
            audit_count: Arc::new(AtomicU64::new(0)),
            iroh_active: false,
            shares_sent: Arc::new(AtomicU64::new(0)),
            shares_received: Arc::new(AtomicU64::new(0)),
            bytes_staged: Arc::new(AtomicU64::new(0)),
            bytes_fetched: Arc::new(AtomicU64::new(0)),
            pending_offers: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Configure from AGENTS.md parsed config.
    pub fn configure_from_agents_md(&mut self) {
        let config = arkavo_router::load_agent_config().unwrap_or_default();

        if let Some(ref kas) = config.kas {
            self.kas_enabled = kas.enabled;
            self.key_id = kas
                .key_id
                .clone()
                .unwrap_or_else(|| "kas-key-1".to_string());
        }

        self.agent_id = config
            .name
            .clone()
            .unwrap_or_else(|| "unknown-agent".to_string());

        if let Some(ref pf) = config.preflight {
            self.preflight_enabled = true;
            self.preflight_policy_count = pf.policies.len() as u32;
        }
    }

    /// Set the local KAS URL (agent's own A2A endpoint).
    pub fn set_kas_url(&mut self, url: String) {
        self.kas_url = url;
    }

    /// Get the audit counter for external incrementing.
    pub fn audit_counter(&self) -> Arc<AtomicU64> {
        self.audit_count.clone()
    }

    /// Record that an audit encryption happened.
    pub fn record_audit(&self) {
        self.audit_count.fetch_add(1, Ordering::Relaxed);
    }

    /// Set whether the Iroh P2P node is active.
    pub fn set_iroh_active(&mut self, active: bool) {
        self.iroh_active = active;
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

    /// Redact a sensitive string value for UI display.
    fn redact_sensitive(value: &str) -> String {
        if value.is_empty() {
            return String::new();
        }
        arkavo_validation::sanitize::REDACTED_SENTINEL.to_string()
    }

    /// Handle security and data plane UI events.
    ///
    /// Redacts sensitive values (KAS URL, agent ID, key ID) before
    /// sending to the UI to prevent exposure via browser DevTools or
    /// UI state inspection.
    pub async fn handle_event(
        &self,
        event: &AgUiEvent,
        tx: &mpsc::Sender<AgUiEvent>,
    ) -> anyhow::Result<()> {
        match event {
            AgUiEvent::GetSecurityStatus => {
                let response = AgUiEvent::SecurityStatusUpdate {
                    kas_enabled: self.kas_enabled,
                    kas_url: Self::redact_sensitive(&self.kas_url),
                    agent_id: Self::redact_sensitive(&self.agent_id),
                    key_id: Self::redact_sensitive(&self.key_id),
                    encryption_algorithm: "AES-256-GCM".to_string(),
                    audit_count: self.audit_count.load(Ordering::Relaxed),
                    preflight_enabled: self.preflight_enabled,
                    preflight_policies: self.preflight_policy_count,
                    timestamp: chrono::Utc::now().to_rfc3339(),
                };
                tx.send(response).await?;
            }
            AgUiEvent::GetDataPlaneStatus => {
                let response = AgUiEvent::DataPlaneStatusUpdate {
                    iroh_active: self.iroh_active,
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
        assert!(!handler.kas_enabled);
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
    fn set_kas_url() {
        let mut handler = SecurityHandler::new();
        handler.set_kas_url("http://localhost:8360".to_string());
        assert_eq!(handler.kas_url, "http://localhost:8360");
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
    async fn handle_get_security_status() {
        let mut handler = SecurityHandler::new();
        handler.set_kas_url("http://localhost:8360".to_string());
        handler.record_audit();

        let (tx, mut rx) = mpsc::channel(10);
        let event = AgUiEvent::GetSecurityStatus;

        handler.handle_event(&event, &tx).await.unwrap();

        let response = rx.recv().await.unwrap();
        if let AgUiEvent::SecurityStatusUpdate {
            kas_enabled,
            kas_url,
            audit_count,
            encryption_algorithm,
            ..
        } = response
        {
            assert!(!kas_enabled);
            assert_eq!(kas_url, arkavo_validation::sanitize::REDACTED_SENTINEL);
            assert_eq!(audit_count, 1);
            assert_eq!(encryption_algorithm, "AES-256-GCM");
        } else {
            panic!("Expected SecurityStatusUpdate");
        }
    }

    #[tokio::test]
    async fn handle_get_data_plane_status() {
        let mut handler = SecurityHandler::new();
        handler.set_iroh_active(true);
        handler.record_share_sent(1024);
        handler.record_share_received(512);
        handler.set_pending_offers(2);

        let (tx, mut rx) = mpsc::channel(10);
        let event = AgUiEvent::GetDataPlaneStatus;

        handler.handle_event(&event, &tx).await.unwrap();

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
