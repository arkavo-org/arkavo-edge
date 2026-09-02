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

use crate::policy::{CallContext, Decision, PolicyHook};
use arkavo_dispatch_gate::{DispatchGate, GateDecision, GateRequest};
use arkavo_observability::subsystem_timing::global_timing;
use async_trait::async_trait;
use std::time::Instant;

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

#[async_trait]
impl PolicyHook for PermitPolicy {
    async fn evaluate(&self, ctx: &CallContext) -> Decision {
        let started = Instant::now();
        let decision = match (&ctx.permit, &ctx.proof) {
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
                reason: "authn: tools/call carries no permit and proof in _meta.arkavo".into(),
            },
        };
        global_timing()
            .dispatch_gate
            .record(started.elapsed().as_millis() as u64);
        decision
    }
}
