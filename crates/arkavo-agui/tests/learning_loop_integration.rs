//! Integration tests for the AG-UI learning loop
//!
//! Tests the full gateway-side pipeline:
//! judge → extract_lesson → RoutingRecord quality tracking → LearningStatusUpdate event

use arkavo_agui::lesson_extractor::{LessonContext, extract_lesson};
use arkavo_agui::response_judge::{FailureMode, TaskJudgment, judge};
use arkavo_agui::types::{AgUiEvent, QualityTrend, RoutingRecord};

fn routing_record(agent: &str, category: &str, quality: Option<f64>) -> RoutingRecord {
    RoutingRecord {
        task_id: uuid::Uuid::new_v4().to_string(),
        selected_agent: agent.to_string(),
        was_exploration: false,
        outcome: quality.map(|q| {
            if q > 0.5 {
                "success".to_string()
            } else {
                "degraded".to_string()
            }
        }),
        quality_score: quality,
        quality_issues: vec![],
        category: Some(category.to_string()),
        timestamp: chrono::Utc::now().to_rfc3339(),
    }
}

/// Simulate the full judge → extract_lesson pipeline for a low-quality response
#[test]
fn test_judge_to_lesson_pipeline() {
    let judgment = judge(
        "Analyze the authentication module for vulnerabilities",
        "OK",
    );

    // "OK" is a generic non-answer — judge should score it low
    assert!(
        judgment.quality_score < 0.5,
        "Generic 'OK' should score low, got {}",
        judgment.quality_score
    );
    assert!(
        !judgment.failure_modes.is_empty(),
        "Should detect at least one failure mode"
    );

    // Extract a lesson from the low-quality judgment
    let ctx = LessonContext {
        agent_id: "code-analyzer".to_string(),
        task_category: "security_scan".to_string(),
        task_description: None,
        response_snippet: None,
    };
    let lesson = extract_lesson(&judgment, &ctx).expect("should produce a lesson for low quality");

    assert_eq!(lesson.agent_id, "code-analyzer");
    assert_eq!(lesson.category, "security_scan");
    assert!(lesson.confidence > 0.0);

    // Verify metadata carries the quality score for PolicyCache ingestion
    let meta = lesson
        .pattern
        .metadata
        .as_ref()
        .expect("metadata should exist");
    let q = meta["quality_score"].as_f64().unwrap();
    assert!((q - judgment.quality_score).abs() < f64::EPSILON);
}

/// Verify that high-quality responses do NOT produce lessons
#[test]
fn test_good_response_produces_no_lesson() {
    let good_result = r#"
## Security Analysis

### Findings
1. **SQL Injection** in `login.rs:42` — user input passed directly to query
2. **Missing CSRF tokens** on `/api/update` endpoint
3. **Weak password hashing** — using MD5 instead of bcrypt

### Recommendations
- Use parameterized queries for all database access
- Add CSRF middleware to all mutation endpoints
- Migrate to argon2 for password hashing
"#;

    let judgment = judge(
        "Analyze the authentication module for vulnerabilities",
        good_result,
    );
    assert!(
        judgment.quality_score > 0.5,
        "Detailed analysis should score high, got {}",
        judgment.quality_score
    );

    let ctx = LessonContext {
        agent_id: "code-analyzer".to_string(),
        task_category: "security_scan".to_string(),
        task_description: None,
        response_snippet: None,
    };
    assert!(
        extract_lesson(&judgment, &ctx).is_none(),
        "Good responses should not produce lessons"
    );
}

/// Multi-round simulation: verify lessons accumulate across task rounds
#[test]
fn test_multi_round_lesson_accumulation() {
    let ctx = LessonContext {
        agent_id: "agent-1".to_string(),
        task_category: "code_review".to_string(),
        task_description: None,
        response_snippet: None,
    };

    // Round 1: three poor responses
    let poor_judgments: Vec<TaskJudgment> = vec![
        TaskJudgment {
            quality_score: 0.0,
            issues: vec!["Empty".into()],
            failure_modes: vec![FailureMode::Empty],
        },
        TaskJudgment {
            quality_score: 0.2,
            issues: vec!["Generic".into()],
            failure_modes: vec![FailureMode::Generic],
        },
        TaskJudgment {
            quality_score: 0.1,
            issues: vec!["Too short".into()],
            failure_modes: vec![FailureMode::TooShort],
        },
    ];

    let lessons: Vec<_> = poor_judgments
        .iter()
        .filter_map(|j| extract_lesson(j, &ctx))
        .collect();

    assert_eq!(
        lessons.len(),
        3,
        "All three poor judgments should produce lessons"
    );

    // Each lesson should have distinct condition text (different failure modes)
    let conditions: std::collections::HashSet<_> = lessons
        .iter()
        .map(|l| l.pattern.condition.clone())
        .collect();
    assert_eq!(
        conditions.len(),
        3,
        "Each failure mode should produce a unique condition"
    );

    // All lessons should have confidence > 0
    for lesson in &lessons {
        assert!(lesson.confidence > 0.0);
        assert_eq!(lesson.category, "code_review");
    }
}

/// Verify LearningStatusUpdate event round-trips through JSON correctly
#[test]
fn test_learning_status_update_serde() {
    let event = AgUiEvent::LearningStatusUpdate {
        agents: vec![],
        routing_history: vec![
            routing_record("agent-1", "code", Some(0.3)),
            routing_record("agent-1", "code", Some(0.7)),
        ],
        quality_trends: vec![QualityTrend {
            agent_id: "agent-1".to_string(),
            category: "code".to_string(),
            scores: vec![0.3, 0.7],
        }],
        lesson_count: 2,
        lessons: vec![],
        optimal_configs: vec![],
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    let json = serde_json::to_string(&event).unwrap();

    // Verify all expected fields are present
    assert!(json.contains("\"qualityTrends\""), "camelCase rename");
    assert!(json.contains("\"lessonCount\":2"));
    assert!(json.contains("\"agentId\":\"agent-1\""));

    // Verify round-trip
    let deserialized: AgUiEvent = serde_json::from_str(&json).unwrap();
    match deserialized {
        AgUiEvent::LearningStatusUpdate {
            quality_trends,
            lesson_count,
            routing_history,
            ..
        } => {
            assert_eq!(lesson_count, 2);
            assert_eq!(quality_trends.len(), 1);
            assert_eq!(quality_trends[0].scores, vec![0.3, 0.7]);
            assert_eq!(routing_history.len(), 2);
        }
        other => panic!("Expected LearningStatusUpdate, got {other:?}"),
    }
}

/// Verify RoutingOutcome event carries quality data
#[test]
fn test_routing_outcome_quality_serde() {
    let event = AgUiEvent::RoutingOutcome {
        task_id: "task-1".to_string(),
        agent_id: "agent-1".to_string(),
        success: true,
        quality_score: 0.3,
        quality_issues: vec!["Generic response".into(), "Too short".into()],
        timestamp: chrono::Utc::now().to_rfc3339(),
    };

    let json = serde_json::to_string(&event).unwrap();
    assert!(json.contains("\"qualityScore\":0.3"));
    assert!(json.contains("\"qualityIssues\""));

    let deserialized: AgUiEvent = serde_json::from_str(&json).unwrap();
    match deserialized {
        AgUiEvent::RoutingOutcome {
            quality_score,
            quality_issues,
            ..
        } => {
            assert!((quality_score - 0.3).abs() < f64::EPSILON);
            assert_eq!(quality_issues.len(), 2);
        }
        other => panic!("Expected RoutingOutcome, got {other:?}"),
    }
}
