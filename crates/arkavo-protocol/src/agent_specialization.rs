//! Per-role specialization bundle delivered to a mesh agent so it can
//! become hyperspecialized for a SwarmKit role.
//!
//! The orchestrator builds one [`AgentSpecializationBundle`] per (role,
//! assigned-agent) pair, wraps it as a TDF with a policy that gates
//! decryption on the agent's DID, and ships it via A2A. The receiving
//! agent unwraps the bundle and hot-reloads its persona, API tokens, ARP
//! overlay, and SwarmFlight role context — replacing what an AGENTS.md
//! file would otherwise carry.
//!
//! A bundle bundles four things:
//!
//! 1. [`AgentPersona`] — name, purpose, model, MCP tool grants. Mirrors
//!    the fields the agent would otherwise pick up from AGENTS.md.
//! 2. `api_tokens` — credentials needed for the role's tools, scoped to
//!    just this role. Stored in-memory only on the receiving agent.
//! 3. `arp_overlay` — full [`ArpDocument`] used to instantiate a
//!    per-agent `ArpRuntime` so policy enforcement is in effect for every
//!    tool call this agent makes inside the flight.
//! 4. [`RoleContext`] — flight provenance and handoff metadata, used by
//!    the agent to tag tool outcomes for the right per-role
//!    `DecisionTrace`.

use std::collections::HashMap;

pub use arkavo_arp::ArpDocument;
use serde::{Deserialize, Serialize};

/// Errors raised while serializing or wrapping a bundle.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    #[error("serialize bundle: {0}")]
    Serialize(#[from] serde_json::Error),
    #[error(
        "policy dissemination list does not include expected DID {expected:?} (policy carries {found:?})"
    )]
    DidNotPermitted {
        expected: String,
        found: Vec<String>,
    },
    #[error("decrypted payload is not valid UTF-8: {0}")]
    Utf8(#[from] std::string::FromUtf8Error),
}

/// Full configuration package shipped by the orchestrator to one mesh
/// agent so it becomes specialized for a single SwarmKit role.
///
/// `PartialEq` is intentionally omitted because [`ArpDocument`] does not
/// implement it; round-trip equality is verified via canonical-JSON
/// byte-for-byte comparison instead, which is what the wire format
/// guarantees anyway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpecializationBundle {
    /// Identity-level config: name, purpose, model, MCP tools.
    pub persona: AgentPersona,
    /// API tokens scoped to this role's tool grants. Token name → value.
    /// Stored in-memory only on the receiving agent.
    pub api_tokens: HashMap<String, String>,
    /// ARP document used to spin up a per-agent runtime for this role.
    /// Carries budget, network, tool_use, isolation, etc.
    pub arp_overlay: ArpDocument,
    /// Authored per-MTok model pricing for this flight. When non-empty, the
    /// receiving agent loads these rates as authoritative for its live cost
    /// gate, overriding the built-in static estimate. Travels inside the
    /// TDF-encrypted, DID-bound bundle — same trust envelope as `arp_overlay`.
    /// Flight-global (every role gets the same rates); per-role spend ceilings
    /// live in `arp_overlay`. Default empty for backward compatibility.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub manifest_pricing: Vec<arkavo_budget::provider_costs::PricingEntry>,
    /// Flight provenance the agent uses to tag tool outcomes back to the
    /// right per-role `DecisionTrace` on the orchestrator's flight.
    pub role_context: RoleContext,
}

/// AGENTS.md-equivalent fields for a specialized agent.
///
/// `mcp_tools` mirrors the manifest's `RoleSpec.mcp_tools` — each entry
/// is a server name plus the tools the agent is allowed to invoke on
/// that server.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentPersona {
    pub name: String,
    pub purpose: String,
    pub model: String,
    pub mcp_tools: Vec<McpToolGrant>,
}

/// Single MCP tool grant inside a persona.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct McpToolGrant {
    pub server: String,
    pub tools: Vec<String>,
}

