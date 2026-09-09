//! The `PolicyHook` that runs the dispatch gate on every `tools/call` and
//! records its latency against the 25ms budget.
//!
//! Samples go to the `dispatch_gate` tracker of this process's
//! `arkavo-observability` registry, which is process-local: they are visible
//! to an embedder hosting the proxy in-process, and nowhere else — a
//! standalone `arkavo mcp proxy` has no sampler reading them. Recording is in
//! whole milliseconds, so a sub-millisecond evaluation, which is the normal
//! case, records as 0 ms; the `gate_latency` bench is where sub-ms precision
//! lives.

use crate::policy::{CallContext, Credential, Decision, ForwardOutcome, PolicyHook};
use arkavo_dispatch_gate::{DispatchGate, GateDecision, GateRequest};
use arkavo_observability::subsystem_timing::global_timing;
use async_trait::async_trait;
use std::time::Instant;
use tracing::warn;

/// Policy hook that only allows a `tools/call` bound to a valid permit and
/// a matching proof-of-possession, per [`DispatchGate::evaluate`].
pub struct PermitPolicy {
    gate: DispatchGate,
}

impl PermitPolicy {
    /// Wrap `gate` as a [`PolicyHook`].
    pub fn new(gate: DispatchGate) -> Self {
        Self { gate }
    }
}

/// Why a call cannot be gated at all, said precisely enough for a client to
/// fix it. All of these are `authn:` refusals, like every other refusal that
/// happens before the permit is checked.
fn missing_credentials(permit: &Credential, proof: &Credential) -> String {
    let reason = match (permit, proof) {
        (Credential::Oversized, _) => "permit is longer than any permit can be",
        (_, Credential::Oversized) => "pop is longer than a proof of possession can be",
        (Credential::Undecodable, _) | (_, Credential::Undecodable) => {
            "permit or pop is not base64url"
        }
        (Credential::Present(_), Credential::Absent) => "permit present without pop",
        (Credential::Absent, Credential::Present(_)) => "pop present without permit",
        _ => "tools/call carries no permit and proof in _meta.arkavo",
    };
    format!("authn: {reason}")
}

#[async_trait]
impl PolicyHook for PermitPolicy {
    async fn evaluate(&self, ctx: &CallContext) -> Decision {
        let started = Instant::now();
        let decision = match (ctx.permit.bytes(), ctx.proof.bytes()) {
            (Some(permit), Some(proof)) => {
                let request = GateRequest {
                    tool_name: &ctx.tool_name,
                    arguments: &ctx.arguments,
                    permit,
                    proof,
                };
                match self.gate.evaluate(&request) {
                    GateDecision::Allow { .. } => Decision::Allow,
                    GateDecision::Deny { stage, reason } => Decision::Deny {
                        reason: format!("{stage}: {reason}"),
                    },
                }
            }
            _ => Decision::Deny {
                reason: missing_credentials(&ctx.permit, &ctx.proof),
            },
        };
        global_timing()
            .dispatch_gate
            .record(started.elapsed().as_millis() as u64);
        decision
    }

    /// Give the permit back the invocation the gate spent admitting a call
    /// the upstream never received — and only then.
    ///
    /// A call that reached the upstream keeps its invocation even though the
    /// proxy has no answer for it. A timeout is the case that matters: the
    /// request was delivered and a slow tool goes on running after the wait
    /// is abandoned, so refunding it would let any tool slower than the
    /// request timeout be invoked without ever depleting `max_invocations`.
    async fn on_forward_failed(&self, ctx: &CallContext, outcome: ForwardOutcome) {
        match outcome {
            ForwardOutcome::NotDelivered => {
                if let Some(permit) = ctx.permit.bytes() {
                    self.gate.refund_invocation(permit);
                }
            }
            ForwardOutcome::MaybeExecuted => warn!(
                tool = %ctx.tool_name,
                "budget retained: the call reached the upstream and may have executed"
            ),
        }
    }
}

