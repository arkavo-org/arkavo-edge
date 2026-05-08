#![allow(clippy::disallowed_methods)]

//! End-to-end SwarmKit specialization-bundle round trip.
//!
//! Exercises the full wire path that the orchestrator drives in
//! production:
//!
//!   build bundle → wrap with TDF (recipient DID policy) → base64
//!   encode → deliver to receiving agent's `agent.specialize` handler
//!   → decrypt → apply to AgentMetadata + RoleSpecializationStore
//!
//! This test crosses the arkavo-protocol / arkavo-server boundary so a
//! wire-format break in the bundle JSON, the TDF dissemination check,
//! or the handler's apply path will fail it. Uses MockTdfService for
//! both wrap and unwrap so KAS is not required.

use std::collections::HashMap;
use std::sync::Arc;

use arkavo_protocol::AgentSpecializationBundle;
use arkavo_protocol::agent_specialization::{
    AgentPersona, ArpDocument, McpToolGrant, RoleContext, unwrap_bundle, wrap_bundle,
};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::MetricsCollector;
use arkavo_protocol::rate_limit::{RateLimitConfig, RateLimiter};
use arkavo_protocol::types::AgentSpecializeRequest;
use arkavo_server::server::config_helpers::{AgentMetadata, RoleSpecializationStore};
use arkavo_server::server::handlers::specialization::{BundleDecryptor, handle_agent_specialize};
use arkavo_tdf::TdfManifest;
use arkavo_tdf::testing::MockTdfService;
use async_trait::async_trait;
use base64::Engine;

fn arp_doc() -> ArpDocument {
    let json = serde_json::json!({
        "arp_spec": "0.1.0",
        "adl_ref": { "uri": "urn:test:role", "document_hash": null },
        "adaptation": { "method": "thompson_sampling" },
        "feedback_loops": {
            "immediate": {
                "quality_gate": {
                    "threshold_default": 0.7,
                    "metric": "composite",
                    "on_failure": "update_prior_and_log"
                }
            },
            "short_term": {
                "policy_cache": {
                    "default_ttl_sec": 3600,
                    "decay_strategy": "exponential",
                    "decay_half_life_sec": 86400
                }
            }
        },
        "budget": {
            "task_ceiling_usd": 0.05,
            "on_exhaustion": "halt_and_report",
            "velocity": { "max_spend_per_minute_usd": 0.01 }
        }
    });
    serde_json::from_value(json).expect("arp doc")
}

fn build_bundle(role: &str, agent_did: &str) -> AgentSpecializationBundle {
    let mut tokens = HashMap::new();
    tokens.insert("ASSET_TOKEN".to_string(), "tok-asset".to_string());
    AgentSpecializationBundle {
        persona: AgentPersona {
            name: agent_did.to_string(),
            purpose: format!("Be the {role} role"),
            model: "gemma-4-9b".into(),
            mcp_tools: vec![McpToolGrant {
                server: "asset-store".into(),
                tools: vec!["read".into(), "describe".into()],
            }],
        },
        api_tokens: tokens,
        arp_overlay: arp_doc(),
        role_context: RoleContext {
            kit_id: "kit:demo:0.1.0".into(),
            flight_id: "44444444-4444-4444-4444-444444444444".into(),
            role_id: role.into(),
            role_type: "asset_analyst".into(),
            deliverable_schema: serde_json::json!({"type": "object"}),
            handoff_targets: vec!["copy".into()],
        },
    }
}

/// BundleDecryptor that mirrors what an agent built with the `kas`
/// feature does today: parse TDF manifest from bytes, then call
/// `unwrap_bundle` (which performs the dissemination DID pre-flight
/// check before invoking the decryptor).
struct LocalDecryptor {
    svc: MockTdfService,
}

#[async_trait]
impl BundleDecryptor for LocalDecryptor {
    async fn decrypt(
        &self,
        tdf_bytes: &[u8],
        recipient_did: &str,
    ) -> Result<AgentSpecializationBundle, String> {
        let tdf: TdfManifest =
            serde_json::from_slice(tdf_bytes).map_err(|e| format!("parse TDF manifest: {e}"))?;
        unwrap_bundle(&tdf, &self.svc, recipient_did)
            .await
            .map_err(|e| e.to_string())
    }
}

