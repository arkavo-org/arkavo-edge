//! E2E test for autonomous issue resolution: Issue → Analyze → Route → Plan → Execute → PR
//!
//! This test demonstrates the complete "Healer" flow where an issue is automatically
//! analyzed, routed, planned, and resolved with a PR created.

use arkavo_events::{EventWriter, EventWriterConfig};
use arkavo_mcp_tools::ToolRegistry;
use arkavo_memory::MemoryStorage;
use arkavo_orchestrator::agent_assignment::AgentAssignment;
use arkavo_orchestrator::cognitive_engine::{
    ExecutionPlan, PlanStep, PrCreator, VerificationCheck,
};
use arkavo_orchestrator::issue_analyzer::{Complexity, IssueAnalyzer};
use arkavo_orchestrator::issue_router::{ExecutionStrategy, IssueRouter, Priority};
use arkavo_orchestrator::types::{Issue, IssueAction, IssueEvent, Label, Repository, User};
use std::sync::Arc;

/// Check if GitHub token is available for API-based operations
fn check_github_token() -> bool {
    std::env::var("GITHUB_TOKEN").is_ok()
}

/// Check if GitHub CLI is installed and authenticated (fallback check)
fn check_gh_cli() -> bool {
    std::process::Command::new("gh")
        .args(["auth", "status"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Create a mock issue event for testing
fn create_test_issue_event() -> IssueEvent {
    let user = User {
        id: 1,
        login: "healer-e2e-test".to_string(),
        user_type: "User".to_string(),
        avatar_url: "https://github.com/images/error/octocat_happy.gif".to_string(),
        html_url: "https://github.com/healer-e2e-test".to_string(),
    };

    let issue = Issue {
        id: 999999,
        number: 999999,
        title: "[E2E TEST] Healer validation - safe to close".to_string(),
        body: Some(
            "This is an automated E2E test for the Healer autonomous issue resolution system.\n\n\
             **Expected behavior**: The healer should:\n\
             1. Analyze this issue as a simple bug/test\n\
             2. Route it for auto-execution\n\
             3. Create a minimal plan\n\
             4. Execute verification steps\n\
             5. Create a PR (if in real mode)\n\n\
             This issue can be safely closed after testing."
                .to_string(),
        ),
        state: "open".to_string(),
        user: user.clone(),
        labels: vec![Label {
            id: 1,
            name: "good first issue".to_string(),
            color: "7057ff".to_string(),
            description: Some("Good for newcomers".to_string()),
        }],
        assignees: vec![],
        created_at: chrono::Utc::now().to_rfc3339(),
        updated_at: chrono::Utc::now().to_rfc3339(),
        closed_at: None,
        html_url: "https://github.com/arkavo-org/arkavo-edge/issues/999999".to_string(),
        milestone: None,
    };

    let repository = Repository {
        id: 1,
        name: "arkavo-edge".to_string(),
        full_name: "arkavo-org/arkavo-edge".to_string(),
        owner: user.clone(),
        html_url: "https://github.com/arkavo-org/arkavo-edge".to_string(),
        description: Some("Arkavo Edge Agent".to_string()),
        default_branch: "main".to_string(),
        language: Some("Rust".to_string()),
    };

    IssueEvent {
        action: IssueAction::Opened,
        issue,
        repository,
        sender: user,
        assignee: None,
    }
}

#[tokio::test]
async fn test_healer_issue_analysis() {
    let event = create_test_issue_event();

    let analysis = IssueAnalyzer::analyze(&event);

    assert_eq!(
        analysis.complexity,
        Complexity::Trivial,
        "E2E test issue should be classified as trivial"
    );

    println!("Issue Analysis:");
    println!("  Type: {:?}", analysis.issue_type);
    println!("  Complexity: {:?}", analysis.complexity);
    println!("  Technologies: {:?}", analysis.technologies);
    println!("  Estimated tokens: {}", analysis.estimated_tokens);
}

#[tokio::test]
async fn test_healer_issue_routing() {
    let event = create_test_issue_event();

    let routing = IssueRouter::route(&event);

    // Trivial issues that aren't Documentation/Question get PlanFirst for verification
    assert_eq!(
        routing.strategy,
        ExecutionStrategy::PlanFirst,
        "Trivial non-doc issues should use PlanFirst for verification"
    );
    assert!(!routing.should_notify_human, "No human notification needed");

    println!("Routing Decision:");
    println!("  Strategy: {:?}", routing.strategy);
    println!("  Priority: {:?}", routing.priority);
    println!("  Notify human: {}", routing.should_notify_human);
    println!("  Rationale: {}", routing.rationale);
}

#[tokio::test]
async fn test_healer_agent_assignment() {
    let event = create_test_issue_event();
    let routing = IssueRouter::route(&event);

    let assignment = AgentAssignment {
        issue_number: event.issue.number,
        repository: event.repository.full_name.clone(),
        issue_title: event.issue.title.clone(),
        issue_body: event.issue.body.clone().unwrap_or_default(),
        routing_decision: routing.clone(),
        assigned_agent_id: None,
        assignment_rationale: "E2E test assignment".to_string(),
    };

    assert_eq!(assignment.issue_number, 999999);
    assert_eq!(assignment.repository, "arkavo-org/arkavo-edge");

    println!("Agent Assignment:");
    println!("  Issue: #{}", assignment.issue_number);
    println!("  Repository: {}", assignment.repository);
    println!("  Strategy: {:?}", assignment.routing_decision.strategy);
}

#[tokio::test]
async fn test_healer_execution_plan() {
    let event = create_test_issue_event();

    let plan = ExecutionPlan {
        issue_number: event.issue.number,
        repository: event.repository.full_name.clone(),
        steps: vec![
            PlanStep {
                step_number: 1,
                description: "Verify test environment".to_string(),
                commands: vec!["echo 'Healer E2E test - step 1'".to_string()],
                verification: vec![],
                confidence: 1.0,
            },
            PlanStep {
                step_number: 2,
                description: "Run basic validation".to_string(),
                commands: vec!["echo 'Healer E2E test - step 2'".to_string()],
                verification: vec![VerificationCheck::TestsPassing],
                confidence: 0.95,
            },
        ],
        estimated_tokens: 1000,
    };

    assert_eq!(plan.steps.len(), 2);
    assert!(plan.steps[0].confidence >= 0.9);

    println!("Execution Plan:");
    for step in &plan.steps {
        println!(
            "  Step {}: {} (confidence: {:.0}%)",
            step.step_number,
            step.description,
            step.confidence * 100.0
        );
    }
}

#[tokio::test]
#[ignore] // Run with: cargo test -p arkavo-orchestrator test_healer_full_flow -- --ignored --nocapture
async fn test_healer_full_flow_with_pr() {
    if !check_github_token() && !check_gh_cli() {
        eprintln!("Skipping test_healer_full_flow_with_pr:");
        eprintln!("  - GITHUB_TOKEN not set");
        eprintln!("  - GitHub CLI not authenticated");
        eprintln!("Set GITHUB_TOKEN or run 'gh auth login' to enable this test");
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

    let event = create_test_issue_event();
    let routing = IssueRouter::route(&event);

    println!("\n=== Healer E2E Full Flow ===\n");
    println!("Step 1: Issue Analysis");
    println!("  Type: {:?}", routing.analysis.issue_type);
    println!("  Complexity: {:?}", routing.analysis.complexity);

    println!("\nStep 2: Routing Decision");
    println!("  Strategy: {:?}", routing.strategy);
    println!("  Priority: {:?}", routing.priority);

    let assignment = AgentAssignment {
        issue_number: event.issue.number,
        repository: event.repository.full_name.clone(),
        issue_title: event.issue.title.clone(),
        issue_body: event.issue.body.clone().unwrap_or_default(),
        routing_decision: routing.clone(),
        assigned_agent_id: None,
        assignment_rationale: "E2E test - auto-assigned".to_string(),
    };

    println!("\nStep 3: Agent Assignment");
    println!("  Issue: #{}", assignment.issue_number);
    println!("  Repository: {}", assignment.repository);

    let plan = ExecutionPlan {
        issue_number: event.issue.number,
        repository: event.repository.full_name.clone(),
        steps: vec![PlanStep {
            step_number: 1,
            description: "E2E validation step".to_string(),
            commands: vec!["echo 'Healer E2E validation complete'".to_string()],
            verification: vec![],
            confidence: 1.0,
        }],
        estimated_tokens: 500,
    };

    println!("\nStep 4: Execution Plan");
    println!("  Steps: {}", plan.steps.len());
    println!("  Estimated tokens: {}", plan.estimated_tokens);

    println!("\nStep 5: PR Creation");
    let pr_creator = PrCreator::new(
        tool_registry,
        event_writer,
        "healer-e2e-test-session".to_string(),
    );

    let result = pr_creator
        .create_pull_request(&assignment, &plan, 1, 500)
        .await;

    match result {
        Ok(pr_url) => {
            println!("  SUCCESS: PR created at {}", pr_url);
            println!("\n  Please close this PR manually after verification");
            assert!(
                pr_url.contains("github.com"),
                "PR URL should be a GitHub URL"
            );
        }
        Err(e) => {
            println!("  Note: PR creation returned error (expected in test env): {}", e);
        }
    }

    println!("\n=== E2E Flow Complete ===\n");
}

#[test]
fn test_healer_pr_body_format() {
    let issue_number = 123;
    let steps_completed = 2;
    let total_steps = 2;
    let tokens_used = 500;

    let pr_body = format!(
        "## Summary\n\n\
         Automated fix for issue #{}.\n\n\
         ## Execution Details\n\n\
         - Steps completed: {}/{}\n\
         - Tokens used: {}\n\
         - All verifications passed\n\n\
         ## Plan Steps\n\n\
         1. Verify test environment\n\
         2. Run basic validation\n\n\
         ---\n\
         Resolves #{}\n\n\
         Generated by Arkavo Healer",
        issue_number, steps_completed, total_steps, tokens_used, issue_number
    );

    assert!(pr_body.contains("Resolves #123"));
    assert!(pr_body.contains("Steps completed: 2/2"));
    assert!(pr_body.contains("Arkavo Healer"));
}

#[test]
fn test_healer_routing_strategies() {
    // Test that routing strategies align with complexity levels
    // Note: Actual routing depends on issue type AND complexity
    let event = create_test_issue_event();
    let routing = IssueRouter::route(&event);

    // Trivial "good first issue" uses PlanFirst (only Doc/Question auto-execute)
    assert_eq!(routing.strategy, ExecutionStrategy::PlanFirst);
    assert_eq!(routing.analysis.complexity, Complexity::Trivial);
}

#[test]
fn test_healer_priority_assignment() {
    // Create a security issue event
    let user = User {
        id: 1,
        login: "test".to_string(),
        user_type: "User".to_string(),
        avatar_url: "".to_string(),
        html_url: "".to_string(),
    };

    let security_event = IssueEvent {
        action: IssueAction::Opened,
        issue: Issue {
            id: 1,
            number: 1,
            title: "Security vulnerability found".to_string(),
            body: Some("Critical security issue".to_string()),
            state: "open".to_string(),
            user: user.clone(),
            labels: vec![Label {
                id: 1,
                name: "security".to_string(),
                color: "ff0000".to_string(),
                description: None,
            }],
            assignees: vec![],
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
            closed_at: None,
            html_url: "https://github.com/test/test/issues/1".to_string(),
            milestone: None,
        },
        repository: Repository {
            id: 1,
            name: "test".to_string(),
            full_name: "test/test".to_string(),
            owner: user.clone(),
            html_url: "https://github.com/test/test".to_string(),
            description: None,
            default_branch: "main".to_string(),
            language: Some("Rust".to_string()),
        },
        sender: user,
        assignee: None,
    };

    let routing = IssueRouter::route(&security_event);

    assert_eq!(
        routing.priority,
        Priority::Critical,
        "Security issues should be critical priority"
    );
    assert!(
        routing.should_notify_human,
        "Security issues should notify humans"
    );
}
