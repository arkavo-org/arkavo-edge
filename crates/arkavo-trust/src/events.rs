//! MCP-T v0.2.0 event payload schemas.
//!
//! These structs define the `payload` shape for each event type introduced in
//! v0.2.0 (Sections 6.4-6.6 of the spec). They are not embedded in `TrustEvent`
//! directly — the event log keeps payloads as opaque `serde_json::Value` so the
//! transport schema stays forward-compatible. Use these types when building or
//! parsing payloads on either side of `trust/publish`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ----- behavior.trace ------------------------------------------------------

/// Structured runtime execution record (Section 6.4.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorTrace {
    pub trace_id: String,
    pub contract_id: String,
    pub tool_calls: Vec<ToolCallRecord>,
    pub resources_accessed: Vec<ResourceAccessRecord>,
    pub side_effects: Vec<SideEffectRecord>,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub peak_memory_bytes: Option<u64>,
    pub total_tool_calls: u32,
    pub undeclared_tool_calls: u32,
    pub total_resources: u32,
    pub undeclared_resources: u32,
    /// Ratio of declared actions to total actions, in [0.0, 1.0].
    /// Required for v0.2.0 Level 1 conformance.
    pub fidelity_ratio: f64,
}

/// Single observed tool invocation within a trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCallRecord {
    pub tool_name: String,
    pub timestamp: DateTime<Utc>,
    pub duration_ms: u64,
    /// Whether the call was within the agent's declared scope.
    pub declared: bool,
}

/// Single observed resource access within a trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceAccessRecord {
    pub resource_type: String,
    pub resource_id: String,
    pub access_type: String,
    pub declared: bool,
}

/// Side effect recorded outside tool calls (file write, network egress, etc.).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SideEffectRecord {
    pub effect_type: String,
    pub target: String,
    pub timestamp: DateTime<Utc>,
    pub declared: bool,
}

// ----- behavior.invariant_discovered ---------------------------------------

/// Implicit behavioral pattern detected through observation (Section 6.4.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorInvariantDiscovered {
    pub invariant_id: String,
    pub invariant_type: String,
    pub pattern: String,
    pub confidence: f64,
    pub observation_count: u32,
    pub first_observed: DateTime<Utc>,
    pub last_observed: DateTime<Utc>,
    /// Always false for invariants discovered through observation alone.
    pub declared: bool,
}

// ----- behavior.declaration_delta ------------------------------------------

/// Divergence between declared and observed scope (Section 6.4.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorDeclarationDelta {
    pub declared_scope: ScopeDescriptor,
    pub observed_scope: ScopeDescriptor,
    pub delta_type: String,
    pub severity: String,
    pub undeclared_additions: Vec<String>,
    pub trace_id: String,
}

/// Capability/scope descriptor used in declaration deltas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScopeDescriptor {
    pub tools: Vec<String>,
    pub resources: Vec<String>,
    pub side_effects: Vec<String>,
}

// ----- behavior.resource_access --------------------------------------------

/// Single observed resource access aggregated over a window (Section 6.4.4).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BehaviorResourceAccess {
    pub resource_type: String,
    pub resource_id: String,
    pub access_type: String,
    pub authorized: bool,
    pub frequency: u32,
    pub observation_window_seconds: u32,
    pub trace_id: String,
}

// ----- simulation.executed -------------------------------------------------

/// Pre-execution behavioral simulation completion (Section 6.5.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationExecuted {
    pub simulation_id: String,
    /// SHA-256 hex of the canonical task specification.
    pub task_spec_hash: String,
    pub subject_id: String,
    pub simulator_id: String,
    pub environment: SimulationEnvironment,
    pub execution_paths_explored: u32,
    pub methodology: String,
}

/// Sandbox constraints for a simulation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationEnvironment {
    pub max_memory_bytes: u64,
    pub max_duration_ms: u64,
    pub network_access: bool,
}

// ----- simulation.result ---------------------------------------------------

/// Prediction outcome from a simulation (Section 6.5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub simulation_id: String,
    pub predicted_outcome: String,
    pub confidence: f64,
    pub predicted_duration_ms: u64,
    pub predicted_tool_calls: u32,
    pub predicted_resources: u32,
    pub risk_flags: Vec<RiskFlag>,
    pub comparable_task_count: u32,
}

/// Potential behavioral risk flagged during simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RiskFlag {
    pub risk_type: String,
    pub probability: f64,
    pub evidence: String,
}

