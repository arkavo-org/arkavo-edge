//! Fault injection scenarios for adversarial testing
//!
//! These scenarios test edge cases from specifications:
//! - GOSSIP-008: Network partition handling
//! - REG-005: Clock skew during registration
//! - TDF-002: KAS unavailability

use crate::adversary::{AdversaryConfig, AdversaryScenario, FaultType, ScenarioOutcome};
use crate::AgentProfile;
use std::collections::HashMap;

/// Creates a network partition scenario (GOSSIP-008 edge case)
///
/// Tests system behavior when network partitions occur between agents.
/// Expected: System detects partition, uses fallback agents, eventually converges.
pub fn network_partition_scenario() -> AdversaryScenario {
    AdversaryScenario {
        id: "GOSSIP-008".to_string(),
        name: "Network partition recovery".to_string(),
        description: "Simulates network partition between agent groups. Tests gossip protocol \
                     resilience and eventual consistency when connectivity is restored."
            .to_string(),
        adversaries: vec![
            // Primary agents in partition A (isolated)
            AdversaryConfig {
                profile: AgentProfile {
                    id: "partition-a-agent-1".to_string(),
                    capabilities: vec!["gossip".to_string(), "compute".to_string()],
                    success_rate: 1.0,
                    latency_ms: 100,
                    responses: HashMap::new(),
                },
                fault_rate: 0.8, // High fault rate simulates partition
                fault_type: FaultType::Timeout,
            },
            AdversaryConfig {
                profile: AgentProfile {
                    id: "partition-a-agent-2".to_string(),
                    capabilities: vec!["gossip".to_string(), "storage".to_string()],
                    success_rate: 1.0,
                    latency_ms: 100,
                    responses: HashMap::new(),
                },
                fault_rate: 0.8,
                fault_type: FaultType::Timeout,
            },
        ],
        fallback_agents: vec![
            // Fallback agents in partition B (available)
            AgentProfile {
                id: "partition-b-fallback".to_string(),
                capabilities: vec!["compute".to_string(), "storage".to_string()],
                success_rate: 1.0,
                latency_ms: 50,
                responses: HashMap::from([(
                    "Subtask 1: Network partition recovery test".to_string(),
                    serde_json::json!({
                        "status": "completed",
                        "result": "Task completed despite network partition",
                        "partition_detected": true,
                        "fallback_used": true
                    }),
                )]),
            },
        ],
        expected: ScenarioOutcome {
            task_succeeds: true,
            expected_error: None,
            min_retries: 2,
            agent_penalized: true,
            critic_vetoed: false,
        },
    }
}

/// Creates a clock skew scenario (REG-005 edge case)
///
/// Tests registration token validation when system clocks are skewed.
/// Expected: System detects skew, uses token freshness checks, may reject expired tokens.
pub fn clock_skew_scenario() -> AdversaryScenario {
    AdversaryScenario {
        id: "REG-005".to_string(),
        name: "Clock skew during registration".to_string(),
        description: "Simulates clock skew between agent and registration server. \
                     Tests token expiration handling and time-based validation robustness."
            .to_string(),
        adversaries: vec![
            // Agent with "future" clock (produces tokens that appear expired)
            AdversaryConfig {
                profile: AgentProfile {
                    id: "future-clock-agent".to_string(),
                    capabilities: vec!["registration".to_string()],
                    success_rate: 1.0,
                    latency_ms: 10,
                    responses: HashMap::from([(
                        "Subtask 1: Clock skew registration test".to_string(),
                        serde_json::json!({
                            "status": "completed",
                            "result": "Registration succeeded with skew correction",
                            "clock_skew_detected": true,
                            "skew_ms": 3_600_000, // 1 hour skew
                            "token_fresh": false,
                            "retry_with_sync": true
                        }),
                    )]),
                },
                fault_rate: 0.5,
                fault_type: FaultType::PolicyViolation, // Simulates expired token response
            },
        ],
        fallback_agents: vec![
            // Fallback with correct clock
            AgentProfile {
                id: "synced-agent".to_string(),
                capabilities: vec!["registration".to_string()],
                success_rate: 1.0,
                latency_ms: 10,
                responses: HashMap::from([(
                    "Subtask 1: Clock skew registration test".to_string(),
                    serde_json::json!({
                        "status": "completed",
                        "result": "Registration successful after clock sync",
                        "ntp_synced": true,
                        "token_fresh": true
                    }),
                )]),
            },
        ],
        expected: ScenarioOutcome {
            task_succeeds: true,
            expected_error: None,
            min_retries: 1,
            agent_penalized: true,
            critic_vetoed: true, // Critic should catch the expired token
        },
    }
}

