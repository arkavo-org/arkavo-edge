//! Integration tests for the agent-side learning loop
//!
//! Tests the full mesh-side pipeline:
//! lesson → PolicyCache → behavior guidance → lesson_tx → multi-round accumulation

use std::sync::Arc;

use arkavo_crypto::AgentKeypair;
use arkavo_gossip::GossipConfig;
use arkavo_router::learning::{Lesson, LessonPattern};
use arkavo_server::{LearningBus, PolicyCache};
use arkavo_test_macros::spec;

fn make_bus() -> LearningBus {
    let keypair = Arc::new(AgentKeypair::generate());
    LearningBus::new(
        "test-agent".to_string(),
        "test-swarm".to_string(),
        keypair,
        GossipConfig::default(),
    )
}

fn behavior_lesson(agent: &str, category: &str, condition: &str, quality: f64) -> Lesson {
    Lesson::new(
        agent.to_string(),
        "local".to_string(),
        category.to_string(),
        LessonPattern::new(
            condition.to_string(),
            format!("Reduce routing weight for {agent}"),
            "Substantive response".to_string(),
        )
        .with_metadata(serde_json::json!({"quality_score": quality})),
        1.0 - quality,
        1,
    )
}

/// Multi-round learning: lessons accumulate and guidance grows across rounds
#[spec("SRV-004")]
#[tokio::test]
async fn test_multi_round_guidance_accumulation() {
    let bus = make_bus();

    // Round 1: no lessons, no guidance
    let guidance = bus.get_behavior_guidance(None).await;
    assert!(guidance.is_empty(), "Round 1 should have no guidance");

    // Simulate Round 1 failures producing lessons
    bus.add_lesson_to_cache(behavior_lesson(
        "agent-1",
        "code",
        "Agent agent-1 returns generic non-answers for code",
        0.2,
    ))
    .await;

    // Round 2: guidance should contain the Round 1 lesson
    let guidance = bus.get_behavior_guidance(None).await;
    assert!(
        guidance.contains("generic non-answers"),
        "Round 2 should inject Round 1 failure lesson"
    );
    assert!(
        guidance.contains("RULES (follow these strictly):"),
        "Should have guidance header"
    );
    assert_eq!(bus.behavior_lesson_count().await, 1);

    // Simulate Round 2 failures producing more lessons
    bus.add_lesson_to_cache(behavior_lesson(
        "agent-1",
        "code",
        "Agent agent-1 produces overly brief code responses",
        0.3,
    ))
    .await;

    // Round 3: guidance should contain both lessons
    let guidance = bus.get_behavior_guidance(None).await;
    assert!(guidance.contains("generic non-answers"));
    assert!(guidance.contains("overly brief"));
    assert_eq!(bus.behavior_lesson_count().await, 2);
}

/// New category gets no guidance while existing categories retain theirs
#[spec("SRV-004")]
#[tokio::test]
async fn test_new_category_clean_slate() {
    let bus = make_bus();

    // Seed "code" category with lessons
    bus.add_lesson_to_cache(behavior_lesson(
        "agent-1",
        "code",
        "Agent returns empty code responses",
        0.0,
    ))
    .await;

    // "code" has guidance
    let code_guidance = bus.get_behavior_guidance(Some("code")).await;
    assert!(!code_guidance.is_empty());

    // "security" (new category) has no guidance
    let sec_guidance = bus.get_behavior_guidance(Some("security")).await;
    assert!(
        sec_guidance.is_empty(),
        "New category should have clean slate"
    );
}

