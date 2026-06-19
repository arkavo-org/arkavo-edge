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