/// Creates a KAS unavailability scenario (TDF-002 edge case)
///
/// Tests TDF encryption/decryption when Key Access Server is unavailable.
/// Expected: System queues operations, uses cached keys, or fails gracefully.
pub fn kas_unavailability_scenario() -> AdversaryScenario {
    AdversaryScenario {
        id: "TDF-002".to_string(),
        name: "KAS unavailability during TDF operations".to_string(),
        description: "Simulates Key Access Server being unavailable during TDF \
                     encryption/decryption. Tests key caching and graceful degradation."
            .to_string(),
        adversaries: vec![
            // Agent attempting TDF operations with KAS down
            AdversaryConfig {
                profile: AgentProfile {
                    id: "tdf-agent".to_string(),
                    capabilities: vec!["tdf-encrypt".to_string(), "tdf-decrypt".to_string()],
                    success_rate: 1.0,
                    latency_ms: 500,
                    responses: HashMap::from([(
                        "Subtask 1: KAS unavailability test".to_string(),
                        serde_json::json!({
                            "status": "completed",
                            "result": "TDF operation completed with cached key",
                            "kas_available": false,
                            "key_cached": true,
                            "cache_ttl_remaining": 300
                        }),
                    )]),
                },
                fault_rate: 0.7,
                fault_type: FaultType::Timeout, // KAS timeout
            },
        ],
        fallback_agents: vec![
            // Fallback KAS endpoint
            AgentProfile {
                id: "backup-kas-agent".to_string(),
                capabilities: vec!["tdf-encrypt".to_string(), "tdf-decrypt".to_string()],
                success_rate: 1.0,
                latency_ms: 100,
                responses: HashMap::from([(
                    "Subtask 1: KAS unavailability test".to_string(),
                    serde_json::json!({
                        "status": "completed",
                        "result": "TDF operation via backup KAS",
                        "kas_available": true,
                        "backup_used": true,
                        "primary_kas_status": "unreachable"
                    }),
                )]),
            },
        ],
        expected: ScenarioOutcome {
            task_succeeds: true,
            expected_error: None,
            min_retries: 1,
            agent_penalized: false, // Network issues shouldn't penalize agent
            critic_vetoed: false,
        },
    }
}

/// Creates a cascading failure scenario
///
/// Tests system behavior when multiple dependent services fail in sequence.
/// Expected: Circuit breaker activates, fallback kicks in, partial results returned.
pub fn cascading_failure_scenario() -> AdversaryScenario {
    AdversaryScenario {
        id: "ADV-CASCADE".to_string(),
        name: "Cascading service failures".to_string(),
        description: "Simulates cascading failures across dependent services. \
                     Tests circuit breaker and bulkhead patterns."
            .to_string(),
        adversaries: vec![
            // Primary service (fails)
            AdversaryConfig {
                profile: AgentProfile {
                    id: "primary-service".to_string(),
                    capabilities: vec!["compute".to_string()],
                    success_rate: 1.0,
                    latency_ms: 50,
                    responses: HashMap::new(),
                },
                fault_rate: 1.0, // Always fails
                fault_type: FaultType::Timeout,
            },
            // Dependent service (affected by primary failure)
            AdversaryConfig {
                profile: AgentProfile {
                    id: "dependent-service".to_string(),
                    capabilities: vec!["storage".to_string()],
                    success_rate: 1.0,
                    latency_ms: 50,
                    responses: HashMap::new(),
                },
                fault_rate: 0.5,
                fault_type: FaultType::SchemaCorruption,
            },
        ],
        fallback_agents: vec![
            // Circuit breaker isolated fallback
            AgentProfile {
                id: "circuit-breaker-fallback".to_string(),
                capabilities: vec!["compute".to_string(), "storage".to_string()],
                success_rate: 1.0,
                latency_ms: 30,
                responses: HashMap::from([(
                    "Subtask 1: Cascading failure test".to_string(),
                    serde_json::json!({
                        "status": "completed",
                        "result": "Circuit breaker activated, degraded mode",
                        "circuit_breaker_open": true,
                        "services_affected": vec!["primary-service", "dependent-service"],
                        "degraded_mode": true
                    }),
                )]),
            },
        ],
        expected: ScenarioOutcome {
            task_succeeds: true,
            expected_error: None,
            min_retries: 2,
            agent_penalized: true,
            critic_vetoed: false,
        },
    }
}