#[tokio::test]
async fn bundle_round_trips_orchestrator_to_agent() {
    let did = "did:web:agent-7.arkavo.net";
    let svc = MockTdfService::default();
    let bundle = build_bundle("analyst", did);

    // Orchestrator side: wrap.
    let tdf = wrap_bundle(&bundle, &svc, did).await.expect("wrap");
    let tdf_bytes = serde_json::to_vec(&tdf).expect("serialize tdf");
    let encoded = base64::engine::general_purpose::STANDARD.encode(&tdf_bytes);

    // Agent side: receive via agent.specialize handler.
    let agent_metadata = Arc::new(tokio::sync::RwLock::new(AgentMetadata {
        name: did.into(),
        purpose: "(unspecialized)".into(),
        model: "(none)".into(),
        did: Some(did.into()),
        ..AgentMetadata::default()
    }));
    let role_store = Arc::new(RoleSpecializationStore::default());
    let metrics = Arc::new(MetricsCollector::new(false));
    let limiter = RateLimiter::new(RateLimitConfig::default());
    let registry = Arc::new(McpRegistry::new());
    let decryptor = LocalDecryptor { svc };

    let response = handle_agent_specialize(
        &metrics,
        &limiter,
        &registry,
        &agent_metadata,
        &role_store,
        &decryptor,
        AgentSpecializeRequest {
            requester_id: "did:web:orchestrator.arkavo.net".into(),
            encrypted_bundle: encoded,
            task_context: None,
            session_id: None,
        },
    )
    .await
    .expect("specialize round trip");

    assert!(response.accepted);
    assert!(
        response
            .activated_capabilities
            .iter()
            .any(|c| c == "asset-store:read")
    );

    let meta = agent_metadata.read().await;
    assert_eq!(meta.purpose, "Be the analyst role");
    assert_eq!(meta.model, "gemma-4-9b");
    assert_eq!(
        meta.api_keys.get("ASSET_TOKEN").map(String::as_str),
        Some("tok-asset")
    );

    let stored = role_store.get().await.expect("role context");
    assert_eq!(stored.role_id, "analyst");
    assert_eq!(stored.kit_id, "kit:demo:0.1.0");
}

#[tokio::test]
async fn bundle_for_other_agent_is_rejected_at_unwrap() {
    let bundle = build_bundle("analyst", "did:web:agent-A.arkavo.net");
    let svc = MockTdfService::default();
    let tdf = wrap_bundle(&bundle, &svc, "did:web:agent-A.arkavo.net")
        .await
        .expect("wrap");
    let encoded = base64::engine::general_purpose::STANDARD
        .encode(serde_json::to_vec(&tdf).expect("serialize"));

    // Agent B receives a bundle wrapped for agent A — handler must reject.
    let our_did = "did:web:agent-B.arkavo.net";
    let agent_metadata = Arc::new(tokio::sync::RwLock::new(AgentMetadata {
        name: our_did.into(),
        did: Some(our_did.into()),
        ..AgentMetadata::default()
    }));
    let role_store = Arc::new(RoleSpecializationStore::default());
    let metrics = Arc::new(MetricsCollector::new(false));
    let limiter = RateLimiter::new(RateLimitConfig::default());
    let registry = Arc::new(McpRegistry::new());
    let decryptor = LocalDecryptor { svc };

    let err = handle_agent_specialize(
        &metrics,
        &limiter,
        &registry,
        &agent_metadata,
        &role_store,
        &decryptor,
        AgentSpecializeRequest {
            requester_id: "did:web:orchestrator.arkavo.net".into(),
            encrypted_bundle: encoded,
            task_context: None,
            session_id: None,
        },
    )
    .await
    .expect_err("misrouted bundle rejected");

    assert_eq!(err.code(), -32603);
    assert!(role_store.get().await.is_none());
}
