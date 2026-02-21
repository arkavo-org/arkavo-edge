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
                match event {
                    LearningEvent::ToolCall {
                        tool_name,
                        args,
                        result,
                        success,
                        latency_ms,
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
                                    tracing::info!(
                                        "Episode synthesized: {} (success={})",
                                        episode.id,
                                        episode.outcome.success
                                    );

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

                                        match learning_bus
                                            .synthesize_lesson(&episodes, &lesson_category)
                                            .await
                                        {
                                            Ok(Some(lesson)) => {
                                                tracing::info!(
                                                    "Lesson synthesized: {} confidence={}",
                                                    lesson.pattern.condition,
                                                    lesson.confidence
                                                );

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
                        // Task completion can trigger immediate episode synthesis
                        // even before threshold is reached
                    }
                    LearningEvent::GossipReceived(_) => {
                        // Gossip messages are handled separately via handle_gossip
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