/// SwarmFlight provenance the agent records on every tool outcome it
/// reports back, plus enough context to make handoffs to the next role.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RoleContext {
    pub kit_id: String,
    pub flight_id: String,
    pub role_id: String,
    pub role_type: String,
    /// Schema for the deliverable this role produces. Free-form JSON so
    /// kits can describe whatever shape they need without coupling this
    /// crate to a specific schema language.
    pub deliverable_schema: serde_json::Value,
    /// Role IDs this role hands off to per the manifest's coordination
    /// topology.
    pub handoff_targets: Vec<String>,
}

impl AgentSpecializationBundle {
    /// Canonical JSON serialization for TDF encryption. Sorted keys, no
    /// insignificant whitespace — round-trips bit-for-bit through wrap +
    /// unwrap.
    pub fn to_canonical_json(&self) -> Result<Vec<u8>, BundleError> {
        let value = serde_json::to_value(self)?;
        Ok(canonical_json(&value).into_bytes())
    }

    /// Inverse of [`to_canonical_json`].
    pub fn from_canonical_json(bytes: &[u8]) -> Result<Self, BundleError> {
        let bundle: AgentSpecializationBundle = serde_json::from_slice(bytes)?;
        Ok(bundle)
    }
}

/// JCS (RFC 8785) canonicalization.
///
/// **Source-of-truth: `arkavo_swarmkit::canonical_json`.** This is a
/// byte-for-byte copy of the writer there, kept in lockstep so bundle
/// signing and `kit.id` computation produce identical canonical bytes
/// for identical input. The parity is verified by the
/// `parity_with_arkavo_swarmkit_canonical_json` test below — that test
/// fails if the two implementations drift, which is the signal to
/// re-sync this copy.
///
/// We inline rather than depend on `arkavo-swarmkit` directly: nine
/// workspace crates depend on `arkavo-protocol`, and pulling
/// `arkavo-swarmkit` in transitively would invert the natural
/// layering (swarmkit sits above protocol). The previous
/// implementation built on `serde_json::Map`'s `BTreeMap` backing for
/// sort-on-insert and used `serde_json::to_vec` for output — both of
/// which silently break if any workspace dep enables
/// `serde_json/preserve_order`. The hand-written writer here is
/// feature-flag-independent.
fn canonical_json(value: &serde_json::Value) -> String {
    let mut buf = String::new();
    write_canonical(value, &mut buf);
    buf
}

fn write_canonical(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        serde_json::Value::Number(n) => out.push_str(&n.to_string()),
        serde_json::Value::String(s) => write_json_string(s, out),
        serde_json::Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            out.push('{');
            for (i, k) in keys.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_json_string(k, out);
                out.push(':');
                write_canonical(&map[*k], out);
            }
            out.push('}');
        }
    }
}