/// lesson_tx -> PolicyCache async flow: lessons sent through channel appear in guidance
#[spec("SRV-004")]
#[tokio::test]
async fn test_lesson_tx_to_guidance_flow() {
    let bus = make_bus();
    let (tx, rx) = tokio::sync::mpsc::channel(16);

    bus.start_lesson_receiver(rx);

    // Send two lessons through the channel
    let l1 = behavior_lesson("agent-x", "code", "Agent agent-x returns empty", 0.0);
    let l2 = behavior_lesson(
        "agent-y",
        "security",
        "Agent agent-y hallucinates tools",
        0.1,
    );
    tx.send(l1).await.unwrap();
    tx.send(l2).await.unwrap();

    // Wait for the spawned task to process
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Both lessons should be in the policy cache
    assert!(bus.cached_lesson_count().await >= 2);

    // Guidance should contain both conditions
    let guidance = bus.get_behavior_guidance(None).await;
    assert!(guidance.contains("empty"));
    assert!(guidance.contains("hallucinates"));

    // Quality trends should have entries from lesson metadata
    let trends = bus.get_quality_trends().await;
    assert!(
        trends.len() >= 2,
        "Should have quality trends for both agent/category pairs"
    );
}

/// Quality scores propagate from lesson metadata into quality trends
#[spec("SRV-004")]
#[tokio::test]
async fn test_quality_score_propagation_via_lesson() {
    let bus = make_bus();

    let lesson = behavior_lesson("agent-1", "code", "some condition", 0.2);
    bus.add_lesson_to_cache(lesson).await;

    let trends = bus.get_quality_trends().await;
    let code_trend = trends
        .iter()
        .find(|t| t.agent_id == "agent-1" && t.category == "code");
    assert!(code_trend.is_some(), "Should have a trend for agent-1/code");
    assert_eq!(code_trend.unwrap().scores, vec![0.2]);
}

/// Deduplication: identical lessons don't produce duplicate guidance lines
#[spec("SRV-004")]
#[tokio::test]
async fn test_duplicate_lessons_deduplicated_in_guidance() {
    let bus = make_bus();

    // Add the same lesson 3 times
    for _ in 0..3 {
        bus.add_lesson_to_cache(behavior_lesson(
            "agent-1",
            "code",
            "Agent agent-1 returns generic non-answers for code",
            0.2,
        ))
        .await;
    }

    let guidance = bus.get_behavior_guidance(Some("code")).await;
    let count = guidance.matches("generic non-answers").count();
    assert_eq!(
        count, 1,
        "Identical conditions should appear once, got {count}"
    );
}

/// PolicyCache directly: verify ring buffer eviction preserves newest lessons
#[spec("SRV-004")]
#[test]
fn test_policy_cache_ring_buffer_newest_retained() {
    let mut cache = PolicyCache::new();

    // Add 8 lessons to the same (agent, category) — ring buffer is 5
    for i in 0..8 {
        let lesson = Lesson::new(
            "a".to_string(),
            "s".to_string(),
            "cat".to_string(),
            LessonPattern::new(
                format!("condition-{i}"),
                "action".to_string(),
                "outcome".to_string(),
            ),
            0.8,
            1,
        );
        cache.add_lesson(lesson);
    }

    let guidance = cache.get_behavior_guidance(Some("cat"));
    // Oldest 3 (0, 1, 2) should be evicted
    assert!(!guidance.contains("condition-0"));
    assert!(!guidance.contains("condition-1"));
    assert!(!guidance.contains("condition-2"));
    // Newest 5 (3, 4, 5, 6, 7) should remain
    assert!(guidance.contains("condition-3"));
    assert!(guidance.contains("condition-7"));
}

/// Anti-patterns recorded from tool errors appear in behavior guidance
#[spec("SRV-006")]
#[tokio::test]
async fn test_anti_pattern_in_guidance() {
    let bus = make_bus();

    // Record a tool failure as anti-pattern
    bus.record_human_correction("don't hunt wolves", None, Some("model-a"))
        .await;

    let guidance = bus.get_behavior_guidance(None).await;
    assert!(
        guidance.contains("AVOID"),
        "Anti-pattern should appear as avoidance warning in guidance: {guidance}"
    );
}

/// Case retrieval returns empty when no episodes indexed
#[spec("SRV-004")]
#[tokio::test]
async fn test_case_retrieval_empty_index() {
    let bus = make_bus();
    let context = bus
        .get_case_context("how do I fix food shortage?", None, None)
        .await;
    assert!(
        context.is_empty(),
        "Empty case index should return empty context"
    );
}

