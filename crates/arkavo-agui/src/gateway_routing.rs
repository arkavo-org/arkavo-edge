use crate::agent_connection::AgentConnection;
use crate::types::*;
use arkavo_router::learning::{BurstFeedback, LearningModule, Lesson};
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

#[allow(clippy::too_many_arguments)]
pub(crate) async fn update_task_status(
    task_store: &Arc<RwLock<HashMap<String, super::gateway::TrackedTask>>>,
    task_id: &str,
    status: &str,
    progress: Option<f32>,
    result: Option<String>,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    learning_module: &Arc<RwLock<LearningModule>>,
    routing_history: &Arc<RwLock<VecDeque<RoutingRecord>>>,
    lesson_tx: &Option<mpsc::Sender<Lesson>>,
) {
    // Get agent_id before updating store
    let agent_id = {
        let store = task_store.read().await;
        store.get(task_id).and_then(|t| t.target_agent.clone())
    };

    // Update store and capture timing data
    let (created_at, first_working_at) = {
        let mut store = task_store.write().await;
        if let Some(task) = store.get_mut(task_id) {
            task.status = status.to_string();
            if let Some(p) = progress {
                task.progress = Some(p);
            }
            if let Some(ref r) = result {
                task.result = Some(r.clone());
            }
            if status == "working" && task.first_working_at.is_none() {
                task.first_working_at = Some(chrono::Utc::now().to_rfc3339());
            }
            if status == "completed" || status == "failed" {
                task.completed_at = Some(chrono::Utc::now().to_rfc3339());
            }
            (task.created_at.clone(), task.first_working_at.clone())
        } else {
            (String::new(), None)
        }
    };

    // Judge + Learn: assess quality and feed back into Thompson Sampling
    let is_terminal = status == "completed" || status == "failed";
    if is_terminal && let Some(ref aid) = agent_id {
        let success = status == "completed";

        // Run the judge on completed tasks to assess actual quality
        let judgment = if success {
            if let Some(ref result_text) = result {
                let task_desc = {
                    let store = task_store.read().await;
                    store
                        .get(task_id)
                        .map(|t| t.description.clone())
                        .unwrap_or_default()
                };
                let j = crate::response_judge::judge(&task_desc, result_text);
                if !j.issues.is_empty() {
                    println!(
                        "AG-UI: Task {} quality={:.2}, issues: {:?}",
                        crate::gateway_events::short_id(task_id),
                        j.quality_score,
                        j.issues
                    );
                }
                Some(j)
            } else {
                None
            }
        } else {
            // Explicit failure = zero quality
            Some(crate::response_judge::TaskJudgment {
                quality_score: 0.0,
                issues: vec!["Task failed".into()],
                failure_modes: vec![],
            })
        };

        // Build quality-aware feedback for Thompson Sampling
        let quality_score = judgment.as_ref().map(|j| j.quality_score).unwrap_or(1.0);
        let quality_issues: Vec<String> = judgment
            .as_ref()
            .map(|j| j.issues.clone())
            .unwrap_or_default();

        let task_cat = {
            let store = task_store.read().await;
            store
                .get(task_id)
                .and_then(|t| t.task_category.clone())
                .unwrap_or_else(|| "general".to_string())
        };

        let feedback = if success {
            BurstFeedback::success(uuid::Uuid::new_v4(), task_cat.clone(), 0)
                .with_quality(quality_score)
        } else {
            BurstFeedback::failure(uuid::Uuid::new_v4(), task_cat.clone(), 0).with_quality(0.0)
        };

        learning_module
            .write()
            .await
            .immediate_update(aid, &feedback)
            .await;

        // Extract lesson from low-quality judgments and propagate via gossip
        if let Some(ref j) = judgment {
            let ctx = crate::lesson_extractor::LessonContext {
                agent_id: aid.clone(),
                task_category: task_cat.clone(),
            };
            if let Some(lesson) = crate::lesson_extractor::extract_lesson(j, &ctx) {
                println!(
                    "AG-UI: Lesson extracted for {} on {}: {}",
                    aid, task_cat, lesson.pattern.condition
                );
                if let Some(tx) = lesson_tx {
                    let _ = tx.try_send(lesson);
                }
            }
        }

        // Emit RoutingOutcome with quality info
        let outcome_event = AgUiEvent::RoutingOutcome {
            task_id: task_id.to_string(),
            agent_id: aid.clone(),
            success,
            quality_score,
            quality_issues: quality_issues.clone(),
            timestamp: chrono::Utc::now().to_rfc3339(),
        };
        broadcast_event(&outcome_event, connections).await;

        // Update routing history with outcome and quality
        let outcome_str = if success && quality_score > 0.5 {
            "success"
        } else if success {
            "degraded"
        } else {
            "failed"
        };
        let mut history = routing_history.write().await;
        for record in history.iter_mut() {
            if record.task_id == task_id {
                record.outcome = Some(outcome_str.to_string());
                record.quality_score = Some(quality_score);
                record.quality_issues = quality_issues.clone();
            }
        }
    }

    // Calculate inference metrics on terminal status
    let metrics = if (status == "completed" || status == "failed") && !created_at.is_empty() {
        compute_task_metrics(&created_at, &first_working_at, result.as_deref())
    } else {
        None
    };

    // Broadcast status change to all connected UIs
    let event = AgUiEvent::TaskStatusChanged {
        task_id: task_id.to_string(),
        status: status.to_string(),
        progress,
        result,
        metrics,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    broadcast_event(&event, connections).await;
}

/// GPU power draw in watts for energy calculations (configurable default)
const DEFAULT_GPU_WATTS: f64 = 150.0;

fn compute_task_metrics(
    created_at: &str,
    first_working_at: &Option<String>,
    result_text: Option<&str>,
) -> Option<TaskMetrics> {
    use chrono::DateTime;
    let created: DateTime<chrono::Utc> = DateTime::parse_from_rfc3339(created_at)
        .ok()?
        .with_timezone(&chrono::Utc);
    let completed = chrono::Utc::now();

    // TTFT: time from submission to first working status
    let ttft_ms = first_working_at
        .as_deref()
        .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
        .map(|working| {
            (working.with_timezone(&chrono::Utc) - created)
                .num_milliseconds()
                .max(0) as u64
        })
        .unwrap_or(0);

    // Inference duration: from first working to completion (excludes routing overhead)
    let inference_start = first_working_at
        .as_deref()
        .and_then(|ts| DateTime::parse_from_rfc3339(ts).ok())
        .map(|t| t.with_timezone(&chrono::Utc))
        .unwrap_or(created);
    let inference_duration_ms = (completed - inference_start).num_milliseconds().max(1) as u64;

    // Estimate token count from result text (~4 chars per token for English)
    let tokens_generated = result_text
        .map(|t| (t.len() as f64 / 4.0).ceil() as u32)
        .unwrap_or(0);

    let inference_secs = inference_duration_ms as f64 / 1000.0;
    let tokens_per_sec = if inference_secs > 0.0 {
        tokens_generated as f64 / inference_secs
    } else {
        0.0
    };

    // Energy: inference_duration_hours × GPU watts → Wh
    let inference_hours = inference_secs / 3600.0;
    let energy_wh = inference_hours * DEFAULT_GPU_WATTS;

    Some(TaskMetrics {
        tokens_generated,
        tokens_per_sec,
        ttft_ms,
        inference_duration_ms,
        energy_wh,
    })
}

pub(crate) async fn build_routing_candidates(
    agent_ids: &[String],
    learning_module: &Arc<RwLock<LearningModule>>,
    category: Option<&str>,
) -> Vec<RoutingCandidate> {
    let lm = learning_module.read().await;
    let mut candidates = Vec::with_capacity(agent_ids.len());

    for agent_id in agent_ids {
        let score = lm.thompson_sample(agent_id, category).await;
        let stats = lm.get_stats(agent_id).await;

        candidates.push(RoutingCandidate {
            agent_id: agent_id.clone(),
            score,
            alpha: stats.as_ref().map(|s| s.alpha).unwrap_or(2.0),
            beta_param: stats.as_ref().map(|s| s.beta_param).unwrap_or(1.0),
            observations: stats.as_ref().map(|s| s.total_observations).unwrap_or(0),
            success_rate: stats.as_ref().map(|s| s.success_rate).unwrap_or(0.0),
            probationary: stats.as_ref().map(|s| s.probationary).unwrap_or(true),
        });
    }

    candidates
}

pub(crate) async fn broadcast_event(
    event: &AgUiEvent,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
) {
    let conns = connections.read().await;
    for (_, conn_info) in conns.iter() {
        let _ = conn_info._ws_tx.send(event.clone()).await;
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn handle_request_learning_status(
    learning_module: &Arc<RwLock<LearningModule>>,
    routing_history: &Arc<RwLock<VecDeque<RoutingRecord>>>,
    lesson_count: usize,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    connections: &Arc<RwLock<HashMap<String, super::gateway::ConnectionInfo>>>,
    tx: &tokio::sync::mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("AG-UI: Received RequestLearningStatus");

    // Collect local learning data
    let lm = learning_module.read().await;
    let all_stats = lm.get_all_stats().await;

    let mut agents: Vec<AgentLearningInfo> = Vec::with_capacity(all_stats.len());
    for s in all_stats {
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
        agents.push(AgentLearningInfo {
            agent_id: s.agent_id,
            alpha: s.alpha,
            beta_param: s.beta_param,
            expected_value: s.expected_value,
            std_dev: s.std_dev,
            total_observations: s.total_observations,
            success_rate: s.success_rate,
            probationary: s.probationary,
            category_stats,
        });
    }
    drop(lm);

    // Query connected agents for their Thompson Sampling state and routing history
    let local_ids: std::collections::HashSet<String> =
        agents.iter().map(|a| a.agent_id.clone()).collect();
    let existing_task_ids: std::collections::HashSet<String> = {
        let hist = routing_history.read().await;
        hist.iter().map(|r| r.task_id.clone()).collect()
    };
    let conns = agent_connections.read().await;
    let mut new_records: Vec<RoutingRecord> = Vec::new();
    for (_id, conn) in conns.iter() {
        if !conn.is_connected().await {
            continue;
        }
        match conn
            .send_request("learning/status", serde_json::json!(null), "learning-poll")
            .await
        {
            Ok(resp) => {
                if let Some(remote_agents) = resp.get("agents").and_then(|v| v.as_array()) {
                    for ra in remote_agents {
                        let agent_id = ra
                            .get("agentId")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        if agent_id.is_empty() || local_ids.contains(&agent_id) {
                            continue;
                        }
                        agents.push(parse_remote_agent_info(ra));
                    }
                }
                // Parse routing history from agent response (Bugs 2, 4, 5)
                if let Some(remote_history) = resp.get("routingHistory").and_then(|v| v.as_array())
                {
                    for rh in remote_history {
                        let record = parse_remote_routing_record(rh);
                        if !record.task_id.is_empty()
                            && !existing_task_ids.contains(&record.task_id)
                        {
                            new_records.push(record);
                        }
                    }
                }
            }
            Err(e) => {
                println!("AG-UI: Failed to query agent learning/status: {e}");
            }
        }
    }
    drop(conns);

    // Merge agent routing records and emit events for new ones
    if !new_records.is_empty() {
        let mut hist = routing_history.write().await;
        for record in &new_records {
            // Emit ModelSelected for Router Decisions tab (Bug 2)
            let model_event = AgUiEvent::ModelSelected {
                agent_id: record.selected_agent.clone(),
                provider: "local".to_string(),
                model: record.selected_agent.clone(),
                estimated_cost: arkavo_budget::TokenCost::ZERO,
                reason: record
                    .category
                    .clone()
                    .unwrap_or_else(|| "agent-internal".to_string()),
                event_id: uuid::Uuid::new_v4().to_string(),
            };
            broadcast_event(&model_event, connections).await;

            // Emit TelemetryEvent for Telemetry tab (Bug 4)
            let telemetry = AgUiEvent::TelemetryEvent {
                event_type: "routing_decision".to_string(),
                agent_id: record.selected_agent.clone(),
                details: serde_json::json!({
                    "taskId": record.task_id,
                    "wasExploration": record.was_exploration,
                    "category": record.category,
                    "qualityScore": record.quality_score,
                }),
                timestamp: record.timestamp.clone(),
            };
            broadcast_event(&telemetry, connections).await;

            hist.push_back(record.clone());
        }
        // Cap history at 200 entries
        while hist.len() > 200 {
            hist.pop_front();
        }
    }

    let history: Vec<RoutingRecord> = routing_history.read().await.iter().cloned().collect();
    let quality_trends = derive_quality_trends(&history);

    tx.send(AgUiEvent::LearningStatusUpdate {
        agents,
        routing_history: history,
        quality_trends,
        lesson_count,
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
    .await?;

    Ok(())
}

fn parse_remote_routing_record(v: &serde_json::Value) -> RoutingRecord {
    RoutingRecord {
        task_id: v
            .get("taskId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        selected_agent: v
            .get("selectedAgent")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        was_exploration: v
            .get("wasExploration")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        outcome: v.get("outcome").and_then(|v| v.as_str()).map(String::from),
        quality_score: v.get("qualityScore").and_then(|v| v.as_f64()),
        quality_issues: v
            .get("qualityIssues")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        category: v.get("category").and_then(|v| v.as_str()).map(String::from),
        timestamp: v
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
    }
}

fn parse_remote_agent_info(v: &serde_json::Value) -> AgentLearningInfo {
    let cat_stats = v
        .get("categoryStats")
        .and_then(|cs| cs.as_array())
        .map(|arr| {
            arr.iter()
                .map(|c| CategoryStat {
                    category: c
                        .get("category")
                        .and_then(|v| v.as_str())
                        .unwrap_or("general")
                        .to_string(),
                    alpha: c.get("alpha").and_then(|v| v.as_f64()).unwrap_or(2.0),
                    beta_param: c.get("betaParam").and_then(|v| v.as_f64()).unwrap_or(1.0),
                    expected_value: c
                        .get("expectedValue")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.667),
                    observations: c.get("observations").and_then(|v| v.as_u64()).unwrap_or(0),
                })
                .collect()
        })
        .unwrap_or_default();

    AgentLearningInfo {
        agent_id: v
            .get("agentId")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        alpha: v.get("alpha").and_then(|v| v.as_f64()).unwrap_or(2.0),
        beta_param: v.get("betaParam").and_then(|v| v.as_f64()).unwrap_or(1.0),
        expected_value: v
            .get("expectedValue")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.667),
        std_dev: v.get("stdDev").and_then(|v| v.as_f64()).unwrap_or(0.0),
        total_observations: v
            .get("totalObservations")
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
        success_rate: v.get("successRate").and_then(|v| v.as_f64()).unwrap_or(0.0),
        probationary: v
            .get("probationary")
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        category_stats: cat_stats,
    }
}

/// Derive quality trends from routing history records
pub(crate) fn derive_quality_trends(history: &[RoutingRecord]) -> Vec<crate::types::QualityTrend> {
    let mut trends: HashMap<(String, String), Vec<f64>> = HashMap::new();
    for record in history {
        if let Some(score) = record.quality_score {
            let category = record.category.clone().unwrap_or_else(|| "general".into());
            let key = (record.selected_agent.clone(), category);
            trends.entry(key).or_default().push(score);
        }
    }
    trends
        .into_iter()
        .map(
            |((agent_id, category), scores)| crate::types::QualityTrend {
                agent_id,
                category,
                scores,
            },
        )
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(agent: &str, cat: Option<&str>, score: Option<f64>) -> RoutingRecord {
        RoutingRecord {
            task_id: uuid::Uuid::new_v4().to_string(),
            selected_agent: agent.to_string(),
            was_exploration: false,
            outcome: Some("completed".to_string()),
            quality_score: score,
            quality_issues: vec![],
            category: cat.map(|c| c.to_string()),
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    #[test]
    fn test_derive_trends_empty_history() {
        let trends = derive_quality_trends(&[]);
        assert!(trends.is_empty());
    }

    #[test]
    fn test_derive_trends_single_agent() {
        let history = vec![
            record("agent-1", Some("code"), Some(0.3)),
            record("agent-1", Some("code"), Some(0.5)),
            record("agent-1", Some("code"), Some(0.9)),
        ];
        let trends = derive_quality_trends(&history);
        assert_eq!(trends.len(), 1);
        assert_eq!(trends[0].agent_id, "agent-1");
        assert_eq!(trends[0].category, "code");
        assert_eq!(trends[0].scores, vec![0.3, 0.5, 0.9]);
    }

    #[test]
    fn test_derive_trends_multiple_agents() {
        let history = vec![
            record("agent-1", Some("code"), Some(0.4)),
            record("agent-2", Some("code"), Some(0.8)),
        ];
        let trends = derive_quality_trends(&history);
        assert_eq!(trends.len(), 2);

        let a1 = trends.iter().find(|t| t.agent_id == "agent-1").unwrap();
        let a2 = trends.iter().find(|t| t.agent_id == "agent-2").unwrap();
        assert_eq!(a1.scores, vec![0.4]);
        assert_eq!(a2.scores, vec![0.8]);
    }

    #[test]
    fn test_derive_trends_missing_quality_score() {
        let history = vec![
            record("agent-1", Some("code"), Some(0.5)),
            record("agent-1", Some("code"), None),
            record("agent-1", Some("code"), Some(0.7)),
        ];
        let trends = derive_quality_trends(&history);
        assert_eq!(trends.len(), 1);
        // The None record is skipped
        assert_eq!(trends[0].scores, vec![0.5, 0.7]);
    }

    #[test]
    fn test_derive_trends_missing_category_defaults_to_general() {
        let history = vec![
            record("agent-1", None, Some(0.6)),
            record("agent-1", None, Some(0.8)),
        ];
        let trends = derive_quality_trends(&history);
        assert_eq!(trends.len(), 1);
        assert_eq!(trends[0].category, "general");
        assert_eq!(trends[0].scores, vec![0.6, 0.8]);
    }

    #[test]
    fn test_derive_trends_preserves_insertion_order() {
        let history = vec![
            record("agent-1", Some("code"), Some(0.1)),
            record("agent-1", Some("code"), Some(0.9)),
            record("agent-1", Some("code"), Some(0.5)),
        ];
        let trends = derive_quality_trends(&history);
        assert_eq!(trends[0].scores, vec![0.1, 0.9, 0.5]);
    }
}