fn write_json_string(s: &str, out: &mut String) {
    use std::fmt::Write as _;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Verify that `expected_did` appears in `dissemination`.
///
/// Used by the receiving agent as a pre-flight check before invoking
/// the KAS rewrap — distinguishes "this bundle is not for me" from
/// "the KAS denied me".
pub fn verify_dissemination_includes(
    dissemination: &[String],
    expected_did: &str,
) -> Result<(), BundleError> {
    if dissemination.iter().any(|d| d == expected_did) {
        Ok(())
    } else {
        Err(BundleError::DidNotPermitted {
            expected: expected_did.to_string(),
            found: dissemination.to_vec(),
        })
    }
}

#[cfg(feature = "kas")]
mod tdf_io {
    use super::{AgentSpecializationBundle, BundleError, verify_dissemination_includes};
    use arkavo_tdf::{Policy, PolicyBuilder, TdfDecryptor, TdfEncryptor, TdfError, TdfManifest};

    /// Errors raised while wrapping or unwrapping a bundle.
    #[derive(Debug, thiserror::Error)]
    pub enum BundleTdfError {
        #[error("serialize bundle: {0}")]
        Bundle(#[from] BundleError),
        #[error("encrypt bundle: {0}")]
        Encrypt(TdfError),
        #[error("decrypt bundle: {0}")]
        Decrypt(TdfError),
        #[error("build policy: {0}")]
        Policy(TdfError),
    }

    /// Build a TDF Policy that gates bundle decryption on the recipient agent's DID.
    ///
    /// The policy carries one attribute
    /// (`https://attr.arkavo.com/role/specialist`) so the KAS knows
    /// the payload is intended for a SwarmKit role; the recipient DID
    /// is added to the dissemination list so only that agent can rewrap.
    pub fn bundle_policy_for_recipient(recipient_did: &str) -> Result<Policy, BundleTdfError> {
        PolicyBuilder::new()
            .id(&format!("swarmkit:bundle:{recipient_did}"))
            .attribute_single("https://attr.arkavo.com/role", "specialist")
            .add_dissemination(recipient_did)
            .build()
            .map_err(BundleTdfError::Policy)
    }

    /// Wrap a bundle into a TDF envelope with a policy bound to
    /// `recipient_did`. The orchestrator calls this once per assigned
    /// role and ships the resulting manifest via A2A.
    pub async fn wrap_bundle<E: TdfEncryptor>(
        bundle: &AgentSpecializationBundle,
        encryptor: &E,
        recipient_did: &str,
    ) -> Result<TdfManifest, BundleTdfError> {
        let canonical = bundle.to_canonical_json()?;
        let policy = bundle_policy_for_recipient(recipient_did)?;
        encryptor
            .encrypt(&canonical, &policy)
            .await
            .map_err(BundleTdfError::Encrypt)
    }

    /// Receiving-agent flow. Performs the dissemination pre-flight
    /// check first (so a misrouted bundle fails fast with a distinct
    /// error), then decrypts and parses.
    pub async fn unwrap_bundle<D: TdfDecryptor>(
        tdf: &TdfManifest,
        decryptor: &D,
        expected_did: &str,
    ) -> Result<AgentSpecializationBundle, BundleTdfError> {
        let policy_b64 = &tdf.encryption_information.policy;
        let policy_json =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, policy_b64)
                .map_err(|e| {
                    BundleTdfError::Bundle(BundleError::Serialize(serde_json::Error::io(
                        std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()),
                    )))
                })?;
        let policy: Policy =
            serde_json::from_slice(&policy_json).map_err(|e| BundleTdfError::Bundle(e.into()))?;
        verify_dissemination_includes(&policy.dissemination, expected_did)
            .map_err(BundleTdfError::Bundle)?;
        let plaintext = decryptor
            .decrypt(tdf)
            .await
            .map_err(BundleTdfError::Decrypt)?;
        AgentSpecializationBundle::from_canonical_json(&plaintext).map_err(BundleTdfError::Bundle)
    }

    #[cfg(test)]
    #[allow(clippy::disallowed_methods)]
    mod tests {
        use super::super::tests::sample_bundle;
        use super::{BundleError, BundleTdfError, unwrap_bundle, wrap_bundle};
        use arkavo_tdf::testing::MockTdfService;

        #[tokio::test]
        async fn bundle_round_trips_through_tdf() {
            let bundle = sample_bundle();
            let svc = MockTdfService::default();
            let did = "did:web:agent-7.arkavo.net";
            let tdf = wrap_bundle(&bundle, &svc, did).await.expect("wrap");
            let recovered = unwrap_bundle(&tdf, &svc, did).await.expect("unwrap");
            // Bytes match → fields match.
            let original_bytes = bundle.to_canonical_json().expect("ser");
            let recovered_bytes = recovered.to_canonical_json().expect("ser");
            assert_eq!(original_bytes, recovered_bytes);
        }

        #[tokio::test]
        async fn unwrap_with_wrong_did_fails() {
            let bundle = sample_bundle();
            let svc = MockTdfService::default();
            let tdf = wrap_bundle(&bundle, &svc, "did:web:agent-7.arkavo.net")
                .await
                .expect("wrap");
            let err = unwrap_bundle(&tdf, &svc, "did:web:agent-8.arkavo.net")
                .await
                .expect_err("wrong DID");
            match err {
                BundleTdfError::Bundle(BundleError::DidNotPermitted { expected, .. }) => {
                    assert_eq!(expected, "did:web:agent-8.arkavo.net");
                }
                other => panic!("wrong error: {other:?}"),
            }
        }
    }
}