#[cfg(test)]
// The `#[tokio::test]` macro expands to `Runtime::block_on`, which
// `.clippy.toml` disallows outside test code.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;
    use arkavo_dispatch_gate::GateConfig;
    use arkavo_permit::{
        Budget, HashAlgorithm, PermitClaims, PermitSigner, argument_hash, decode, mint,
        prove_invocation,
    };
    use arkavo_test_macros::spec;
    use serde_json::{Value, json};

    const NOW: i64 = 1_700_000_060;
    fn clock() -> i64 {
        NOW
    }

    /// A gate, a permit for `echo` with `budget` invocations, and the
    /// context a client would send to exercise it.
    fn gated_call(budget: u64) -> (PermitPolicy, CallContext) {
        let issuer = PermitSigner::Ed25519(AgentKeypair::generate());
        let holder = PermitSigner::Ed25519(AgentKeypair::generate());
        let arguments = json!({"n": 1});
        let claims = PermitClaims {
            issuer: "edge".into(),
            subject: "agent-1".into(),
            expires_at: NOW + 300,
            not_before: NOW - 60,
            issued_at: NOW - 60,
            agent_workload_id: "wl-1".into(),
            policy_bundle_hash: vec![7; 32],
            tool_name: "echo".into(),
            argument_hash: argument_hash(&arguments, HashAlgorithm::Sha256),
            data_classifications: vec![],
            budget: Budget {
                max_invocations: budget,
                token_ceiling: None,
                cost_micro_usd: None,
            },
            sequence_state_hash: vec![9; 32],
            parent_permit: None,
        };
        let cwt = mint(&claims, &issuer, &holder.public_key()).expect("mint");
        let permit_id = decode(&cwt).expect("decode").id;
        let proof = prove_invocation(
            &holder,
            &permit_id,
            "echo",
            &arguments,
            HashAlgorithm::Sha256,
        );

        let policy = PermitPolicy::new(DispatchGate::new(GateConfig {
            policy_bundle_hash: vec![7; 32],
            hash: HashAlgorithm::Sha256,
            clock,
            trusted_issuers: vec![issuer.public_key()],
        }));
        let ctx = CallContext {
            tool_name: "echo".to_string(),
            arguments,
            permit: Credential::Present(cwt),
            proof: Credential::Present(proof),
        };
        (policy, ctx)
    }

    fn ctx_with(permit: Credential, proof: Credential) -> CallContext {
        CallContext {
            tool_name: "echo".to_string(),
            arguments: Value::Null,
            permit,
            proof,
        }
    }

    async fn deny_reason(permit: Credential, proof: Credential) -> String {
        let (policy, _) = gated_call(1);
        match policy.evaluate(&ctx_with(permit, proof)).await {
            Decision::Deny { reason } => reason,
            Decision::Allow => panic!("a call without both credentials must be denied"),
        }
    }

    /// "No permit and proof" is the wrong thing to tell a client that sent
    /// both and mis-encoded one, or that sent only one of them.
    #[tokio::test]
    #[spec("PDG-009")]
    async fn each_way_of_arriving_without_usable_credentials_says_which() {
        assert_eq!(
            deny_reason(Credential::Absent, Credential::Absent).await,
            "authn: tools/call carries no permit and proof in _meta.arkavo"
        );
        assert_eq!(
            deny_reason(Credential::Undecodable, Credential::Present(vec![1])).await,
            "authn: permit or pop is not base64url"
        );
        assert_eq!(
            deny_reason(Credential::Present(vec![1]), Credential::Undecodable).await,
            "authn: permit or pop is not base64url"
        );
        assert_eq!(
            deny_reason(Credential::Oversized, Credential::Present(vec![1])).await,
            "authn: permit is longer than any permit can be"
        );
        // The two fields have separate bounds, so they get separate reasons:
        // a proof is one signature and nothing near a permit's size.
        assert_eq!(
            deny_reason(Credential::Present(vec![1]), Credential::Oversized).await,
            "authn: pop is longer than a proof of possession can be"
        );
        assert_eq!(
            deny_reason(Credential::Present(vec![1]), Credential::Absent).await,
            "authn: permit present without pop"
        );
        assert_eq!(
            deny_reason(Credential::Absent, Credential::Present(vec![1])).await,
            "authn: pop present without permit"
        );
    }

    /// A call the upstream never received must not cost the permit an
    /// invocation: without the refund a budget of one is spent by a single
    /// transport failure.
    #[tokio::test]
    #[spec("PDG-006")]
    async fn an_undelivered_forward_returns_the_invocation() {
        let (policy, ctx) = gated_call(1);

        assert_eq!(policy.evaluate(&ctx).await, Decision::Allow);
        // Without a refund the budget is now gone.
        match policy.evaluate(&ctx).await {
            Decision::Deny { reason } => assert!(reason.contains("budget"), "reason: {reason}"),
            Decision::Allow => panic!("a budget of one must not admit two calls"),
        }

        policy
            .on_forward_failed(&ctx, ForwardOutcome::NotDelivered)
            .await;
        assert_eq!(
            policy.evaluate(&ctx).await,
            Decision::Allow,
            "the invocation the failed dispatch gave back must be spendable"
        );
    }

    /// The other side of the rule, and the one that carries the security
    /// weight: a call that reached the upstream keeps its invocation even
    /// though no response came back. Refunding a timeout would let any tool
    /// slower than the request timeout be invoked over and over on a budget
    /// of one.
    #[tokio::test]
    #[spec("PDG-006")]
    async fn a_forward_that_may_have_executed_keeps_the_invocation() {
        let (policy, ctx) = gated_call(1);

        assert_eq!(policy.evaluate(&ctx).await, Decision::Allow);
        policy
            .on_forward_failed(&ctx, ForwardOutcome::MaybeExecuted)
            .await;

        match policy.evaluate(&ctx).await {
            Decision::Deny { reason } => assert!(reason.contains("budget"), "reason: {reason}"),
            Decision::Allow => {
                panic!("a call that may have run upstream must not get its invocation back")
            }
        }
    }

    /// The refund only ever credits the permit the call carried, and a call
    /// that never had a usable permit refunds nothing at all.
    #[tokio::test]
    async fn a_failed_forward_without_a_permit_does_nothing() {
        let (policy, ctx) = gated_call(1);
        assert_eq!(policy.evaluate(&ctx).await, Decision::Allow);

        policy
            .on_forward_failed(
                &ctx_with(Credential::Absent, Credential::Absent),
                ForwardOutcome::NotDelivered,
            )
            .await;
        policy
            .on_forward_failed(
                &ctx_with(
                    Credential::Present(b"not a permit".to_vec()),
                    Credential::Absent,
                ),
                ForwardOutcome::NotDelivered,
            )
            .await;

        match policy.evaluate(&ctx).await {
            Decision::Deny { reason } => assert!(reason.contains("budget"), "reason: {reason}"),
            Decision::Allow => panic!("neither refund names this permit"),
        }
    }
}
