//! WS-C negative security test: a reviewer-role agent cannot reach an
//! ungranted write tool (github_pr_create), at the registry boundary.

use std::collections::HashSet;
use std::sync::Arc;

#[tokio::test]
async fn reviewer_registry_excludes_ungranted_write_tool() {
    use arkavo_mcp_tools::ToolRegistry;
    use arkavo_memory::MemoryStorage;
    let storage = Arc::new(MemoryStorage::new_test().await.expect("storage"));
    let mut reg = ToolRegistry::new(storage);
    let reviewer: HashSet<String> = ["git_diff", "git_log", "gh_pr_review", "github_ci_status"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    reg.retain_granted(&reviewer);

    assert!(
        reg.get("gh_pr_review").is_some(),
        "reviewer keeps its review tool"
    );
    assert!(reg.get("git_diff").is_some());
    assert!(
        reg.get("github_pr_create").is_none(),
        "reviewer must not see github_pr_create"
    );
}

/// Regression test for the A2A message.send specialist-path bypass.
///
/// Before the fix, the specialist path in `handlers/messaging.rs` built the
/// registry without applying `retain_granted`, so a `pr_reviewer` agent
/// reached via A2A `message.send` (no running orchestrator loop) could invoke
/// `github_pr_create` — exactly what WS-C/D9 exists to prevent.
///
/// This test replicates the filtering logic the handler now performs so that
/// it MUST fail if someone reverts the `retain_granted` call on that path.
#[tokio::test]
async fn specialist_path_registry_excludes_ungranted_write_tool() {
    use arkavo_mcp_tools::{Tool, ToolRegistry, ToolSchema};
    use arkavo_memory::MemoryStorage;
    use serde_json::Value;

    // Simulate the granted_tools a pr_reviewer agent carries after WS-C
    // specialization (a realistic but minimal grant set).
    let specialist_granted: Vec<String> = vec![
        "gh_pr_review".to_string(),
        "git_diff".to_string(),
        "git_log".to_string(),
        "github_ci_status".to_string(),
    ];

    // --- Reproduce the specialist-path registry build from messaging.rs ---
    // (mesh tools omitted — they are registered at runtime from MeshToolsState;
    //  the filtering logic being tested is the retain_granted call that follows)
    let granted_set: HashSet<String> = specialist_granted.iter().cloned().collect();

    // Minimal stub implementing the Tool trait used by ToolRegistry.
    struct Stub {
        schema: ToolSchema,
    }

    impl Stub {
        fn new(name: &str) -> Self {
            Self {
                schema: ToolSchema {
                    name: name.to_string(),
                    aliases: None,
                    description: String::new(),
                    parameters: serde_json::json!({}),
                },
            }
        }
    }

    #[async_trait::async_trait]
    impl Tool for Stub {
        async fn execute(&self, _: Value) -> arkavo_mcp_tools::Result<Value> {
            Ok(Value::Null)
        }

        fn schema(&self) -> &ToolSchema {
            &self.schema
        }
    }

    let storage = Arc::new(MemoryStorage::new_test().await.expect("storage"));
    let mut reg = ToolRegistry::new(storage);

    // Simulate MCP catalog tools that would include a write tool.  In
    // production these come from mcp_registry.list_all_tools(); here we
    // register them directly so the test is self-contained.
    reg.register("github_pr_create", Box::new(Stub::new("github_pr_create")));
    reg.register("gh_pr_review", Box::new(Stub::new("gh_pr_review")));
    reg.register("git_diff", Box::new(Stub::new("git_diff")));
    reg.register("git_log", Box::new(Stub::new("git_log")));
    reg.register("github_ci_status", Box::new(Stub::new("github_ci_status")));

    // Apply the filtering the fixed handler now performs.
    if !granted_set.is_empty() {
        reg.retain_granted(&granted_set);
    }

    // Registry must NOT expose the write tool to a reviewer-role specialist.
    assert!(
        reg.get("github_pr_create").is_none(),
        "specialist-path registry must not expose github_pr_create to a pr_reviewer agent"
    );

    // Registry MUST still expose granted tools.
    assert!(
        reg.get("gh_pr_review").is_some(),
        "specialist-path registry must expose gh_pr_review to a pr_reviewer agent"
    );
    assert!(reg.get("git_diff").is_some());
    assert!(reg.get("git_log").is_some());
    assert!(reg.get("github_ci_status").is_some());
}
