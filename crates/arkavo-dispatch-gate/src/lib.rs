//! The dispatch gate: authn (permit signature, window, proof-of-possession),
//! policy (bundle hash, tool and argument binding), budget (invocations per
//! permit). Local crypto only, no I/O, so it fits inside the 25ms budget
//! documented in `docs/gate-latency-baseline.md`. Sequence integrity and
//! step-up are later stages and plug in before `Allow` is returned.
//!
//! `GateConfig::trusted_issuers` forms one trust domain: authn passes for a
//! permit signed by any listed issuer, with no per-issuer policy and no
//! binding to the permit's `iss` claim yet. `arkavo_permit::decode` must
//! never be used for authn — it checks neither the issuer nor the
//! signature, only claim structure.

use arkavo_permit::{HashAlgorithm, PermitVerifier, verify, verify_invocation_proof};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

pub struct GateConfig {
    pub policy_bundle_hash: Vec<u8>,
    pub hash: HashAlgorithm,
    pub clock: fn() -> i64,
    pub trusted_issuers: Vec<PermitVerifier>,
}

pub struct GateRequest<'a> {
    pub tool_name: &'a str,
    pub arguments: &'a Value,
    pub permit: &'a [u8],
    pub proof: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Authn,
    Policy,
    Budget,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Authn => "authn",
            Self::Policy => "policy",
            Self::Budget => "budget",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateDecision {
    Allow {
        permit_id: [u8; 32],
        subject: String,
    },
    Deny {
        stage: Stage,
        reason: String,
    },
}

struct Usage {
    invocations: u64,
    expires_at: i64,
}

/// Counters are keyed by permit digest. Once the map holds more than
/// `PRUNE_ABOVE` entries, expired counters are pruned first; if the map is
/// still over the threshold after that, entries are evicted in bulk by
/// soonest `expires_at` until at most `PRUNE_TARGET` remain. A caller can
/// mint arbitrarily many permits, so a live (not-yet-expired) counter can be
/// evicted too — but only under that memory pressure, never otherwise.
const PRUNE_ABOVE: usize = 4096;
const PRUNE_TARGET: usize = 3072;

pub struct DispatchGate {
    config: GateConfig,
    usage: Mutex<HashMap<[u8; 32], Usage>>,
}

impl DispatchGate {
    pub fn new(config: GateConfig) -> Self {
        Self {
            config,
            usage: Mutex::new(HashMap::new()),
        }
    }

    pub fn evaluate(&self, request: &GateRequest<'_>) -> GateDecision {
        let now = (self.config.clock)();

        let permit = match verify(request.permit, now, &self.config.trusted_issuers) {
            Ok(permit) => permit,
            Err(error) => return deny(Stage::Authn, error.to_string()),
        };
        if let Err(error) = verify_invocation_proof(
            &permit,
            request.permit,
            request.tool_name,
            request.arguments,
            request.proof,
            self.config.hash,
        ) {
            return deny(Stage::Authn, error.to_string());
        }

        if permit.claims.policy_bundle_hash != self.config.policy_bundle_hash {
            return deny(
                Stage::Policy,
                "permit was issued under a different policy bundle".into(),
            );
        }
        if let Err(error) =
            permit
                .claims
                .verify_invocation(request.tool_name, request.arguments, self.config.hash)
        {
            return deny(Stage::Policy, error.to_string());
        }

        let permit_id = permit_id(request.permit);
        let mut usage = self
            .usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if usage.len() > PRUNE_ABOVE {
            usage.retain(|_, entry| entry.expires_at > now);
        }
        if usage.len() > PRUNE_ABOVE {
            let mut by_expiry: Vec<([u8; 32], i64)> = usage
                .iter()
                .map(|(key, entry)| (*key, entry.expires_at))
                .collect();
            by_expiry.sort_unstable_by_key(|(_, expires_at)| *expires_at);
            let evict = usage.len() - PRUNE_TARGET;
            for (key, _) in by_expiry.into_iter().take(evict) {
                usage.remove(&key);
            }
        }
        let entry = usage.entry(permit_id).or_insert(Usage {
            invocations: 0,
            expires_at: permit.claims.expires_at,
        });
        if entry.invocations >= permit.claims.budget.max_invocations {
            return deny(
                Stage::Budget,
                format!(
                    "invocation budget of {} exhausted",
                    permit.claims.budget.max_invocations
                ),
            );
        }
        // Must stay the last step before `Allow`: no stage runs after this
        // one, so a later addition must not be inserted below it, or it
        // could consume budget on a request this function goes on to deny.
        entry.invocations += 1;
        drop(usage);

        GateDecision::Allow {
            permit_id,
            subject: permit.claims.subject,
        }
    }

