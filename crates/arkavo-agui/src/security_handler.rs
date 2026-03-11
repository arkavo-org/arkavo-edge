//! Security and TDF audit handler for AG-UI.
//!
//! Provides security status, TDF encryption audit events, and policy
//! information to the web UI. Each agent is its own KAS -- this handler
//! reports on the local agent's encryption posture.

use crate::types::AgUiEvent;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::mpsc;

/// Tracks TDF audit state and serves security status to the UI.
pub struct SecurityHandler {
    kas_enabled: bool,
    kas_url: String,
    agent_id: String,
    key_id: String,
    preflight_enabled: bool,
    preflight_policy_count: u32,
    audit_count: Arc<AtomicU64>,
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

    /// Redact a sensitive string value for UI display.
    fn redact_sensitive(value: &str) -> String {
        if value.is_empty() {
            return String::new();
        }
        arkavo_validation::sanitize::REDACTED_SENTINEL.to_string()
    }

    /// Handle security-related UI events.
    ///
    /// Redacts sensitive values (KAS URL, agent ID, key ID) before
    /// sending to the UI to prevent exposure via browser DevTools or
    /// UI state inspection.
    pub async fn handle_event(
        &self,
        event: &AgUiEvent,
        tx: &mpsc::Sender<AgUiEvent>,
    ) -> anyhow::Result<()> {
        if let AgUiEvent::GetSecurityStatus = event {
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
            // KAS URL is now redacted for UI display
            assert_eq!(kas_url, arkavo_validation::sanitize::REDACTED_SENTINEL);
            assert_eq!(audit_count, 1);
            assert_eq!(encryption_algorithm, "AES-256-GCM");
        } else {
            panic!("Expected SecurityStatusUpdate");
        }
    }
}