/// Creates a Byzantine fault scenario
///
/// Tests system behavior when agents produce conflicting/malicious outputs.
/// Expected: Quorum-based consensus detects anomaly, faulty agents isolated.
pub fn byzantine_fault_scenario() -> AdversaryScenario {
    AdversaryScenario {
        id: "ADV-BYZANTINE".to_string(),
        name: "Byzantine agent behavior".to_string(),
        description: "Simulates Byzantine faults where agents produce conflicting \
                     or malicious outputs. Tests consensus and anomaly detection."
            .to_string(),
        adversaries: vec![
            // Malicious agent (produces wrong results)
            AdversaryConfig {
                profile: AgentProfile {
                    id: "byzantine-agent-1".to_string(),
                    capabilities: vec!["compute".to_string()],
                    success_rate: 1.0,
                    latency_ms: 20,
                    responses: HashMap::from([(
                        "Subtask 1: Byzantine fault test".to_string(),
                        serde_json::json!({
                            "status": "completed",
                            "result": "wrong_answer_123", // Intentionally wrong
                            "hash": "fake_hash_xyz"
                        }),
                    )]),
                },
                fault_rate: 0.9,
                fault_type: FaultType::LazyCompliance,
            },
            // Another malicious agent (conflicting output)
            AdversaryConfig {
                profile: AgentProfile {
                    id: "byzantine-agent-2".to_string(),
                    capabilities: vec!["compute".to_string()],
                    success_rate: 1.0,
                    latency_ms: 20,
                    responses: HashMap::from([(
                        "Subtask 1: Byzantine fault test".to_string(),
                        serde_json::json!({
                            "status": "completed",
                            "result": "different_wrong_answer_456", // Conflicts with agent-1
                            "hash": "another_fake_hash"
                        }),
                    )]),
                },
                fault_rate: 0.9,
                fault_type: FaultType::HallucinatedTool,
            },
        ],
        fallback_agents: vec![
            // Honest agents for quorum
            AgentProfile {
                id: "honest-agent-1".to_string(),
                capabilities: vec!["compute".to_string()],
                success_rate: 1.0,
                latency_ms: 30,
                responses: HashMap::from([(
                    "Subtask 1: Byzantine fault test".to_string(),
                    serde_json::json!({
                        "status": "completed",
                        "result": "correct_answer_42",
                        "hash": "valid_hash_abc123",
                        "quorum_verified": true
                    }),
                )]),
            },
            AgentProfile {
                id: "honest-agent-2".to_string(),
                capabilities: vec!["compute".to_string()],
                success_rate: 1.0,
                latency_ms: 35,
                responses: HashMap::from([(
                    "Subtask 1: Byzantine fault test".to_string(),
                    serde_json::json!({
                        "status": "completed",
                        "result": "correct_answer_42", // Matches honest-agent-1
                        "hash": "valid_hash_abc123",
                        "quorum_verified": true
                    }),
                )]),
            },
        ],
        expected: ScenarioOutcome {
            task_succeeds: true,
            expected_error: None,
            min_retries: 3,
            agent_penalized: true,
            critic_vetoed: true, // Critic should catch the bad outputs
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_partition_scenario() {
        let scenario = network_partition_scenario();
        assert_eq!(scenario.id, "GOSSIP-008");
        assert!(!scenario.adversaries.is_empty());
        assert!(!scenario.fallback_agents.is_empty());
        assert!(scenario.expected.task_succeeds);
    }

    #[test]
    fn test_clock_skew_scenario() {
        let scenario = clock_skew_scenario();
        assert_eq!(scenario.id, "REG-005");
        assert!(scenario.expected.critic_vetoed); // Should detect expired tokens
    }

    #[test]
    fn test_kas_unavailability_scenario() {
        let scenario = kas_unavailability_scenario();
        assert_eq!(scenario.id, "TDF-002");
        assert!(!scenario.expected.agent_penalized); // Network issues shouldn't penalize
    }

    #[test]
    fn test_cascading_failure_scenario() {
        let scenario = cascading_failure_scenario();
        assert_eq!(scenario.id, "ADV-CASCADE");
        assert!(scenario.expected.min_retries >= 2);
    }

    #[test]
    fn test_byzantine_fault_scenario() {
        let scenario = byzantine_fault_scenario();
        assert_eq!(scenario.id, "ADV-BYZANTINE");
        assert!(scenario.expected.critic_vetoed);
        assert!(scenario.adversaries.len() >= 2); // Multiple faulty agents
    }
}
