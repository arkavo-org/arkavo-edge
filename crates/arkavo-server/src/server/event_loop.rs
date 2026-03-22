//! Event processing loops for the learning system
//!
//! Background tasks that process learning events and apply lessons.

use std::sync::Arc;

use arkavo_gossip::LessonAnnouncement;
use arkavo_router::learning::{Lesson, LessonPattern};
use chrono::Utc;
use tokio::sync::broadcast;
use tokio::sync::mpsc;

use super::episode_buffer::ToolObservation;
use super::learning_bus::{LearningBus, LearningEvent};
use super::synthesis::parse_lesson_from_historian;

/// Delegate lesson synthesis to the historian agent via A2A message/send.
/// Returns Ok(Some(lesson)) if historian produces a valid lesson, Ok(None) if
/// no pattern found, Err if transport or parsing fails (caller falls back to local).
async fn delegate_to_historian(
    historian_addr: &str,
    episodes: &[arkavo_router::learning::Episode],
    category: &str,
    agent_id: &str,
    swarm_id: &str,
) -> Result<Option<Lesson>, String> {
    use arkavo_protocol::http::HttpTransport;
    use arkavo_protocol::transport::{
        A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, TransportConfig,
    };

    let eps_json: Vec<serde_json::Value> = episodes
        .iter()
        .map(|e| {
            serde_json::json!({
                "category": e.task_category,
                "action": e.observation.action_taken,
                "success": e.outcome.success,
                "quality": e.outcome.quality_metrics.correctness,
                "tools": e.observation.tools_used
            })
        })
        .collect();

    let failure_count = episodes.iter().filter(|e| !e.outcome.success).count();
    let success_count = episodes.iter().filter(|e| e.outcome.success).count();

    let prompt = format!(
        "Analyze these {} episodes ({} successes, {} failures) in category '{}' \
         and extract a reusable lesson.\n\n{}\n\n\
         Respond with a JSON lesson or NO_LESSON.",
        episodes.len(),
        success_count,
        failure_count,
        category,
        serde_json::to_string_pretty(&eps_json).unwrap_or_default()
    );

    let mut config = TransportConfig::default();
    config.tls_config.require_tls = false;
    config.timeout_ms = 60_000; // historian may take a while with 9b model

    let transport = HttpTransport::new(config).map_err(|e| format!("Transport: {e}"))?;
    let endpoint = A2aEndpoint {
        url: historian_addr.to_string(),
        agent_id: "historian".to_string(),
        public_key: None,
    };
    transport
        .connect(&endpoint)
        .await
        .map_err(|e| format!("Connect: {e}"))?;

    let message = serde_json::json!({
        "parts": [{"type": "text", "content": prompt}]
    });
    let request = A2aRequest::new("message/send", serde_json::json!([{"message": message}]));

    let response = transport
        .send_request(request)
        .await
        .map_err(|e| format!("Request: {e}"))?;

    let _ = transport.close().await;

    // Extract task_id from response, then poll for result
    let task_id = match response {
        A2aResponse::Success { result, .. } => result
            .get("task_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or("No task_id in historian response")?,
        A2aResponse::Error { error, .. } => {
            return Err(format!("Historian RPC error: {}", error.message));
        }
    };

    // Poll for completion (historian runs inference asynchronously)
    for _ in 0..30 {
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;

        let transport2 = HttpTransport::new({
            let mut c = TransportConfig::default();
            c.tls_config.require_tls = false;
            c.timeout_ms = 5_000;
            c
        })
        .map_err(|e| format!("Transport: {e}"))?;
        transport2
            .connect(&endpoint)
            .await
            .map_err(|e| format!("Connect: {e}"))?;

        let get_request = A2aRequest::new("tasks/get", serde_json::json!([{"task_id": task_id}]));
        let get_response = transport2
            .send_request(get_request)
            .await
            .map_err(|e| format!("Poll: {e}"))?;
        let _ = transport2.close().await;

        if let A2aResponse::Success { result, .. } = get_response {
            let status = result.get("status").and_then(|v| v.as_str()).unwrap_or("");
            if status == "completed" || status == "failed" {
                let text = result
                    .get("result")
                    .and_then(|v| v.get("parts"))
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                    .and_then(|p| p.get("content"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("");

                tracing::info!(
                    task_id = %task_id,
                    response_len = text.len(),
                    "Historian synthesis completed"
                );

                return parse_lesson_from_historian(
                    text,
                    agent_id,
                    swarm_id,
                    category,
                    episodes.len(),
                );
            }
        }
    }

    Err("Historian timed out after 60s".to_string())
}

/// Start the lesson application loop that processes approved lessons
///
/// This function should be spawned as a background task. It listens for
/// lesson approvals from the gossip protocol and adds them to the policy cache.
pub async fn start_lesson_application_loop(
    learning_bus: Arc<LearningBus>,
    mut lesson_rx: broadcast::Receiver<LessonAnnouncement>,
) {
    tracing::info!("Starting lesson application loop");

    loop {
        match lesson_rx.recv().await {
            Ok(announcement) => {
                tracing::info!(
                    "Received approved lesson {}: {} from {}",
                    announcement.lesson_id,
                    announcement.category,
                    announcement.originator
                );

                // Convert announcement to Lesson using pattern metadata if available
                let condition = announcement
                    .condition
                    .clone()
                    .unwrap_or_else(|| announcement.category.clone());
                let action = announcement
                    .action
                    .clone()
                    .unwrap_or_else(|| "adjust approach".to_string());
                let expected_outcome = announcement
                    .expected_outcome
                    .clone()
                    .unwrap_or_else(|| "improved quality".to_string());

                let lesson = Lesson::new(
                    announcement.originator.clone(),
                    learning_bus.swarm_id().to_string(),
                    announcement.category.clone(),
                    LessonPattern::new(condition, action, expected_outcome),
                    announcement.confidence,
                    1,
                );

                learning_bus.add_lesson_to_cache(lesson).await;

                tracing::info!(
                    "Added lesson {} to policy cache (total: {})",
                    announcement.lesson_id,
                    learning_bus.cached_lesson_count().await
                );
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("Lesson application loop lagged by {} messages", n);
            }
            Err(broadcast::error::RecvError::Closed) => {
                tracing::info!("Lesson application loop channel closed, exiting");
                break;
            }
        }
    }
}

/// Start the event processing loop that converts observations to episodes and lessons
///
/// This function should be spawned as a background task. It:
/// 1. Receives LearningEvents from tool calls
/// 2. Buffers observations
/// 3. Synthesizes episodes when threshold reached
/// 4. Synthesizes lessons from episodes
/// 5. Announces lessons via gossip
pub async fn start_event_processing_loop(
    learning_bus: Arc<LearningBus>,
    mut event_rx: mpsc::Receiver<LearningEvent>,
) {
    tracing::info!("Starting event processing loop");

    loop {
        match event_rx.recv().await {
            Some(event) => {
                learning_bus.record_event();
                match event {
                    LearningEvent::ToolCall {
                        tool_name,
                        args,
                        result,
                        success,
                        latency_ms,
                        decision_trace_id,
                        step_index,
                        model_name,
                    } => {
                        tracing::debug!(
                            "Event processing: tool={} success={} latency={}ms",
                            tool_name,
                            success,
                            latency_ms
                        );

                        // Create observation
                        let obs = ToolObservation {
                            tool_name,
                            args,
                            result,
                            success,
                            latency_ms,
                            timestamp: Utc::now(),
                            decision_trace_id,
                            step_index,
                            model_name,
                        };

                        // Add to buffer
                        {
                            let mut buffer = learning_bus.episode_buffer().write().await;
                            buffer.add_observation(obs);
                        }

                        // Check if ready for episode synthesis
                        let category_ready = {
                            let buffer = learning_bus.episode_buffer().read().await;
                            buffer.ready_for_episode_synthesis()
                        };

                        if let Some(category) = category_ready {
                            let observations = {
                                let mut buffer = learning_bus.episode_buffer().write().await;
                                buffer.take_observations(&category)
                            };

                            tracing::info!(
                                "Synthesizing episode from {} observations in {}",
                                observations.len(),
                                category
                            );

                            match learning_bus
                                .synthesize_episode(&observations, &category)
                                .await
                            {
                                Ok(episode) => {
                                    learning_bus.record_episode_synthesized();
                                    tracing::info!(
                                        "Episode synthesized: {} (success={})",
                                        episode.id,
                                        episode.outcome.success
                                    );

                                    // Persist episode to SQLite
                                    if let Some(ref store) =
                                        *learning_bus.learning_store.read().await
                                        && let Err(e) = store.store_episode(&episode).await
                                    {
                                        tracing::warn!("Failed to persist episode: {e}");
                                    }

                                    // Index in case retrieval for similarity search
                                    let summary = format!(
                                        "{}: {}",
                                        episode.observation.action_taken,
                                        if episode.outcome.success {
                                            "succeeded"
                                        } else {
                                            "failed"
                                        }
                                    );
                                    let quality = episode.outcome.quality_metrics.overall();
                                    let case_index = learning_bus.case_index().clone();
                                    let ep_id = episode.id;
                                    let ep_cat = episode.task_category.clone();
                                    let ep_success = episode.outcome.success;
                                    let ep_domain = episode.swarm_id.clone();
                                    tokio::spawn(async move {
                                        if let Err(e) = case_index
                                            .index_episode(
                                                ep_id,
                                                &summary,
                                                &ep_cat,
                                                quality,
                                                ep_success,
                                                arkavo_memory::IndexMetadata {
                                                    domain: Some(ep_domain),
                                                },
                                            )
                                            .await
                                        {
                                            tracing::debug!("Case indexing skipped: {e}");
                                        }
                                    });

                                    // Add to buffer
                                    {
                                        let mut buffer =
                                            learning_bus.episode_buffer().write().await;
                                        buffer.add_episode(episode);
                                    }

                                    // Check if ready for lesson synthesis
                                    let lesson_category_ready = {
                                        let buffer = learning_bus.episode_buffer().read().await;
                                        buffer.ready_for_lesson_synthesis()
                                    };

                                    if let Some(lesson_category) = lesson_category_ready {
                                        let episodes = {
                                            let mut buffer =
                                                learning_bus.episode_buffer().write().await;
                                            buffer.take_episodes(&lesson_category)
                                        };

                                        tracing::info!(
                                            "Synthesizing lesson from {} episodes in {}",
                                            episodes.len(),
                                            lesson_category
                                        );

                                        // Try historian agent first (9b, high quality),
                                        // fall back to local route_fast (0.8b) if unavailable.
                                        let lesson_result = if let Some(addr) =
                                            learning_bus.get_peer_address("historian").await
                                        {
                                            tracing::info!(
                                                "Delegating lesson synthesis to historian at {addr}"
                                            );
                                            delegate_to_historian(
                                                &addr,
                                                &episodes,
                                                &lesson_category,
                                                learning_bus.agent_id(),
                                                learning_bus.swarm_id(),
                                            )
                                            .await
                                        } else {
                                            learning_bus
                                                .synthesize_lesson(&episodes, &lesson_category)
                                                .await
                                        };

                                        match lesson_result {
                                            Ok(Some(lesson)) => {
                                                tracing::info!(
                                                    "Lesson synthesized: {} confidence={}",
                                                    lesson.pattern.condition,
                                                    lesson.confidence
                                                );

                                                // Persist to SQLite
                                                if let Some(ref store) =
                                                    *learning_bus.learning_store.read().await
                                                    && let Err(e) =
                                                        store.store_lesson(&lesson).await
                                                {
                                                    tracing::warn!(
                                                        "Failed to persist synthesized lesson: {e}"
                                                    );
                                                }

                                                // Create announcement and gossip
                                                let announcement = LessonAnnouncement::new(
                                                    lesson.id,
                                                    [0u8; 32], // Hash computed during signing
                                                    learning_bus.agent_id().to_string(),
                                                    learning_bus.swarm_id().to_string(),
                                                    lesson.category.clone(),
                                                    lesson.confidence,
                                                )
                                                .with_pattern(
                                                    lesson.pattern.condition.clone(),
                                                    lesson.pattern.action.clone(),
                                                    lesson.pattern.expected_outcome.clone(),
                                                );

                                                if let Err(e) =
                                                    learning_bus.announce_lesson(announcement).await
                                                {
                                                    tracing::error!(
                                                        "Failed to announce lesson: {}",
                                                        e
                                                    );
                                                }
                                            }
                                            Ok(None) => {
                                                tracing::debug!(
                                                    "No lesson pattern found in {} episodes",
                                                    episodes.len()
                                                );
                                            }
                                            Err(e) => {
                                                tracing::error!(
                                                    "Failed to synthesize lesson: {}",
                                                    e
                                                );
                                            }
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("Failed to synthesize episode: {}", e);
                                }
                            }
                        }
                    }
                    LearningEvent::TaskComplete {
                        task_id,
                        category,
                        success,
                        ..
                    } => {
                        tracing::debug!(
                            "Task complete: {} category={} success={}",
                            task_id,
                            category,
                            success
                        );
                        // Periodic consolidation + axiom promotion
                        if learning_bus.should_consolidate() {
                            let before = {
                                let cache = learning_bus.policy_cache().read().await;
                                cache.behavior_lesson_count()
                            };
                            let (merged, promoted) = {
                                let mut cache = learning_bus.policy_cache().write().await;
                                cache.run_consolidation()
                            };
                            let after = {
                                let cache = learning_bus.policy_cache().read().await;
                                cache.behavior_lesson_count()
                            };
                            // Contradiction detection after consolidation (1 LLM call per category)
                            let contradictions = learning_bus.detect_contradictions().await;

                            // Curiosity: generate exploration tasks from uncertainty gaps
                            let curiosity_tasks = learning_bus.generate_curiosity_tasks().await;

                            if merged > 0
                                || promoted > 0
                                || contradictions > 0
                                || !curiosity_tasks.is_empty()
                            {
                                tracing::info!(
                                    before,
                                    after,
                                    merged,
                                    promoted,
                                    contradictions,
                                    curiosity = curiosity_tasks.len(),
                                    "Consolidation: {merged} merged, {promoted} axioms, {contradictions} contradictions, {} curiosity tasks",
                                    curiosity_tasks.len()
                                );
                            }

                            // Log curiosity tasks (execution is handled by the orchestrator)
                            for task in &curiosity_tasks {
                                tracing::info!(task = %task, "Curiosity exploration task generated");
                            }
                        }
                    }
                    LearningEvent::GossipReceived(_) => {
                        // Gossip messages are handled separately via handle_gossip
                    }
                    LearningEvent::HumanCorrection {
                        text,
                        trace_id,
                        model_name,
                    } => {
                        tracing::info!(
                            "Human correction received: {} (trace={:?})",
                            &text[..text.len().min(60)],
                            trace_id
                        );
                        learning_bus
                            .record_human_correction(&text, trace_id, model_name.as_deref())
                            .await;
                    }
                    LearningEvent::HumanReinforcement { trace_id, .. } => {
                        tracing::info!("Human reinforcement received (trace={:?})", trace_id);
                        learning_bus.record_human_reinforcement(trace_id).await;
                    }
                }
            }
            None => {
                tracing::info!("Event processing loop channel closed, exiting");
                break;
            }
        }
    }
}