// ----- simulation.delta ----------------------------------------------------

/// Measured divergence between simulated and actual execution (Section 6.5.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationDelta {
    pub simulation_id: String,
    pub contract_id: String,
    pub trace_id: String,
    pub predicted_outcome: String,
    pub actual_outcome: String,
    pub outcome_match: bool,
    /// Divergence on [0.0, 1.0] where 0.0 is a perfect prediction.
    pub divergence_score: f64,
    pub divergences: Vec<DivergenceRecord>,
}

/// Single category of divergence between prediction and reality.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DivergenceRecord {
    pub category: String,
    pub predicted: serde_json::Value,
    pub actual: serde_json::Value,
    pub delta: String,
}

// ----- contract.bid_submitted / accepted / rejected ------------------------

/// Bid placed on a contract (Section 6.6.1).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractBidSubmitted {
    pub contract_id: String,
    pub bid_id: String,
    pub bid_amount: String,
    pub currency: String,
    pub estimated_duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub simulation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stake_amount: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stake_currency: Option<String>,
    pub capabilities_declared: Vec<String>,
}

/// Bid accepted by the contract issuer (Section 6.6.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractBidAccepted {
    pub contract_id: String,
    pub bid_id: String,
    pub agent_id: String,
    pub accepted_amount: String,
    pub currency: String,
    pub acceptance_criteria: String,
}

