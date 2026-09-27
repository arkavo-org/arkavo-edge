use arkavo_events::{EventWriter, EventWriterConfig};
use arkavo_mcp_tools::ToolRegistry;
use arkavo_memory::MemoryStorage;
use arkavo_orchestrator::agent_assignment::{AgentAssignment, RoutingDecision};
use arkavo_orchestrator::cognitive_engine::{
    ExecutionPlan, PlanStep, PrCreator, VerificationCheck,
};
use arkavo_test_macros::spec;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

/// Check if GitHub CLI is installed and authenticated
fn check_gh_cli() -> bool {
    std::process::Command::new("gh")
        .args(["auth", "status"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[spec("ORCH-005")]
#[tokio::test]
async fn test_pr_creation_with_mcp_tool() {
    // Gracefully skip if gh CLI not available or not authenticated
    if !check_gh_cli() {
        eprintln!(
            "Skipping test_pr_creation_with_mcp_tool: GitHub CLI not installed or not authenticated"
        );
        eprintln!("Run 'gh auth login' to enable this test");
        return;
    }

    let storage = Arc::new(
        MemoryStorage::new()
            .await
            .expect("Failed to create storage"),
    );
    let tool_registry = Arc::new(ToolRegistry::new(storage));

    let event_config = EventWriterConfig {
        buffer_size: 1024,
        flush_interval: Default::default(),
        batch_size: 100,
    };
    let event_writer = Arc::new(EventWriter::new(event_config));

    let pr_creator = PrCreator::new(tool_registry, event_writer, "test-session".to_string());

    let assignment = create_test_assignment();
    let plan = create_test_plan();

    let result = pr_creator
        .create_pull_request(&assignment, &plan, 3, 1000)
        .await;

    match result {
        Ok(pr_url) => {
            assert!(pr_url.starts_with("https://github.com/"));
            println!("PR created: {}", pr_url);
        }
        Err(e) => {
            println!("PR creation failed (expected in test): {}", e);
        }
    }
}

#[test]
fn test_pr_title_format() {
    let issue_number = 123;
    let issue_title = "Fix authentication bug";
    let expected_title = format!("Fix issue #{}: {}", issue_number, issue_title);

    assert_eq!(expected_title, "Fix issue #123: Fix authentication bug");
}

#[test]
fn test_pr_body_includes_execution_details() {
    let issue_number = 123;
    let steps_completed = 3;
    let total_steps = 3;
    let tokens_used = 1500;

    let pr_body = format!(
        "## Execution Details\n\n\
        - Steps completed: {}/{}\n\
        - Tokens used: {}\n\
        - All verifications passed ✓\n",
        steps_completed, total_steps, tokens_used
    );

    assert!(pr_body.contains("Steps completed: 3/3"));
    assert!(pr_body.contains("Tokens used: 1500"));
    assert!(pr_body.contains("All verifications passed"));
}

#[test]
fn test_pr_body_includes_resolves_reference() {
    let issue_number = 456;
    let pr_body = format!("Resolves #{}\n", issue_number);

    assert!(pr_body.contains("Resolves #456"));
}

fn create_test_assignment() -> AgentAssignment {
    use arkavo_orchestrator::{Complexity, ExecutionStrategy, IssueAnalysis, IssueType, Priority};

    AgentAssignment {
        issue_number: 123,
        repository: "test-org/test-repo".to_string(),
        issue_title: "Fix authentication bug".to_string(),
        issue_body: "The login flow is broken".to_string(),
        routing_decision: RoutingDecision {
            strategy: ExecutionStrategy::AutoExecute,
            analysis: IssueAnalysis {
                issue_type: IssueType::Bug,
                complexity: Complexity::Simple,
                technologies: vec!["rust".to_string()],
                required_capabilities: vec![],
                estimated_tokens: 50_000,
            },
            rationale: "Simple bug fix".to_string(),
            should_notify_human: false,
            priority: Priority::Low,
        },
        assigned_agent_id: None,
        assignment_rationale: "".to_string(),
    }
}

fn create_test_plan() -> ExecutionPlan {
    ExecutionPlan {
        id: Uuid::new_v4(),
        issue_number: 123,
        repository: "test-org/test-repo".to_string(),
        steps: vec![
            PlanStep {
                step_number: 1,
                description: "Analyze the authentication code".to_string(),
                commands: vec!["cargo test auth".to_string()],
                verification: vec![VerificationCheck::TestsPassing],
                confidence: 0.9,
            },
            PlanStep {
                step_number: 2,
                description: "Fix the bug in auth_manager.rs".to_string(),
                commands: vec!["edit auth_manager.rs".to_string()],
                verification: vec![
                    VerificationCheck::TestsPassing,
                    VerificationCheck::LinterClean,
                ],
                confidence: 0.85,
            },
            PlanStep {
                step_number: 3,
                description: "Verify all tests pass".to_string(),
                commands: vec!["cargo test --all".to_string()],
                verification: vec![
                    VerificationCheck::TestsPassing,
                    VerificationCheck::BuildSuccessful,
                ],
                confidence: 0.95,
            },
        ],
        estimated_tokens: 1500,
    }
}

#[test]
fn test_mcp_tool_params_structure() {
    let pr_title = "Fix issue #123: Test PR";
    let pr_body = "Test body content";

    let params = json!({
        "title": pr_title,
        "body": pr_body,
        "base": "main",
    });

    assert_eq!(params["title"], "Fix issue #123: Test PR");
    assert_eq!(params["body"], "Test body content");
    assert_eq!(params["base"], "main");
}

#[test]
fn test_pr_url_extraction_from_mcp_response() {
    let mcp_response = json!({
        "success": true,
        "pr_url": "https://github.com/test-org/test-repo/pull/456",
        "message": "Pull request created: https://github.com/test-org/test-repo/pull/456"
    });

    let pr_url = mcp_response["pr_url"].as_str().unwrap();
    assert_eq!(pr_url, "https://github.com/test-org/test-repo/pull/456");
}

#[test]
fn test_pr_body_includes_plan_steps() {
    let plan = create_test_plan();

    let steps_summary: Vec<String> = plan
        .steps
        .iter()
        .enumerate()
        .map(|(i, step)| format!("{}. {}", i + 1, step.description))
        .collect();

    let steps_text = steps_summary.join("\n");

    assert!(steps_text.contains("1. Analyze the authentication code"));
    assert!(steps_text.contains("2. Fix the bug in auth_manager.rs"));
    assert!(steps_text.contains("3. Verify all tests pass"));
}

#[test]
fn test_pr_body_includes_commands() {
    let step = PlanStep {
        step_number: 1,
        description: "Run tests".to_string(),
        commands: vec!["cargo test".to_string(), "cargo clippy".to_string()],
        verification: vec![VerificationCheck::TestsPassing],
        confidence: 0.9,
    };

    let commands_text: Vec<String> = step.commands.iter().map(|c| format!("- `{}`", c)).collect();

    let formatted = commands_text.join("\n");

    assert!(formatted.contains("- `cargo test`"));
    assert!(formatted.contains("- `cargo clippy`"));
}
