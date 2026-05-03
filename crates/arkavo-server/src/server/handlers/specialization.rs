use arkavo_protocol::AgentSpecializationBundle;
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_protocol::types::{AgentSpecializeRequest, AgentSpecializeResponse};
use async_trait::async_trait;
use base64::Engine;
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;
use tracing::{info, warn};

use super::super::config_helpers::{AgentMetadata, RoleSpecializationStore};

/// Decrypts a TDF-wrapped specialization bundle for a specific recipient.
///
/// Lives behind a trait so production code can wire a KAS-backed
/// decryptor (`arkavo-tdf::OpenTdfService`) while tests inject a stub.
/// The handler is generic over this trait, keeping the server crate's
/// dependency surface small.
#[async_trait]
pub trait BundleDecryptor: Send + Sync {
    async fn decrypt(
        &self,
        tdf_bytes: &[u8],
        recipient_did: &str,
    ) -> Result<AgentSpecializationBundle, String>;
}

/// Decryptor used when no real bundle decryptor is wired (e.g. agent
/// built without KAS support, or in tests that exercise the validation
/// path without going through TDF at all). Always errors.
pub struct UnconfiguredBundleDecryptor;

#[async_trait]
impl BundleDecryptor for UnconfiguredBundleDecryptor {
    async fn decrypt(
        &self,
        _tdf_bytes: &[u8],
        _recipient_did: &str,
    ) -> Result<AgentSpecializationBundle, String> {
        Err("agent has no bundle decryptor configured (rebuild with --features kas to apply specialization bundles)".to_string())
    }
}

/// Handle agent.specialize RPC method.
///
/// Receives a TDF-encrypted [`AgentSpecializationBundle`] (base64-encoded
/// in `encrypted_bundle`), decrypts it via the local KAS-backed
/// decryptor, and applies it to the agent's runtime: persona fields are
/// merged into [`AgentMetadata`], API tokens populate the in-memory
/// keyring, and the role context is stashed in
/// [`RoleSpecializationStore`] so subsequent tool outcomes can be tagged
/// with `flight_id` / `role_id`.
///
/// Without the `kas` feature, decryption is unavailable and the handler
/// rejects the request rather than silently dropping the bundle.
pub async fn handle_agent_specialize(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    _mcp_registry: &Arc<McpRegistry>,
    agent_metadata: &Arc<tokio::sync::RwLock<AgentMetadata>>,
    role_specialization: &Arc<RoleSpecializationStore>,
    decryptor: &dyn BundleDecryptor,
    request: AgentSpecializeRequest,
) -> Result<AgentSpecializeResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("agent.specialize".to_string(), metrics.clone());
    match handle_inner(
        rate_limiter,
        metrics,
        agent_metadata,
        role_specialization,
        decryptor,
        request,
    )
    .await
    {
        Ok(response) => {
            timer.success();
            Ok(response)
        }
        Err(err) => {
            timer.error();
            Err(err)
        }
    }
}

async fn handle_inner(
    rate_limiter: &RateLimiter,
    metrics: &Arc<MetricsCollector>,
    agent_metadata: &Arc<tokio::sync::RwLock<AgentMetadata>>,
    role_specialization: &Arc<RoleSpecializationStore>,
    decryptor: &dyn BundleDecryptor,
    request: AgentSpecializeRequest,
) -> Result<AgentSpecializeResponse, ErrorObjectOwned> {
    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        return Err(e);
    }

    let (agent_name, agent_did) = {
        let metadata = agent_metadata.read().await;
        (metadata.name.clone(), metadata.did.clone())
    };

    info!(
        "Received specialization request from '{}' for agent '{}'",
        request.requester_id, agent_name
    );

    if request.encrypted_bundle.is_empty() {
        return Err(ErrorObjectOwned::owned(
            -32602,
            "Invalid params: encrypted_bundle is required",
            Some("The encrypted_bundle field cannot be empty".to_string()),
        ));
    }

    let session_id = request
        .session_id
        .clone()
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let tdf_bytes = base64::engine::general_purpose::STANDARD
        .decode(request.encrypted_bundle.as_bytes())
        .map_err(|e| {
            ErrorObjectOwned::owned(
                -32602,
                "Invalid params: encrypted_bundle is not valid base64",
                Some(e.to_string()),
            )
        })?;

    let did = agent_did.as_deref().ok_or_else(|| {
        ErrorObjectOwned::owned(
            -32603,
            "Agent identity has no DID; cannot decrypt bundle",
            Some("Bundle decryption requires an authenticated DID".to_string()),
        )
    })?;

    let bundle = decryptor
        .decrypt(&tdf_bytes, did)
        .await
        .map_err(|message| {
            warn!(
                session_id = %session_id,
                agent = %agent_name,
                "specialization bundle decryption failed: {message}"
            );
            ErrorObjectOwned::owned(-32603, "Bundle decryption failed", Some(message))
        })?;

    let activated = bundle
        .persona
        .mcp_tools
        .iter()
        .flat_map(|grant| {
            grant
                .tools
                .iter()
                .map(move |t| format!("{}:{}", grant.server, t))
        })
        .collect::<Vec<_>>();

    apply_bundle_to_metadata(agent_metadata, &bundle).await;
    role_specialization.set(bundle.role_context.clone()).await;

    info!(
        session_id = %session_id,
        agent = %agent_name,
        kit_id = %bundle.role_context.kit_id,
        flight_id = %bundle.role_context.flight_id,
        role_id = %bundle.role_context.role_id,
        "agent specialized: persona + tokens + role context applied"
    );

    Ok(AgentSpecializeResponse {
        session_id,
        accepted: true,
        message: Some(format!(
            "Specialization applied: role={}, kit={}",
            bundle.role_context.role_id, bundle.role_context.kit_id
        )),
        activated_capabilities: activated,
    })
}