    #[cfg(test)]
    fn usage_len(&self) -> usize {
        self.usage
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }
}

pub fn permit_id(permit_cwt: &[u8]) -> [u8; 32] {
    Sha256::digest(permit_cwt).into()
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().cast_signed())
        .unwrap_or(0)
}

fn deny(stage: Stage, reason: String) -> GateDecision {
    GateDecision::Deny { stage, reason }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;
    use arkavo_permit::{
        Budget, PermitClaims, PermitSigner, argument_hash, mint, prove_invocation,
    };
    use serde_json::json;

    const NOW: i64 = 1_700_000_060;
    fn clock() -> i64 {
        NOW
    }

    fn gate() -> (DispatchGate, PermitSigner) {
        let issuer = PermitSigner::Ed25519(AgentKeypair::generate());
        let dispatch_gate = DispatchGate::new(GateConfig {
            policy_bundle_hash: vec![7; 32],
            hash: HashAlgorithm::Sha256,
            clock,
            trusted_issuers: vec![issuer.public_key()],
        });
        (dispatch_gate, issuer)
    }

    fn permit(
        issuer: &PermitSigner,
        holder: &PermitSigner,
        tool: &str,
        args: &serde_json::Value,
        max: u64,
        exp: i64,
        bundle: u8,
    ) -> Vec<u8> {
        let claims = PermitClaims {
            issuer: "edge".into(),
            subject: "agent-1".into(),
            expires_at: exp,
            not_before: NOW - 60,
            issued_at: NOW - 60,
            agent_workload_id: "wl-1".into(),
            policy_bundle_hash: vec![bundle; 32],
            tool_name: tool.into(),
            argument_hash: argument_hash(args, HashAlgorithm::Sha256),
            data_classifications: vec![],
            budget: Budget {
                max_invocations: max,
                token_ceiling: None,
                cost_micro_usd: None,
            },
            sequence_state_hash: vec![9; 32],
            parent_permit: None,
        };
        mint(&claims, issuer, &holder.public_key()).unwrap()
    }

    fn call<'a>(
        tool: &'a str,
        args: &'a serde_json::Value,
        cwt: &'a [u8],
        proof: &'a [u8],
    ) -> GateRequest<'a> {
        GateRequest {
            tool_name: tool,
            arguments: args,
            permit: cwt,
            proof,
        }
    }

    #[test]
    fn valid_permit_and_proof_allow() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 2, NOW + 300, 7);
        let proof = prove_invocation(&holder, &cwt, "merge", &args, HashAlgorithm::Sha256);
        match gate.evaluate(&call("merge", &args, &cwt, &proof)) {
            GateDecision::Allow { subject, .. } => assert_eq!(subject, "agent-1"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn expired_permit_is_denied_at_authn() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({});
        let cwt = permit(&issuer, &holder, "merge", &args, 2, NOW - 1, 7);
        let proof = prove_invocation(&holder, &cwt, "merge", &args, HashAlgorithm::Sha256);
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Authn,
                ..
            }
        ));
    }

    #[test]
    fn replay_with_different_args_is_denied_at_authn() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 2, NOW + 300, 7);
        let proof = prove_invocation(&holder, &cwt, "merge", &args, HashAlgorithm::Sha256);
        let other = json!({"pr": 2});
        assert!(matches!(
            gate.evaluate(&call("merge", &other, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Authn,
                ..
            }
        ));
    }

    #[test]
    fn cross_agent_reuse_is_denied_at_authn() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let intruder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 2, NOW + 300, 7);
        let proof = prove_invocation(&intruder, &cwt, "merge", &args, HashAlgorithm::Sha256);
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Authn,
                ..
            }
        ));
    }

    #[test]
    fn foreign_policy_bundle_is_denied_at_policy() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({});
        let cwt = permit(&issuer, &holder, "merge", &args, 2, NOW + 300, 8);
        let proof = prove_invocation(&holder, &cwt, "merge", &args, HashAlgorithm::Sha256);
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Policy,
                ..
            }
        ));
    }

    #[test]
    fn budget_exhaustion_is_denied_at_budget() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({});
        let cwt = permit(&issuer, &holder, "merge", &args, 1, NOW + 300, 7);
        let proof = prove_invocation(&holder, &cwt, "merge", &args, HashAlgorithm::Sha256);
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Allow { .. }
        ));
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Budget,
                ..
            }
        ));
    }

    #[test]
    fn self_minted_permit_is_denied_at_authn() {
        let (gate, _issuer) = gate();
        let rogue = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&rogue, &rogue, "merge", &args, 2, NOW + 300, 7);
        let proof = prove_invocation(&rogue, &cwt, "merge", &args, HashAlgorithm::Sha256);
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &cwt, &proof)),
            GateDecision::Deny {
                stage: Stage::Authn,
                ..
            }
        ));
    }

    #[test]
    fn usage_table_stays_bounded_under_permit_flood() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let mut last_cwt = Vec::new();
        let mut last_proof = Vec::new();
        for i in 0..4_200i64 {
            let claims = PermitClaims {
                issuer: "edge".into(),
                subject: "agent-1".into(),
                expires_at: NOW + 300,
                not_before: NOW - 60,
                issued_at: NOW - 60 - i,
                agent_workload_id: "wl-1".into(),
                policy_bundle_hash: vec![7; 32],
                tool_name: "merge".into(),
                argument_hash: argument_hash(&args, HashAlgorithm::Sha256),
                data_classifications: vec![],
                budget: Budget {
                    max_invocations: 2,
                    token_ceiling: None,
                    cost_micro_usd: None,
                },
                sequence_state_hash: vec![9; 32],
                parent_permit: None,
            };
            let cwt = mint(&claims, &issuer, &holder.public_key()).unwrap();
            let proof = prove_invocation(&holder, &cwt, "merge", &args, HashAlgorithm::Sha256);
            assert!(matches!(
                gate.evaluate(&call("merge", &args, &cwt, &proof)),
                GateDecision::Allow { .. }
            ));
            last_cwt = cwt;
            last_proof = proof;
        }
        assert!(gate.usage_len() <= 4096);
        assert!(matches!(
            gate.evaluate(&call("merge", &args, &last_cwt, &last_proof)),
            GateDecision::Allow { .. }
        ));
    }

    #[test]
    fn evaluation_stays_under_the_gate_budget() {
        let (gate, issuer) = gate();
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&issuer, &holder, "merge", &args, 10_000, NOW + 300, 7);
        let proof = prove_invocation(&holder, &cwt, "merge", &args, HashAlgorithm::Sha256);
        let mut samples: Vec<u128> = (0..200)
            .map(|_| {
                let start = std::time::Instant::now();
                let _ = gate.evaluate(&call("merge", &args, &cwt, &proof));
                start.elapsed().as_micros()
            })
            .collect();
        samples.sort_unstable();
        let p95 = samples[189];
        assert!(p95 < 25_000, "p95 {p95}µs exceeds the 25ms gate budget");
    }
}