/// EpisodeBuffer: observations + episodes flow through thresholds correctly
#[spec("SRV-004")]
#[test]
fn test_episode_buffer_full_lifecycle() {
    use arkavo_router::learning::{Episode, EpisodeOutcome, Observation, QualityMetrics};
    use arkavo_server::ToolObservation;

    let mut buffer = arkavo_server::EpisodeBuffer::with_thresholds(2, 2);

    // Phase 1: Accumulate observations until episode synthesis triggers
    buffer.add_observation(ToolObservation {
        tool_name: "navigate_to".to_string(),
        args: serde_json::json!({"sector": 4}),
        result: "arrived".to_string(),
        success: true,
        latency_ms: 50,
        timestamp: chrono::Utc::now(),
        decision_trace_id: None,
        step_index: 0,
        model_name: None,
    });
    assert!(buffer.ready_for_episode_synthesis().is_none());

    buffer.add_observation(ToolObservation {
        tool_name: "sector_scan".to_string(),
        args: serde_json::json!({}),
        result: "clear".to_string(),
        success: true,
        latency_ms: 30,
        timestamp: chrono::Utc::now(),
        decision_trace_id: None,
        step_index: 0,
        model_name: None,
    });
    assert_eq!(
        buffer.ready_for_episode_synthesis(),
        Some("navigation".to_string())
    );

    // Take observations for synthesis
    let obs = buffer.take_observations("navigation");
    assert_eq!(obs.len(), 2);
    assert!(buffer.ready_for_episode_synthesis().is_none());

    // Phase 2: Accumulate episodes until lesson synthesis triggers
    for i in 0..2 {
        let ep = Episode::new(
            "agent-1".to_string(),
            "swarm-1".to_string(),
            uuid::Uuid::new_v4(),
            "navigation".to_string(),
            Observation::new(
                serde_json::json!({}),
                format!("nav-action-{i}"),
                serde_json::json!({}),
                vec!["navigate_to".to_string()],
            ),
            EpisodeOutcome::new(true, QualityMetrics::new(0.9, 0.9, 1.0), 50, 0.0),
        );
        buffer.add_episode(ep);
    }
    assert_eq!(
        buffer.ready_for_lesson_synthesis(),
        Some("navigation".to_string())
    );

    let episodes = buffer.take_episodes("navigation");
    assert_eq!(episodes.len(), 2);
    assert!(buffer.ready_for_lesson_synthesis().is_none());
}

/// Retrospective update with per-step rewards applies correct credit
#[spec("SRV-004")]
#[tokio::test]
async fn test_retrospective_per_step_rewards() {
    use arkavo_router::learning::{AgentContribution, FinalTaskReport, LearningModule};

    let module = LearningModule::default();

    // Two-step task: first step high quality, second step low quality
    let contributions = vec![
        AgentContribution {
            agent_id: "model-a".to_string(),
            position: 0,
            immediate_reward: 0.8,
        },
        AgentContribution {
            agent_id: "model-b".to_string(),
            position: 1,
            immediate_reward: 0.3,
        },
    ];
    let mut report = FinalTaskReport::success(uuid::Uuid::new_v4(), contributions);
    report.per_step_rewards = vec![0.9, 0.2]; // model-a did great, model-b did poorly

    module.retrospective_update(&report).await;

    // Sample many times — model-a should have higher average Thompson score
    // model-a blended: 0.5*0.8 + 0.5*0.9 = 0.85 → weight = +0.7 (alpha boost)
    // model-b blended: 0.5*0.3 + 0.5*0.2 = 0.25 → weight = -0.5 (beta boost)
    let n = 100;
    let mut sum_a = 0.0;
    let mut sum_b = 0.0;
    for _ in 0..n {
        sum_a += module.thompson_sample("model-a", None).await;
        sum_b += module.thompson_sample("model-b", None).await;
    }
    let avg_a = sum_a / n as f64;
    let avg_b = sum_b / n as f64;

    assert!(
        avg_a > avg_b,
        "model-a ({avg_a:.3}) should sample higher than model-b ({avg_b:.3})"
    );
}