async fn apply_bundle_to_metadata(
    agent_metadata: &Arc<tokio::sync::RwLock<AgentMetadata>>,
    bundle: &AgentSpecializationBundle,
) {
    let mut meta = agent_metadata.write().await;
    meta.purpose.clone_from(&bundle.persona.purpose);
    meta.model.clone_from(&bundle.persona.model);
    meta.api_keys.clone_from(&bundle.api_tokens);
    // Persona name only swapped if the manifest's intended-agent matches
    // ours — the orchestrator should never send a persona for a different
    // agent, but if it does we keep our own identity rather than masquerade.
    if bundle.persona.name == meta.name || meta.name.is_empty() {
        meta.name.clone_from(&bundle.persona.name);
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_protocol::agent_specialization::{AgentPersona, McpToolGrant, RoleContext};
    use std::collections::HashMap;

    /// Test decryptor that returns a pre-built bundle and verifies the
    /// DID it was given matches what we expect. Skips actual TDF
    /// machinery so tests don't have to set up KAS.
    struct StubDecryptor {
        bundle: AgentSpecializationBundle,
        expected_did: String,
    }

    #[async_trait]
    impl BundleDecryptor for StubDecryptor {
        async fn decrypt(
            &self,
            _tdf_bytes: &[u8],
            recipient_did: &str,
        ) -> Result<AgentSpecializationBundle, String> {
            if recipient_did != self.expected_did {
                return Err(format!(
                    "wrong recipient: expected {}, got {recipient_did}",
                    self.expected_did
                ));
            }
            Ok(self.bundle.clone())
        }
    }

    /// Construct an `ArpDocument` from a JSON literal so this test
    /// module doesn't need a direct dependency on `arkavo-arp`. Mirrors
    /// the canonical shape produced by `derive_arp_for_role`.
    fn arp_doc() -> arkavo_protocol::agent_specialization::ArpDocument {
        let json = serde_json::json!({
            "arp_spec": "0.1.0",
            "adl_ref": { "uri": "urn:test", "document_hash": null },
            "adaptation": {
                "method": "thompson_sampling"
            },
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
                "velocity": {
                    "max_spend_per_minute_usd": 0.01
                }
            }
        });
        serde_json::from_value(json).expect("arp doc")
    }

    fn build_bundle(role: &str, agent_name: &str) -> AgentSpecializationBundle {
        let mut tokens = HashMap::new();
        tokens.insert("OPENAI_API_KEY".to_string(), "sk-applied".to_string());
        AgentSpecializationBundle {
            persona: AgentPersona {
                name: agent_name.into(),
                purpose: format!("Be the {role} role for the campaign kit"),
                model: "gemma-4-9b".into(),
                mcp_tools: vec![McpToolGrant {
                    server: "asset-store".into(),
                    tools: vec!["read".into(), "describe".into()],
                }],
            },
            api_tokens: tokens,
            arp_overlay: arp_doc(),
            role_context: RoleContext {
                kit_id: "kit:campaign:0.1".into(),
                flight_id: "33333333-3333-3333-3333-333333333333".into(),
                role_id: role.into(),
                role_type: "asset_analyst".into(),
                deliverable_schema: serde_json::json!({"type": "object"}),
                handoff_targets: vec!["copy".into()],
            },
        }
    }

    fn metadata_with_did(name: &str, did: &str) -> Arc<tokio::sync::RwLock<AgentMetadata>> {
        Arc::new(tokio::sync::RwLock::new(AgentMetadata {
            name: name.into(),
            purpose: "(unspecialized)".into(),
            model: "(none)".into(),
            did: Some(did.into()),
            ..AgentMetadata::default()
        }))
    }

    fn deps() -> (
        Arc<MetricsCollector>,
        RateLimiter,
        Arc<McpRegistry>,
        Arc<RoleSpecializationStore>,
    ) {
        (
            Arc::new(MetricsCollector::new(false)),
            RateLimiter::new(arkavo_protocol::rate_limit::RateLimitConfig::default()),
            Arc::new(McpRegistry::new()),
            Arc::new(RoleSpecializationStore::default()),
        )
    }

    fn encoded_dummy_bytes() -> String {
        // The stub decryptor ignores the bytes; we only need a valid
        // base64 string so the handler's decode step succeeds.
        base64::engine::general_purpose::STANDARD.encode(b"placeholder-tdf")
    }

    #[tokio::test]
    async fn handle_specialize_decrypts_and_applies_bundle() {
        let did = "did:web:agent-7.arkavo.net";
        let bundle = build_bundle("analyst", "agent-7");
        let agent_metadata = metadata_with_did("agent-7", did);
        let (metrics, limiter, registry, role_store) = deps();
        let decryptor = StubDecryptor {
            bundle: bundle.clone(),
            expected_did: did.to_string(),
        };

        let response = handle_agent_specialize(
            &metrics,
            &limiter,
            &registry,
            &agent_metadata,
            &role_store,
            &decryptor,
            AgentSpecializeRequest {
                requester_id: "did:web:orchestrator.arkavo.net".into(),
                encrypted_bundle: encoded_dummy_bytes(),
                task_context: None,
                session_id: None,
            },
        )
        .await
        .expect("specialize");

        assert!(response.accepted);
        assert!(
            response
                .activated_capabilities
                .iter()
                .any(|c| c == "asset-store:read")
        );

        let meta = agent_metadata.read().await;
        assert_eq!(meta.purpose, "Be the analyst role for the campaign kit");
        assert_eq!(meta.model, "gemma-4-9b");
        assert_eq!(
            meta.api_keys.get("OPENAI_API_KEY").map(String::as_str),
            Some("sk-applied")
        );

        let stored = role_store.get().await.expect("role context stored");
        assert_eq!(stored.role_id, "analyst");
        assert_eq!(stored.kit_id, "kit:campaign:0.1");
    }

    #[tokio::test]
    async fn handle_specialize_rejects_when_decryptor_errors() {
        let our_did = "did:web:agent-7.arkavo.net";
        let bundle = build_bundle("analyst", "agent-9");
        let agent_metadata = metadata_with_did("agent-7", our_did);
        let (metrics, limiter, registry, role_store) = deps();
        // Stub expects DID-9; we'll send DID-7 → wrong recipient error.
        let decryptor = StubDecryptor {
            bundle,
            expected_did: "did:web:agent-9.arkavo.net".to_string(),
        };

        let err = handle_agent_specialize(
            &metrics,
            &limiter,
            &registry,
            &agent_metadata,
            &role_store,
            &decryptor,
            AgentSpecializeRequest {
                requester_id: "did:web:orchestrator.arkavo.net".into(),
                encrypted_bundle: encoded_dummy_bytes(),
                task_context: None,
                session_id: None,
            },
        )
        .await
        .expect_err("wrong DID rejects");

        assert_eq!(err.code(), -32603);
        assert!(role_store.get().await.is_none());
    }

    #[tokio::test]
    async fn handle_specialize_rejects_empty_bundle() {
        let agent_metadata = metadata_with_did("agent-7", "did:web:agent-7.arkavo.net");
        let (metrics, limiter, registry, role_store) = deps();
        let decryptor = UnconfiguredBundleDecryptor;

        let err = handle_agent_specialize(
            &metrics,
            &limiter,
            &registry,
            &agent_metadata,
            &role_store,
            &decryptor,
            AgentSpecializeRequest {
                requester_id: "did:web:orchestrator.arkavo.net".into(),
                encrypted_bundle: String::new(),
                task_context: None,
                session_id: None,
            },
        )
        .await
        .expect_err("empty rejected");

        assert_eq!(err.code(), -32602);
    }

    #[tokio::test]
    async fn handle_specialize_rejects_when_no_decryptor() {
        let agent_metadata = metadata_with_did("agent-7", "did:web:agent-7.arkavo.net");
        let (metrics, limiter, registry, role_store) = deps();
        let decryptor = UnconfiguredBundleDecryptor;

        let err = handle_agent_specialize(
            &metrics,
            &limiter,
            &registry,
            &agent_metadata,
            &role_store,
            &decryptor,
            AgentSpecializeRequest {
                requester_id: "did:web:orchestrator.arkavo.net".into(),
                encrypted_bundle: encoded_dummy_bytes(),
                task_context: None,
                session_id: None,
            },
        )
        .await
        .expect_err("no decryptor wired");

        assert_eq!(err.code(), -32603);
    }
}