#[cfg(feature = "kas")]
pub use tdf_io::{BundleTdfError, bundle_policy_for_recipient, unwrap_bundle, wrap_bundle};

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_arp::ArpDocument;
    use arkavo_arp::adaptation::{Adaptation, AdaptationMethod};
    use arkavo_arp::constraints::{Budget, BudgetExhaustionAction, Velocity};
    use arkavo_arp::feedback::{
        DecayStrategy, FeedbackLoops, ImmediateFeedback, PolicyCacheConfig, QualityFailureAction,
        QualityGate, QualityMetric, ShortTermFeedback,
    };
    use arkavo_arp::model::AdlRef;

    pub(super) fn sample_arp() -> ArpDocument {
        ArpDocument {
            arp_spec: "0.1.0".into(),
            adl_ref: AdlRef {
                uri: Some("urn:swarmkit:role:analyst".into()),
                document_hash: None,
            },
            integrity: None,
            adaptation: Adaptation {
                method: AdaptationMethod::ThompsonSampling,
                parameters: None,
                cold_start: None,
                prior_management: None,
                signal_separation: None,
            },
            feedback_loops: FeedbackLoops {
                immediate: ImmediateFeedback {
                    quality_gate: QualityGate {
                        threshold_default: 0.7,
                        metric: QualityMetric::Composite,
                        on_failure: QualityFailureAction::UpdatePriorAndLog,
                        threshold_overrides: None,
                    },
                },
                short_term: ShortTermFeedback {
                    policy_cache: PolicyCacheConfig {
                        default_ttl_sec: 3600,
                        decay_strategy: DecayStrategy::Exponential,
                        decay_half_life_sec: Some(86_400),
                        human_source_exempt_from_decay: None,
                        incident_source_quarantine_sec: None,
                    },
                },
                gossip: None,
                consolidation: None,
                resilience: None,
            },
            precedence: None,
            cognitive: None,
            execution: None,
            data_sovereignty: None,
            network: None,
            budget: Budget {
                task_ceiling_usd: 0.05,
                on_exhaustion: BudgetExhaustionAction::HaltAndReport,
                degradation_chain: None,
                alert_threshold_pct: None,
                velocity: Velocity {
                    max_spend_per_minute_usd: 0.01,
                    max_tool_calls_per_minute: None,
                    max_tokens_per_minute: None,
                },
                per_layer: None,
                rate_limiting: None,
                accounting: None,
            },
            escalation: None,
            quarantine: None,
            hitl: None,
            session: None,
            state_storage: None,
            observability: None,
            proposal_policy: None,
            metadata: None,
        }
    }

    pub(super) fn sample_bundle() -> AgentSpecializationBundle {
        let mut tokens = HashMap::new();
        tokens.insert("OPENAI_API_KEY".to_string(), "sk-test-1".to_string());
        AgentSpecializationBundle {
            persona: AgentPersona {
                name: "Asset Analyst".to_string(),
                purpose: "Summarize source assets and extract selling points".to_string(),
                model: "gemma-4-9b".to_string(),
                mcp_tools: vec![McpToolGrant {
                    server: "asset-store".to_string(),
                    tools: vec!["read".to_string(), "describe".to_string()],
                }],
            },
            api_tokens: tokens,
            arp_overlay: sample_arp(),
            manifest_pricing: Vec::new(),
            role_context: RoleContext {
                kit_id: "kit:campaign-kit:0.1.0".to_string(),
                flight_id: "11111111-1111-1111-1111-111111111111".to_string(),
                role_id: "analyst".to_string(),
                role_type: "asset_analyst".to_string(),
                deliverable_schema: serde_json::json!({
                    "type": "object",
                    "properties": {"summary": {"type": "string"}}
                }),
                handoff_targets: vec!["copy".to_string()],
            },
        }
    }

    #[test]
    fn bundle_round_trips_through_canonical_json() {
        let original = sample_bundle();
        let bytes = original.to_canonical_json().expect("serialize");
        let recovered =
            AgentSpecializationBundle::from_canonical_json(&bytes).expect("deserialize");
        // ArpDocument has no PartialEq; re-serialize the recovered bundle
        // and compare wire bytes — round-trip equality is exactly the
        // property that matters for transport.
        let recovered_bytes = recovered.to_canonical_json().expect("re-serialize");
        assert_eq!(bytes, recovered_bytes);
        assert_eq!(original.persona, recovered.persona);
        assert_eq!(original.api_tokens, recovered.api_tokens);
        assert_eq!(original.role_context, recovered.role_context);
    }

    #[test]
    fn bundle_without_manifest_pricing_deserializes_as_empty() {
        // #635 backward compat: a bundle serialized before manifest_pricing
        // existed (or with an empty table) must deserialize cleanly, defaulting
        // the field to an empty vec. Build a real bundle, serialize it, then
        // assert deserialization works whether or not pricing is present.
        let bundle = sample_bundle(); // empty manifest_pricing
        let json = serde_json::to_string(&bundle).expect("serialize");
        // skip_serializing_if = "Vec::is_empty" → field absent from JSON.
        assert!(
            !json.contains("manifest_pricing"),
            "empty pricing must be skipped on serialize (preserves kit.id)"
        );
        let recovered: AgentSpecializationBundle =
            serde_json::from_str(&json).expect("deserialize without manifest_pricing");
        assert!(
            recovered.manifest_pricing.is_empty(),
            "absent manifest_pricing must default to empty"
        );
    }

    #[test]
    fn canonical_json_is_deterministic() {
        let bundle = sample_bundle();
        let a = bundle.to_canonical_json().expect("serialize a");
        let b = bundle.to_canonical_json().expect("serialize b");
        assert_eq!(a, b);
    }

    #[test]
    fn verify_dissemination_accepts_listed_did() {
        let dissem = vec![
            "did:web:agent-7.arkavo.net".to_string(),
            "did:web:agent-9.arkavo.net".to_string(),
        ];
        verify_dissemination_includes(&dissem, "did:web:agent-7.arkavo.net").expect("present");
    }

    #[test]
    fn verify_dissemination_rejects_unlisted_did() {
        let dissem = vec!["did:web:agent-7.arkavo.net".to_string()];
        let err = verify_dissemination_includes(&dissem, "did:web:agent-8.arkavo.net")
            .expect_err("unlisted");
        match err {
            BundleError::DidNotPermitted { expected, found } => {
                assert_eq!(expected, "did:web:agent-8.arkavo.net");
                assert_eq!(found, vec!["did:web:agent-7.arkavo.net".to_string()]);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn verify_dissemination_rejects_empty_list() {
        let err =
            verify_dissemination_includes(&[], "did:web:agent-7.arkavo.net").expect_err("empty");
        assert!(matches!(err, BundleError::DidNotPermitted { .. }));
    }

    /// Reviewer M-1: bundle canonicalization must be byte-identical to
    /// `arkavo_swarmkit::canonical_json` (the production manifest
    /// canonicalizer). Anything else makes the docstring claim a lie
    /// and breaks cross-verification of bundle/manifest signatures.
    /// Test fixtures cover every JCS edge case the writer handles.
    #[test]
    fn parity_with_arkavo_swarmkit_canonical_json() {
        let cases = [
            serde_json::json!(null),
            serde_json::json!(true),
            serde_json::json!(false),
            serde_json::json!(0),
            serde_json::json!(-42),
            serde_json::json!(3.14),
            serde_json::json!(""),
            serde_json::json!("plain"),
            serde_json::json!("a\nb\tc\"d\\e"),
            serde_json::json!("non-ascii: ümlaut, café, 漢字, 🦀"),
            serde_json::json!("\u{0001}\u{001f}"),
            serde_json::json!([]),
            serde_json::json!({}),
            serde_json::json!([1, "two", null, true, [3.0, {}]]),
            serde_json::json!({"b": 1, "a": 2, "z": {"y": [9, 8], "x": null}}),
            serde_json::json!({"slash": "a/b/c", "quote": "he said \"hi\""}),
        ];
        for v in &cases {
            let ours = canonical_json(v);
            let theirs = arkavo_swarmkit::canonical_json(v);
            assert_eq!(
                ours, theirs,
                "canonicalization drift between arkavo-protocol and arkavo-swarmkit on input: {v}"
            );
        }
    }
}