/// Bid declined by the contract issuer (Section 6.6.3).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractBidRejected {
    pub contract_id: String,
    pub bid_id: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts() -> DateTime<Utc> {
        DateTime::parse_from_rfc3339("2026-04-01T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn behavior_trace_roundtrip() {
        let trace = BehaviorTrace {
            trace_id: "trc_1".into(),
            contract_id: "ctr_1".into(),
            tool_calls: vec![ToolCallRecord {
                tool_name: "fs.read".into(),
                timestamp: ts(),
                duration_ms: 12,
                declared: true,
            }],
            resources_accessed: vec![ResourceAccessRecord {
                resource_type: "file".into(),
                resource_id: "/tmp/x".into(),
                access_type: "read".into(),
                declared: true,
            }],
            side_effects: vec![SideEffectRecord {
                effect_type: "file_write".into(),
                target: "/tmp/y".into(),
                timestamp: ts(),
                declared: false,
            }],
            duration_ms: 100,
            peak_memory_bytes: Some(1024),
            total_tool_calls: 1,
            undeclared_tool_calls: 0,
            total_resources: 1,
            undeclared_resources: 0,
            fidelity_ratio: 0.75,
        };

        let json = serde_json::to_string(&trace).unwrap();
        let parsed: BehaviorTrace = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.trace_id, "trc_1");
        assert_eq!(parsed.tool_calls.len(), 1);
        assert!((parsed.fidelity_ratio - 0.75).abs() < 1e-9);
    }

    #[test]
    fn behavior_declaration_delta_roundtrip() {
        let delta = BehaviorDeclarationDelta {
            declared_scope: ScopeDescriptor {
                tools: vec!["fs.read".into()],
                resources: vec!["file:/etc".into()],
                side_effects: vec![],
            },
            observed_scope: ScopeDescriptor {
                tools: vec!["fs.read".into(), "net.get".into()],
                resources: vec!["file:/etc".into(), "https://example.com".into()],
                side_effects: vec!["network_egress".into()],
            },
            delta_type: "scope_exceeded".into(),
            severity: "high".into(),
            undeclared_additions: vec!["net.get".into(), "network_egress".into()],
            trace_id: "trc_1".into(),
        };
        let json = serde_json::to_string(&delta).unwrap();
        let parsed: BehaviorDeclarationDelta = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.severity, "high");
        assert_eq!(parsed.undeclared_additions.len(), 2);
    }

    #[test]
    fn behavior_invariant_discovered_roundtrip() {
        let inv = BehaviorInvariantDiscovered {
            invariant_id: "inv_1".into(),
            invariant_type: "call_sequence".into(),
            pattern: "always reads config before any write".into(),
            confidence: 0.9,
            observation_count: 50,
            first_observed: ts(),
            last_observed: ts(),
            declared: false,
        };
        let json = serde_json::to_string(&inv).unwrap();
        let parsed: BehaviorInvariantDiscovered = serde_json::from_str(&json).unwrap();
        assert!(!parsed.declared);
        assert_eq!(parsed.observation_count, 50);
    }

    #[test]
    fn behavior_resource_access_roundtrip() {
        let ra = BehaviorResourceAccess {
            resource_type: "network".into(),
            resource_id: "https://api.example.com".into(),
            access_type: "read".into(),
            authorized: false,
            frequency: 7,
            observation_window_seconds: 60,
            trace_id: "trc_1".into(),
        };
        let json = serde_json::to_string(&ra).unwrap();
        let parsed: BehaviorResourceAccess = serde_json::from_str(&json).unwrap();
        assert!(!parsed.authorized);
        assert_eq!(parsed.frequency, 7);
    }

    #[test]
    fn simulation_executed_and_result_roundtrip() {
        let exe = SimulationExecuted {
            simulation_id: "sim_1".into(),
            task_spec_hash: "abc123".into(),
            subject_id: "did:key:z6MkAgent".into(),
            simulator_id: "did:key:z6MkSim".into(),
            environment: SimulationEnvironment {
                max_memory_bytes: 1 << 28,
                max_duration_ms: 30_000,
                network_access: false,
            },
            execution_paths_explored: 12,
            methodology: "sandbox_execution".into(),
        };
        let res = SimulationResult {
            simulation_id: "sim_1".into(),
            predicted_outcome: "success".into(),
            confidence: 0.82,
            predicted_duration_ms: 1500,
            predicted_tool_calls: 3,
            predicted_resources: 2,
            risk_flags: vec![RiskFlag {
                risk_type: "undeclared_egress".into(),
                probability: 0.1,
                evidence: "1 of 12 paths".into(),
            }],
            comparable_task_count: 200,
        };

        let exe_json = serde_json::to_string(&exe).unwrap();
        let res_json = serde_json::to_string(&res).unwrap();
        let exe2: SimulationExecuted = serde_json::from_str(&exe_json).unwrap();
        let res2: SimulationResult = serde_json::from_str(&res_json).unwrap();
        assert_eq!(exe2.methodology, "sandbox_execution");
        assert_eq!(res2.risk_flags.len(), 1);
    }

    #[test]
    fn simulation_delta_roundtrip() {
        let delta = SimulationDelta {
            simulation_id: "sim_1".into(),
            contract_id: "ctr_1".into(),
            trace_id: "trc_1".into(),
            predicted_outcome: "success".into(),
            actual_outcome: "partial_success".into(),
            outcome_match: false,
            divergence_score: 0.3,
            divergences: vec![DivergenceRecord {
                category: "tool_calls".into(),
                predicted: serde_json::json!(3),
                actual: serde_json::json!(5),
                delta: "+2".into(),
            }],
        };
        let json = serde_json::to_string(&delta).unwrap();
        let parsed: SimulationDelta = serde_json::from_str(&json).unwrap();
        assert!(!parsed.outcome_match);
        assert_eq!(parsed.divergences[0].category, "tool_calls");
    }

    #[test]
    fn contract_bid_lifecycle_roundtrip() {
        let sub = ContractBidSubmitted {
            contract_id: "ctr_1".into(),
            bid_id: "bid_1".into(),
            bid_amount: "100".into(),
            currency: "USDC".into(),
            estimated_duration_ms: 60_000,
            simulation_id: Some("sim_1".into()),
            stake_amount: Some("10".into()),
            stake_currency: Some("USDC".into()),
            capabilities_declared: vec!["fs.read".into()],
        };
        let acc = ContractBidAccepted {
            contract_id: "ctr_1".into(),
            bid_id: "bid_1".into(),
            agent_id: "did:key:z6MkAgent".into(),
            accepted_amount: "100".into(),
            currency: "USDC".into(),
            acceptance_criteria: "best_simulation".into(),
        };
        let rej = ContractBidRejected {
            contract_id: "ctr_1".into(),
            bid_id: "bid_2".into(),
            reason: "trust_threshold_not_met".into(),
            details: Some("composite < 700".into()),
        };

        for json in [
            serde_json::to_string(&sub).unwrap(),
            serde_json::to_string(&acc).unwrap(),
            serde_json::to_string(&rej).unwrap(),
        ] {
            // Each must parse back as serde_json::Value at minimum.
            let v: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert!(v.is_object());
        }
        let sub2: ContractBidSubmitted =
            serde_json::from_str(&serde_json::to_string(&sub).unwrap()).unwrap();
        assert_eq!(sub2.capabilities_declared, vec!["fs.read".to_string()]);
    }
}
