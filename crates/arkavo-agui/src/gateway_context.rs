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
    let cache = context_topology_cache.read().await;
    for (_agent_id, resp) in cache.iter() {
        parse_tool_memory(resp, &mut tool_memory);
        parse_decision_traces(resp, &mut decision_traces);
        parse_anti_patterns(resp, &mut anti_patterns);
        parse_gossip_snapshot(resp, &mut gossip);
        parse_rlm_snapshot(resp, &mut rlm);
        parse_context_strategies(resp, &mut context_strategies);
        parse_memory_lifecycle(resp, &mut memory_lifecycle);
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
        tm.entry_count = t.get("entryCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        tm.max_entries = t.get("maxEntries").and_then(|v| v.as_u64()).unwrap_or(10) as usize;
        tm.error_count = t.get("errorCount").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        tm.duplicate_count = t
            .get("duplicateCount")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        tm.consecutive_same_type = t
            .get("consecutiveSameType")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize;
        tm.has_observe_data = t
            .get("hasObserveData")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if let Some(arr) = t.get("recentActionTypes").and_then(|v| v.as_array()) {
            tm.recent_action_types = arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect();
        }
    }
}

fn parse_decision_traces(resp: &serde_json::Value, traces: &mut Vec<DecisionTraceSnapshot>) {
    if let Some(arr) = resp.get("decisionTraces").and_then(|v| v.as_array()) {
        for t in arr {
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
            g.last_event_secs_ago = Some(t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_structs_serialize_roundtrip() {
        let rlm = RlmSnapshot {
            manifest_count: 3,
            total_chunks: 15,
            total_tokens: 4096,
            activation_threshold: 0.7,
        };
        let json = serde_json::to_string(&rlm).unwrap();
        let parsed: RlmSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.manifest_count, 3);
        assert_eq!(parsed.total_chunks, 15);
    }

    #[test]
    fn anti_pattern_snapshot_serialization() {
        let ap = AntiPatternSnapshot {
            model: "qwen-8b".to_string(),
            category: "code_generation".to_string(),
            failure_signature: "hallucinated_tool:read_file".to_string(),
            failure_count: 5,
            decayed_weight: 0.75,
            last_seen: "2026-03-23T10:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&ap).unwrap();
        assert!(json.contains("hallucinated_tool"));
        let parsed: AntiPatternSnapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.failure_count, 5);
    }

    #[test]
    fn parse_pushed_topology_data() {
        let resp = serde_json::json!({
            "toolMemory": {
                "entryCount": 5,
                "maxEntries": 10,
                "errorCount": 1,
                "duplicateCount": 0,
                "recentActionTypes": ["code_search", "file_read"],
                "consecutiveSameType": 2,
                "hasObserveData": true
            },
            "gossip": {
                "eventsReceived": 42,
                "episodesSynthesized": 3,
                "lessonsStored": 2,
                "gossipPeers": 3,
                "lastEventSecsAgo": 5
            },
            "antiPatterns": [{
                "model": "qwen-8b",
                "category": "code",
                "failureSignature": "timeout:build",
                "failureCount": 3,
                "decayedWeight": 0.5,
                "lastSeen": "2026-03-23T10:00:00Z"
            }],
            "decisionTraces": [{
                "traceId": "abc123",
                "agentId": "code-analyzer",
                "taskCategory": "code_generation",
                "selectedModel": "qwen-8b",
                "selectionReason": "ThompsonSampling",
                "budgetUsagePct": 45.0,
                "feasibleCount": 3,
                "timestamp": "2026-03-23T10:00:00Z"
            }]
        });
        let mut tm = ToolMemorySnapshot {
            entry_count: 0,
            max_entries: 10,
            error_count: 0,
            duplicate_count: 0,
            recent_action_types: vec![],
            consecutive_same_type: 0,
            has_observe_data: false,
        };
        let mut gossip = GossipSnapshot {
            events_received: 0,
            episodes_synthesized: 0,
            lessons_stored: 0,
            gossip_peers: 0,
            last_event_secs_ago: None,
        };
        let mut patterns = Vec::new();
        let mut traces = Vec::new();

        parse_tool_memory(&resp, &mut tm);
        parse_gossip_snapshot(&resp, &mut gossip);
        parse_anti_patterns(&resp, &mut patterns);
        parse_decision_traces(&resp, &mut traces);

        assert_eq!(tm.entry_count, 5);
        assert_eq!(tm.error_count, 1);
        assert!(tm.has_observe_data);
        assert_eq!(gossip.events_received, 42);
        assert_eq!(gossip.gossip_peers, 3);
        assert_eq!(patterns.len(), 1);
        assert_eq!(patterns[0].failure_signature, "timeout:build");
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0].selected_model, "qwen-8b");
    }

    #[test]
    fn parse_empty_response_is_safe() {
        let resp = serde_json::json!({});
        let mut rlm = RlmSnapshot {
            manifest_count: 0,
            total_chunks: 0,
            total_tokens: 0,
            activation_threshold: 0.7,
        };
        let mut strategies = Vec::new();
        let mut tm = ToolMemorySnapshot {
            entry_count: 0,
            max_entries: 10,
            error_count: 0,
            duplicate_count: 0,
            recent_action_types: vec![],
            consecutive_same_type: 0,
            has_observe_data: false,
        };
        parse_rlm_snapshot(&resp, &mut rlm);
        parse_context_strategies(&resp, &mut strategies);
        parse_tool_memory(&resp, &mut tm);
        assert_eq!(rlm.manifest_count, 0);
        assert!(strategies.is_empty());
        assert_eq!(tm.entry_count, 0);
    }
}
