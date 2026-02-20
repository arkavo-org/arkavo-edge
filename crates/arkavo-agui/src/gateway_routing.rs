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

    // Update store
    {
        let mut store = task_store.write().await;
        if let Some(task) = store.get_mut(task_id) {
            task.status = status.to_string();
            if let Some(p) = progress {
                task.progress = Some(p);
            }
            if let Some(ref r) = result {
                task.result = Some(r.clone());
            }
            if status == "completed" || status == "failed" {
                task.completed_at = Some(chrono::Utc::now().to_rfc3339());
            }
        }
    }

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
                        &task_id[..task_id.len().min(8)],
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

    // Broadcast status change to all connected UIs
    let event = AgUiEvent::TaskStatusChanged {
        task_id: task_id.to_string(),
        status: status.to_string(),
        progress,
        result,
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    broadcast_event(&event, connections).await;
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

pub(crate) async fn handle_request_learning_status(
    learning_module: &Arc<RwLock<LearningModule>>,
    routing_history: &Arc<RwLock<VecDeque<RoutingRecord>>>,
    tx: &tokio::sync::mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("AG-UI: Received RequestLearningStatus");

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

    let history: Vec<RoutingRecord> = routing_history.read().await.iter().cloned().collect();

    tx.send(AgUiEvent::LearningStatusUpdate {
        agents,
        routing_history: history,
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
    .await?;

    Ok(())
}
