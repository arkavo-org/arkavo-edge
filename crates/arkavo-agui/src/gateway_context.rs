use crate::types::*;
use arkavo_router::learning::LearningModule;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

/// Handle a RequestContextTopology event by aggregating context mechanism data
/// from the local LearningModule and the pushed telemetry cache.
pub(crate) async fn handle_request_context_topology(
    learning_module: &Arc<RwLock<LearningModule>>,
    agents_registry: &Arc<RwLock<Vec<serde_json::Value>>>,
    context_topology_cache: &Arc<RwLock<HashMap<String, serde_json::Value>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Collect Thompson Sampling priors from local LearningModule
    let lm = learning_module.read().await;
    let all_stats = lm.get_all_stats().await;
    let mut agents: Vec<AgentContextInfo> = Vec::with_capacity(all_stats.len());
    for s in &all_stats {
        let cat_stats = lm.get_category_stats(&s.agent_id).await;
        let category_stats: Vec<CategoryStat> = cat_stats
            .into_iter()
            .map(|(cat, alpha, beta_param, ev, obs)| CategoryStat {
                category: cat,
                alpha,
                beta_param,
                expected_value: ev,
                observations: obs,
            })
            .collect();
        agents.push(AgentContextInfo {
            agent_id: s.agent_id.clone(),
            model: None,
            context_utilization_pct: None,
            alpha: s.alpha,
            beta_param: s.beta_param,
            expected_value: s.expected_value,
            total_observations: s.total_observations,
            category_stats,
        });
    }
    drop(lm);

    // Enrich with model names from agent registry
    {
        let agents_list = agents_registry.read().await;
        for agent_info in agents.iter_mut() {
            agent_info.model = agents_list
                .iter()
                .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(&agent_info.agent_id))
                .and_then(|a| a.get("model").and_then(|v| v.as_str()))
                .map(|s| s.to_string());
        }
    }

    // Aggregate context topology from pushed telemetry cache
    let mut rlm = RlmSnapshot {
        manifest_count: 0,
        total_chunks: 0,
        total_tokens: 0,
        activation_threshold: 0.7,
    };
    let mut context_strategies: Vec<ContextStrategySnapshot> = Vec::new();
    let mut tool_memory = ToolMemorySnapshot {
        entry_count: 0,
        max_entries: 10,
        error_count: 0,
        duplicate_count: 0,
        recent_action_types: Vec::new(),
        consecutive_same_type: 0,
        has_observe_data: false,
    };
    let mut decision_traces: Vec<DecisionTraceSnapshot> = Vec::new();
    let mut anti_patterns: Vec<AntiPatternSnapshot> = Vec::new();
    let mut memory_lifecycle = MemoryLifecycleSnapshot {
        promoted: 0,
        distilled: 0,
        expired: 0,
        demoted: 0,
        transient_ttl_days: 7,
        promotion_threshold: 3,
        canonical_threshold: 10,
    };
    let mut gossip = GossipSnapshot {
        events_received: 0,
        episodes_synthesized: 0,
        lessons_stored: 0,
        gossip_peers: 0,
        last_event_secs_ago: None,
    };

    // Read from the cache of pushed context_topology telemetry
    // Only include agents still in the registry (evicts disconnected agents)
    let known_agent_ids: std::collections::HashSet<String> =
        agents.iter().map(|a| a.agent_id.clone()).collect();
    let cache = context_topology_cache.read().await;
    for (cached_agent_id, resp) in cache.iter() {
        if !known_agent_ids.contains(cached_agent_id) {
            continue;
        }
        parse_tool_memory(resp, &mut tool_memory);
        parse_decision_traces(resp, &mut decision_traces);
        parse_anti_patterns(resp, &mut anti_patterns);
        parse_gossip_snapshot(resp, &mut gossip);
        parse_rlm_snapshot(resp, &mut rlm);
        parse_context_strategies(resp, &mut context_strategies);
        parse_memory_lifecycle(resp, &mut memory_lifecycle);

        // Enrich agent context utilization from per-agent toolMemory snapshot
        if let Some(pct) = resp
            .get("toolMemory")
            .and_then(|tm| tm.get("contextUtilizationPct"))
            .and_then(|v| v.as_f64())
            && pct > 0.0
        {
            for agent in agents.iter_mut() {
                if agent.agent_id == *cached_agent_id {
                    agent.context_utilization_pct = Some(pct);
                }
            }
        }
    }
    drop(cache);

    decision_traces.truncate(10);

    tx.send(AgUiEvent::ContextTopologyUpdate {
        rlm,
        context_strategies,
        tool_memory,
        decision_traces,
        anti_patterns,
        memory_lifecycle,
        gossip,
        agents,
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
    .await?;

    Ok(())
}

fn parse_rlm_snapshot(resp: &serde_json::Value, rlm: &mut RlmSnapshot) {
    if let Some(r) = resp.get("rlm") {
        rlm.manifest_count += r.get("manifestCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        rlm.total_chunks += r.get("totalChunks").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        rlm.total_tokens += r.get("totalTokens").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if let Some(t) = r.get("activationThreshold").and_then(|v| v.as_f64()) {
            rlm.activation_threshold = t as f32;
        }
    }
}

fn parse_context_strategies(
    resp: &serde_json::Value,
    strategies: &mut Vec<ContextStrategySnapshot>,
) {
    if let Some(arr) = resp.get("contextStrategies").and_then(|v| v.as_array()) {
        for s in arr {
            strategies.push(ContextStrategySnapshot {
                strategy: s
                    .get("strategy")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                completion_rate: s
                    .get("completionRate")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                loop_avoidance_rate: s
                    .get("loopAvoidanceRate")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                avg_context_bytes: s
                    .get("avgContextBytes")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize,
                composite_score: s
                    .get("compositeScore")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                burst_count: s.get("burstCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
            });
        }
    }
}

fn parse_tool_memory(resp: &serde_json::Value, tm: &mut ToolMemorySnapshot) {
    if let Some(t) = resp.get("toolMemory") {
        tm.entry_count += t.get("entryCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        tm.max_entries += t.get("maxEntries").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        tm.error_count += t.get("errorCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        tm.duplicate_count += t
            .get("duplicateCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        tm.consecutive_same_type = tm.consecutive_same_type.max(
            t.get("consecutiveSameType")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize,
        );
        if t.get("hasObserveData")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            tm.has_observe_data = true;
        }
        if let Some(arr) = t.get("recentActionTypes").and_then(|v| v.as_array()) {
            for v in arr {
                if let Some(s) = v.as_str()
                    && !tm.recent_action_types.contains(&s.to_string())
                {
                    tm.recent_action_types.push(s.to_string());
                }
            }
        }
    }
}

const MAX_DECISION_TRACES: usize = 20;

fn parse_decision_traces(resp: &serde_json::Value, traces: &mut Vec<DecisionTraceSnapshot>) {
    if let Some(arr) = resp.get("decisionTraces").and_then(|v| v.as_array()) {
        for t in arr {
            if traces.len() >= MAX_DECISION_TRACES {
                return;
            }
            traces.push(DecisionTraceSnapshot {
                trace_id: t
                    .get("traceId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                agent_id: t
                    .get("agentId")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                task_category: t
                    .get("taskCategory")
                    .and_then(|v| v.as_str())
                    .unwrap_or("general")
                    .to_string(),
                selected_model: t
                    .get("selectedModel")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                selection_reason: t
                    .get("selectionReason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string(),
                budget_usage_pct: t
                    .get("budgetUsagePct")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                feasible_count: t.get("feasibleCount").and_then(|v| v.as_u64()).unwrap_or(0)
                    as usize,
                timestamp: t
                    .get("timestamp")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            });
        }
    }
}

fn parse_anti_patterns(resp: &serde_json::Value, patterns: &mut Vec<AntiPatternSnapshot>) {
    if let Some(arr) = resp.get("antiPatterns").and_then(|v| v.as_array()) {
        for p in arr {
            patterns.push(AntiPatternSnapshot {
                model: p
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                category: p
                    .get("category")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                failure_signature: p
                    .get("failureSignature")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                failure_count: p.get("failureCount").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
                decayed_weight: p
                    .get("decayedWeight")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(0.0),
                last_seen: p
                    .get("lastSeen")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
            });
        }
    }
}

fn parse_memory_lifecycle(resp: &serde_json::Value, lc: &mut MemoryLifecycleSnapshot) {
    if let Some(m) = resp.get("memoryLifecycle") {
        lc.promoted += m.get("promoted").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        lc.distilled += m.get("distilled").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        lc.expired += m.get("expired").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        lc.demoted += m.get("demoted").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if let Some(t) = m.get("transientTtlDays").and_then(|v| v.as_u64()) {
            lc.transient_ttl_days = t as u32;
        }
        if let Some(t) = m.get("promotionThreshold").and_then(|v| v.as_u64()) {
            lc.promotion_threshold = t as u32;
        }
        if let Some(t) = m.get("canonicalThreshold").and_then(|v| v.as_u64()) {
            lc.canonical_threshold = t as u32;
        }
    }
}

fn parse_gossip_snapshot(resp: &serde_json::Value, g: &mut GossipSnapshot) {
    if let Some(gs) = resp.get("gossip") {
        g.events_received += gs
            .get("eventsReceived")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        g.episodes_synthesized += gs
            .get("episodesSynthesized")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        g.lessons_stored += gs
            .get("lessonsStored")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        g.gossip_peers += gs.get("gossipPeers").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        if let Some(t) = gs.get("lastEventSecsAgo").and_then(|v| v.as_u64()) {
            g.last_event_secs_ago = Some(match g.last_event_secs_ago {
                Some(existing) => existing.min(t),
                None => t,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn default_tool_memory() -> ToolMemorySnapshot {
        ToolMemorySnapshot {
            entry_count: 0,
            max_entries: 10,
            error_count: 0,
            duplicate_count: 0,
            recent_action_types: vec![],
            consecutive_same_type: 0,
            has_observe_data: false,
        }
    }

    fn default_gossip() -> GossipSnapshot {
        GossipSnapshot {
            events_received: 0,
            episodes_synthesized: 0,
            lessons_stored: 0,
            gossip_peers: 0,
            last_event_secs_ago: None,
        }
    }

    fn default_rlm() -> RlmSnapshot {
        RlmSnapshot {
            manifest_count: 0,
            total_chunks: 0,
            total_tokens: 0,
            activation_threshold: 0.7,
        }
    }

    fn default_lifecycle() -> MemoryLifecycleSnapshot {
        MemoryLifecycleSnapshot {
            promoted: 0,
            distilled: 0,
            expired: 0,
            demoted: 0,
            transient_ttl_days: 7,
            promotion_threshold: 3,
            canonical_threshold: 10,
        }
    }

    // --- AGUI-009: All snapshot structs serialize/deserialize correctly ---

    #[spec("AGUI-009")]
    #[test]
    fn all_snapshot_structs_roundtrip() {
        // RlmSnapshot
        let rlm = RlmSnapshot {
            manifest_count: 3,
            total_chunks: 15,
            total_tokens: 4096,
            activation_threshold: 0.7,
        };
        let parsed: RlmSnapshot =
            serde_json::from_str(&serde_json::to_string(&rlm).unwrap()).unwrap();
        assert_eq!(parsed.manifest_count, 3);
        assert_eq!(parsed.total_tokens, 4096);

        // ToolMemorySnapshot
        let tm = ToolMemorySnapshot {
            entry_count: 7,
            max_entries: 10,
            error_count: 2,
            duplicate_count: 1,
            recent_action_types: vec!["code_search".into(), "file_read".into()],
            consecutive_same_type: 3,
            has_observe_data: true,
        };
        let parsed: ToolMemorySnapshot =
            serde_json::from_str(&serde_json::to_string(&tm).unwrap()).unwrap();
        assert_eq!(parsed.entry_count, 7);
        assert_eq!(parsed.recent_action_types.len(), 2);
        assert!(parsed.has_observe_data);

        // GossipSnapshot
        let gs = GossipSnapshot {
            events_received: 100,
            episodes_synthesized: 5,
            lessons_stored: 3,
            gossip_peers: 4,
            last_event_secs_ago: Some(12),
        };
        let parsed: GossipSnapshot =
            serde_json::from_str(&serde_json::to_string(&gs).unwrap()).unwrap();
        assert_eq!(parsed.gossip_peers, 4);
        assert_eq!(parsed.last_event_secs_ago, Some(12));

        // MemoryLifecycleSnapshot
        let lc = MemoryLifecycleSnapshot {
            promoted: 8,
            distilled: 3,
            expired: 2,
            demoted: 1,
            transient_ttl_days: 7,
            promotion_threshold: 3,
            canonical_threshold: 10,
        };
        let parsed: MemoryLifecycleSnapshot =
            serde_json::from_str(&serde_json::to_string(&lc).unwrap()).unwrap();
        assert_eq!(parsed.promoted, 8);
        assert_eq!(parsed.canonical_threshold, 10);

        // DecisionTraceSnapshot with agentId
        let dt = DecisionTraceSnapshot {
            trace_id: "abc-123".into(),
            agent_id: "code-analyzer".into(),
            task_category: "code_generation".into(),
            selected_model: "ministral-3b".into(),
            selection_reason: "Thompson(0.85)".into(),
            budget_usage_pct: 45.2,
            feasible_count: 3,
            timestamp: "2026-03-23T10:00:00Z".into(),
        };
        let json = serde_json::to_string(&dt).unwrap();
        assert!(json.contains("agentId"));
        assert!(json.contains("code-analyzer"));
        let parsed: DecisionTraceSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.agent_id, "code-analyzer");
        assert_eq!(parsed.feasible_count, 3);

        // AgentContextInfo with context_utilization_pct
        let agent = AgentContextInfo {
            agent_id: "test-agent".into(),
            model: Some("qwen-8b".into()),
            context_utilization_pct: Some(67.5),
            alpha: 8.5,
            beta_param: 2.1,
            expected_value: 0.80,
            total_observations: 42,
            category_stats: vec![CategoryStat {
                category: "code".into(),
                alpha: 6.0,
                beta_param: 1.5,
                expected_value: 0.8,
                observations: 20,
            }],
        };
        let json = serde_json::to_string(&agent).unwrap();
        assert!(json.contains("contextUtilizationPct"));
        assert!(json.contains("67.5"));
        let parsed: AgentContextInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.context_utilization_pct, Some(67.5));
        assert_eq!(parsed.category_stats.len(), 1);

        // AgentContextInfo with None utilization omits field
        let agent_no_util = AgentContextInfo {
            agent_id: "idle-agent".into(),
            model: None,
            context_utilization_pct: None,
            alpha: 2.0,
            beta_param: 1.0,
            expected_value: 0.67,
            total_observations: 0,
            category_stats: vec![],
        };
        let json = serde_json::to_string(&agent_no_util).unwrap();
        assert!(!json.contains("contextUtilizationPct"));
    }

    #[spec("AGUI-009")]
    #[test]
    fn anti_pattern_snapshot_edge_cases() {
        // Zero weight (fully decayed)
        let ap = AntiPatternSnapshot {
            model: "qwen-8b".into(),
            category: "code".into(),
            failure_signature: "hallucinated_tool:read_file".into(),
            failure_count: 5,
            decayed_weight: 0.0,
            last_seen: "2026-03-23T10:00:00Z".into(),
        };
        let parsed: AntiPatternSnapshot =
            serde_json::from_str(&serde_json::to_string(&ap).unwrap()).unwrap();
        assert_eq!(parsed.decayed_weight, 0.0);
        assert_eq!(parsed.failure_count, 5);

        // Max weight (just recorded)
        let ap_fresh = AntiPatternSnapshot {
            decayed_weight: 1.0,
            failure_count: 1,
            ..ap
        };
        let parsed: AntiPatternSnapshot =
            serde_json::from_str(&serde_json::to_string(&ap_fresh).unwrap()).unwrap();
        assert_eq!(parsed.decayed_weight, 1.0);
    }

    #[spec("AGUI-009")]
    #[test]
    fn context_topology_update_event_roundtrip() {
        let event = AgUiEvent::ContextTopologyUpdate {
            rlm: default_rlm(),
            context_strategies: vec![ContextStrategySnapshot {
                strategy: "Full".into(),
                completion_rate: 0.95,
                loop_avoidance_rate: 0.8,
                avg_context_bytes: 8200,
                composite_score: 0.72,
                burst_count: 15,
            }],
            tool_memory: default_tool_memory(),
            decision_traces: vec![],
            anti_patterns: vec![],
            memory_lifecycle: default_lifecycle(),
            gossip: default_gossip(),
            agents: vec![],
            timestamp: "2026-03-24T12:00:00Z".into(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains("contextTopologyUpdate"));
        assert!(json.contains("contextStrategies"));
        assert!(json.contains("0.72"));
    }

    // --- AGUI-010: All 7 parsers handle valid data and extract every field ---

    #[spec("AGUI-010")]
    #[test]
    fn parse_tool_memory_all_fields() {
        let resp = serde_json::json!({
            "toolMemory": {
                "entryCount": 7, "maxEntries": 15, "errorCount": 2,
                "duplicateCount": 1, "recentActionTypes": ["search", "read", "build"],
                "consecutiveSameType": 3, "hasObserveData": true
            }
        });
        let mut tm = default_tool_memory();
        parse_tool_memory(&resp, &mut tm);
        assert_eq!(tm.entry_count, 7);
        assert_eq!(tm.max_entries, 25); // default 10 + 15 (accumulates across agents)
        assert_eq!(tm.error_count, 2);
        assert_eq!(tm.duplicate_count, 1);
        assert_eq!(tm.recent_action_types, vec!["search", "read", "build"]);
        assert_eq!(tm.consecutive_same_type, 3);
        assert!(tm.has_observe_data);
    }

    #[spec("AGUI-010")]
    #[test]
    fn parse_decision_traces_with_agent_id() {
        let resp = serde_json::json!({
            "decisionTraces": [
                {
                    "traceId": "t1", "agentId": "agent-a", "taskCategory": "code",
                    "selectedModel": "ministral-3b", "selectionReason": "Thompson(0.95)",
                    "budgetUsagePct": 45.2, "feasibleCount": 3, "timestamp": "2026-03-24T10:00:00Z"
                },
                {
                    "traceId": "t2", "taskCategory": "security",
                    "selectedModel": "qwen-8b", "selectionReason": "SingleFeasible",
                    "budgetUsagePct": 0.0, "feasibleCount": 1, "timestamp": "2026-03-24T10:01:00Z"
                }
            ]
        });
        let mut traces = Vec::new();
        parse_decision_traces(&resp, &mut traces);
        assert_eq!(traces.len(), 2);
        assert_eq!(traces[0].agent_id, "agent-a");
        assert_eq!(traces[0].budget_usage_pct, 45.2);
        // Missing agentId defaults to empty string
        assert_eq!(traces[1].agent_id, "");
        assert_eq!(traces[1].selection_reason, "SingleFeasible");
        assert_eq!(traces[1].feasible_count, 1);
    }

    #[spec("AGUI-010")]
    #[test]
    fn parse_gossip_accumulates_across_agents() {
        let agent1 = serde_json::json!({
            "gossip": { "eventsReceived": 10, "episodesSynthesized": 2,
                        "lessonsStored": 3, "gossipPeers": 4, "lastEventSecsAgo": 5 }
        });
        let agent2 = serde_json::json!({
            "gossip": { "eventsReceived": 20, "episodesSynthesized": 1,
                        "lessonsStored": 2, "gossipPeers": 4, "lastEventSecsAgo": 12 }
        });
        let mut gossip = default_gossip();
        parse_gossip_snapshot(&agent1, &mut gossip);
        parse_gossip_snapshot(&agent2, &mut gossip);
        // Values accumulate when aggregating multiple agents
        assert_eq!(gossip.events_received, 30);
        assert_eq!(gossip.episodes_synthesized, 3);
        assert_eq!(gossip.lessons_stored, 5);
        assert_eq!(gossip.gossip_peers, 8);
        // Last event takes the minimum (most recent across agents)
        assert_eq!(gossip.last_event_secs_ago, Some(5));
    }

    #[spec("AGUI-010")]
    #[test]
    fn parse_anti_patterns_multiple_entries() {
        let resp = serde_json::json!({
            "antiPatterns": [
                { "model": "a", "category": "code", "failureSignature": "timeout:build",
                  "failureCount": 3, "decayedWeight": 0.5, "lastSeen": "2026-03-24T10:00:00Z" },
                { "model": "b", "category": "security", "failureSignature": "hallucinated_tool:scan",
                  "failureCount": 1, "decayedWeight": 0.99, "lastSeen": "2026-03-24T10:01:00Z" }
            ]
        });
        let mut patterns = Vec::new();
        parse_anti_patterns(&resp, &mut patterns);
        assert_eq!(patterns.len(), 2);
        assert_eq!(patterns[0].model, "a");
        assert_eq!(patterns[1].failure_signature, "hallucinated_tool:scan");
        assert!((patterns[1].decayed_weight - 0.99).abs() < f64::EPSILON);
    }

    #[spec("AGUI-010")]
    #[test]
    fn parse_rlm_accumulates() {
        let agent1 = serde_json::json!({
            "rlm": { "manifestCount": 2, "totalChunks": 8, "totalTokens": 2048 }
        });
        let agent2 = serde_json::json!({
            "rlm": { "manifestCount": 1, "totalChunks": 4, "totalTokens": 1024,
                     "activationThreshold": 0.6 }
        });
        let mut rlm = default_rlm();
        parse_rlm_snapshot(&agent1, &mut rlm);
        parse_rlm_snapshot(&agent2, &mut rlm);
        assert_eq!(rlm.manifest_count, 3);
        assert_eq!(rlm.total_chunks, 12);
        assert_eq!(rlm.total_tokens, 3072);
        assert!((rlm.activation_threshold - 0.6).abs() < f32::EPSILON);
    }

    #[spec("AGUI-010")]
    #[test]
    fn parse_memory_lifecycle_all_fields() {
        let resp = serde_json::json!({
            "memoryLifecycle": {
                "promoted": 5, "distilled": 2, "expired": 3, "demoted": 1,
                "transientTtlDays": 14, "promotionThreshold": 5, "canonicalThreshold": 20
            }
        });
        let mut lc = default_lifecycle();
        parse_memory_lifecycle(&resp, &mut lc);
        assert_eq!(lc.promoted, 5);
        assert_eq!(lc.distilled, 2);
        assert_eq!(lc.expired, 3);
        assert_eq!(lc.demoted, 1);
        assert_eq!(lc.transient_ttl_days, 14);
        assert_eq!(lc.promotion_threshold, 5);
        assert_eq!(lc.canonical_threshold, 20);
    }

    #[spec("AGUI-010")]
    #[test]
    fn parse_context_strategies_all_fields() {
        let resp = serde_json::json!({
            "contextStrategies": [
                { "strategy": "Full", "completionRate": 0.95, "loopAvoidanceRate": 0.8,
                  "avgContextBytes": 8200, "compositeScore": 0.72, "burstCount": 15 },
                { "strategy": "Ledger", "completionRate": 0.82, "loopAvoidanceRate": 0.88,
                  "avgContextBytes": 2100, "compositeScore": 0.63, "burstCount": 11 }
            ]
        });
        let mut strategies = Vec::new();
        parse_context_strategies(&resp, &mut strategies);
        assert_eq!(strategies.len(), 2);
        assert_eq!(strategies[0].strategy, "Full");
        assert!((strategies[0].composite_score - 0.72).abs() < f64::EPSILON);
        assert_eq!(strategies[1].burst_count, 11);
    }

    // --- AGUI-010: All parsers handle empty/missing/malformed input safely ---

    #[spec("AGUI-010")]
    #[test]
    fn all_parsers_handle_empty_json() {
        let resp = serde_json::json!({});
        let mut rlm = default_rlm();
        let mut strategies = Vec::new();
        let mut tm = default_tool_memory();
        let mut traces = Vec::new();
        let mut patterns = Vec::new();
        let mut gossip = default_gossip();
        let mut lc = default_lifecycle();

        parse_rlm_snapshot(&resp, &mut rlm);
        parse_context_strategies(&resp, &mut strategies);
        parse_tool_memory(&resp, &mut tm);
        parse_decision_traces(&resp, &mut traces);
        parse_anti_patterns(&resp, &mut patterns);
        parse_gossip_snapshot(&resp, &mut gossip);
        parse_memory_lifecycle(&resp, &mut lc);

        assert_eq!(rlm.manifest_count, 0);
        assert!(strategies.is_empty());
        assert_eq!(tm.entry_count, 0);
        assert!(traces.is_empty());
        assert!(patterns.is_empty());
        assert_eq!(gossip.events_received, 0);
        assert_eq!(lc.promoted, 0);
    }

    #[spec("AGUI-010")]
    #[test]
    fn parsers_handle_wrong_types_gracefully() {
        let resp = serde_json::json!({
            "toolMemory": { "entryCount": "not_a_number", "hasObserveData": 42 },
            "gossip": { "eventsReceived": "bad", "gossipPeers": null },
            "rlm": { "manifestCount": false },
            "antiPatterns": "not_an_array",
            "decisionTraces": 12345,
        });
        let mut tm = default_tool_memory();
        let mut gossip = default_gossip();
        let mut rlm = default_rlm();
        let mut patterns = Vec::new();
        let mut traces = Vec::new();

        parse_tool_memory(&resp, &mut tm);
        parse_gossip_snapshot(&resp, &mut gossip);
        parse_rlm_snapshot(&resp, &mut rlm);
        parse_anti_patterns(&resp, &mut patterns);
        parse_decision_traces(&resp, &mut traces);

        // All fall back to defaults — no panics
        assert_eq!(tm.entry_count, 0);
        assert!(!tm.has_observe_data);
        assert_eq!(gossip.events_received, 0);
        assert_eq!(rlm.manifest_count, 0);
        assert!(patterns.is_empty());
        assert!(traces.is_empty());
    }

    #[spec("AGUI-010")]
    #[test]
    fn parsers_handle_partial_fields() {
        // Only some fields present — others default
        let resp = serde_json::json!({
            "toolMemory": { "entryCount": 3 },
            "gossip": { "gossipPeers": 5 },
            "decisionTraces": [{ "traceId": "t1", "selectedModel": "m1" }]
        });
        let mut tm = default_tool_memory();
        let mut gossip = default_gossip();
        let mut traces = Vec::new();

        parse_tool_memory(&resp, &mut tm);
        parse_gossip_snapshot(&resp, &mut gossip);
        parse_decision_traces(&resp, &mut traces);

        assert_eq!(tm.entry_count, 3);
        assert_eq!(tm.max_entries, 10); // default preserved
        assert_eq!(gossip.gossip_peers, 5);
        assert_eq!(gossip.events_received, 0); // missing → 0
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].task_category, "general"); // missing → default
        assert_eq!(traces[0].agent_id, ""); // missing → empty
    }
}
