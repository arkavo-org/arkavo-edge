//! Production [`BundleEncryptor`] for the SwarmKit apply pipeline.
//!
//! `arkavo-tdf-iroh` is deliberately transport-only ("encryption composed at
//! the application layer"); the orchestrator *is* that application layer — it
//! pairs the [`BundleEncryptor`](crate::BundleEncryptor) (crypto) with the
//! [`BundleShipper`](crate::BundleShipper) (Iroh data plane) inside
//! [`apply_kit`](crate::apply_kit). This module supplies the KAS-backed
//! encryptor that wraps each role's specialization bundle in TDF, bound by
//! policy to the recipient agent's DID, before it is staged on the data plane.
//!
//! Only compiled with the `kas` feature: without a KAS-backed
//! [`TdfEncryptor`] there is no real encryption, and the apply pipeline must
//! not ship token-bearing bundles in the clear.

use arkavo_protocol::AgentSpecializationBundle;
use arkavo_protocol::agent_specialization::wrap_bundle;
use arkavo_tdf::{OpenTdfService, TdfEncryptor};
use async_trait::async_trait;

use crate::swarmkit_apply::BundleEncryptor;

/// KAS-backed [`BundleEncryptor`].
///
/// Wraps each bundle in a TDF envelope whose policy disseminates only to the
/// recipient agent's DID, then serialises the manifest to the bytes
/// [`apply_kit`](crate::apply_kit) ships.
///
/// Generic over the [`TdfEncryptor`] so production wires a real
/// [`OpenTdfService`] while tests can inject an offline mock. Construct with
/// [`TdfBundleEncryptor::new`] for production (KAS URL) or
/// [`TdfBundleEncryptor::with_encryptor`] to supply any encryptor.
pub struct TdfBundleEncryptor<E: TdfEncryptor = OpenTdfService> {
    encryptor: E,
}

impl TdfBundleEncryptor<OpenTdfService> {
    /// Build an encryptor that mints TDF keys against `kas_url`. Encryption is
    /// offline — the URL is embedded in each manifest so the *recipient* can
    /// rewrap against that KAS; no KAS round-trip happens at wrap time.
    pub fn new(kas_url: impl Into<String>) -> Self {
        Self {
            encryptor: OpenTdfService::with_kas_url(kas_url),
        }
    }
}

impl<E: TdfEncryptor> TdfBundleEncryptor<E> {
    /// Wrap an explicit [`TdfEncryptor`] (e.g. a mock in tests).
    pub fn with_encryptor(encryptor: E) -> Self {
        Self { encryptor }
    }
}

#[async_trait]
impl<E: TdfEncryptor> BundleEncryptor for TdfBundleEncryptor<E> {
    async fn wrap(
        &self,
        bundle: &AgentSpecializationBundle,
        recipient_did: &str,
    ) -> Result<Vec<u8>, String> {
        let manifest = wrap_bundle(bundle, &self.encryptor, recipient_did)
            .await
            .map_err(|e| format!("wrap bundle for {recipient_did:?}: {e}"))?;
        serde_json::to_vec(&manifest).map_err(|e| format!("serialize TDF manifest: {e}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::agent_specialization::{AgentPersona, McpToolGrant, RoleContext};
    use arkavo_tdf::TdfManifest;

    fn sample_bundle() -> AgentSpecializationBundle {
        // A minimal-but-valid ARP overlay; only needs to round-trip through
        // canonical JSON, which the bundle wrap/serialize path exercises.
        let arp_overlay = serde_json::from_value(serde_json::json!({
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
        }))
        .expect("valid arp document");

        AgentSpecializationBundle {
            persona: AgentPersona {
                name: "reviewer-agent".into(),
                purpose: "Review pull requests".into(),
                model: "gemma-4-9b".into(),
                mcp_tools: vec![McpToolGrant {
                    server: "arkavo-github".into(),
                    tools: vec!["github_pr_watch".into()],
                }],
            },
            api_tokens: Default::default(),
            arp_overlay,
            role_context: RoleContext {
                kit_id: "kit:demo:0.1.0".into(),
                flight_id: "44444444-4444-4444-4444-444444444444".into(),
                role_id: "pr_reviewer".into(),
                role_type: "critic".into(),
                deliverable_schema: serde_json::json!({ "type": "object" }),
                handoff_targets: vec![],
            },
        }
    }

    // Encryption is offline (OpenTdfService embeds the KAS URL rather than
    // contacting it), so this wraps a real bundle without a live KAS and
    // proves the adapter produces a parseable TDF manifest bound to the
    // recipient's KAS. Recipient-dissemination round-trip is covered by
    // arkavo-protocol's swarmkit_bundle_round_trip test.
    #[tokio::test]
    async fn wraps_bundle_into_parseable_tdf_manifest() {
        let kas_url = "https://kas.example.com";
        let encryptor = TdfBundleEncryptor::new(kas_url);
        let bytes = encryptor
            .wrap(&sample_bundle(), "did:web:agent-a.arkavo.net")
            .await
            .expect("offline TDF wrap should succeed");

        assert!(!bytes.is_empty(), "wrapped bundle must not be empty");
        let manifest: TdfManifest =
            serde_json::from_slice(&bytes).expect("wrapped bytes must be a TDF manifest");
        let key_access = manifest
            .encryption_information
            .key_access
            .first()
            .expect("manifest must carry a key-access object");
        assert_eq!(
            key_access.url, kas_url,
            "manifest must point the recipient at the configured KAS for rewrap"
        );
    }
}
