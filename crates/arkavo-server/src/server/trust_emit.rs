//! Emit MCP-T `behavior.trace` events from the conductor's tool loop.
//!
//! This is the load-bearing v0.2 emission point: every completed task that
//! ran through the agentic loop produces one `behavior.trace` event whose
//! `fidelity_ratio` is the exact `declared / total` ratio of tool calls.
//!
//! Keeping the assembly here (rather than inlining it in the tool loops)
//! lets the loops stay focused on execution and lets the emit logic live
//! alongside the rest of the trust wiring (`trust_init`, `trust_sync`).

use arkavo_trust::TrustService;
use arkavo_trust::dimensions;
use arkavo_trust::event_types;
use arkavo_trust::events::{BehaviorTrace, ToolCallRecord};
use tracing::warn;

use super::conductor_tool_loop::ToolCallObservation;

/// Build a `behavior.trace` payload from the loop's observations and emit it
/// via the trust service. Failures are logged and swallowed — trust emission
/// must never break a successful task.
pub(super) fn emit_behavior_trace(
    trust_service: &TrustService,
    subject_id: &str,
    contract_id: &str,
    observations: &[ToolCallObservation],
    duration_ms: u64,
) {
    if observations.is_empty() {
        // No tool calls means no fidelity signal to publish. Skipping keeps
        // the score's evidence count honest for agents that produced text-only
        // responses.
        return;
    }

    let total = observations.len() as u32;
    let undeclared = observations.iter().filter(|o| !o.declared).count() as u32;
    let declared = total - undeclared;
    let fidelity_ratio = if total == 0 {
        1.0
    } else {
        declared as f64 / total as f64
    };

    let tool_calls: Vec<ToolCallRecord> = observations
        .iter()
        .map(|o| ToolCallRecord {
            tool_name: o.tool_name.clone(),
            timestamp: o.timestamp,
            duration_ms: o.duration_ms,
            declared: o.declared,
        })
        .collect();

    let trace = BehaviorTrace {
        trace_id: format!("trc_{}", uuid::Uuid::new_v4()),
        contract_id: contract_id.to_string(),
        tool_calls,
        // ResourceAccessRecord and SideEffectRecord are populated by future
        // hooks (file/network observers); the tool-call view alone is enough
        // for fidelity_ratio per v0.2.0 §6.4.1.
        resources_accessed: Vec::new(),
        side_effects: Vec::new(),
        duration_ms,
        peak_memory_bytes: None,
        total_tool_calls: total,
        undeclared_tool_calls: undeclared,
        total_resources: 0,
        undeclared_resources: 0,
        fidelity_ratio,
    };

    let payload = match serde_json::to_value(&trace) {
        Ok(v) => v,
        Err(e) => {
            warn!("MCP-T behavior.trace serialization failed: {e}");
            return;
        }
    };

    let dims_affected = vec![
        dimensions::BEHAVIORAL_FIDELITY.to_string(),
        dimensions::PERFORMANCE.to_string(),
        dimensions::CONSISTENCY.to_string(),
    ];

    if let Err(e) = trust_service.emit_event(
        subject_id,
        event_types::BEHAVIOR_TRACE,
        payload,
        dims_affected,
    ) {
        warn!("MCP-T behavior.trace publish failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;
    use chrono::Utc;

    fn obs(tool: &str, declared: bool) -> ToolCallObservation {
        ToolCallObservation {
            tool_name: tool.to_string(),
            timestamp: Utc::now(),
            duration_ms: 10,
            declared,
        }
    }

    #[test]
    fn empty_observations_skip_emission() {
        let svc = TrustService::new(AgentKeypair::generate(), 3600);
        // Empty observations must not produce an event — verified by checking
        // history is empty afterward.
        emit_behavior_trace(&svc, "agent-1", "ctr_x", &[], 0);
        let hist = svc.history(&arkavo_trust::TrustHistoryRequest {
            subject_id: "agent-1".to_string(),
            event_types: Some(vec![event_types::BEHAVIOR_TRACE.to_string()]),
            after: None,
            before: None,
            limit: None,
            cursor: None,
        });
        assert_eq!(hist.events.len(), 0);
    }

    #[test]
    fn fidelity_ratio_matches_declared_fraction() {
        let svc = TrustService::new(AgentKeypair::generate(), 3600);
        let obs_list = vec![
            obs("fs.read", true),
            obs("fs.write", true),
            obs("net.fetch", false), // undeclared
            obs("fs.read", true),
        ];
        emit_behavior_trace(&svc, "agent-1", "ctr_y", &obs_list, 100);
        let hist = svc.history(&arkavo_trust::TrustHistoryRequest {
            subject_id: "agent-1".to_string(),
            event_types: Some(vec![event_types::BEHAVIOR_TRACE.to_string()]),
            after: None,
            before: None,
            limit: None,
            cursor: None,
        });
        assert_eq!(hist.events.len(), 1);
        let payload: BehaviorTrace =
            serde_json::from_value(hist.events[0].payload.clone()).unwrap();
        assert_eq!(payload.total_tool_calls, 4);
        assert_eq!(payload.undeclared_tool_calls, 1);
        assert!((payload.fidelity_ratio - 0.75).abs() < 1e-9);
        assert_eq!(payload.contract_id, "ctr_y");
    }
}
