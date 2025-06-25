use arkavo_git::{GitManager, Repository};
use arkavo_test::mcp::git_tools::{
    GitBranchKit, GitCommitKit, GitDiffKit, GitLogKit, GitStatusKit,
};
use arkavo_test::mcp::server::Tool;
use serde_json::json;
use std::fs;
use tempfile::TempDir;

async fn setup_test_repo() -> (TempDir, Repository) {
    let temp_dir = TempDir::new().unwrap();
    let manager = GitManager::new();

    // Initialize repo
    manager.init_repo(temp_dir.path()).unwrap();
    let repo = manager.open_repo(temp_dir.path()).unwrap();

    // Set initial branch to main
    repo.set_head("refs/heads/main").ok();

    // Create initial commit
    fs::write(temp_dir.path().join("README.md"), "# Test Repository\n").unwrap();
    manager.add_all(&repo).unwrap();
    let oid = manager.commit_changes(&repo, "Initial commit").unwrap();

    // Create main branch pointing to the commit
    {
        let commit = repo.find_commit(oid).unwrap();
        repo.branch("main", &commit, true).ok();
    }

    (temp_dir, repo)
}

#[tokio::test]
async fn test_git_status_tool() {
    let (temp_dir, _repo) = setup_test_repo().await;

    // Create a new file
    fs::write(temp_dir.path().join("new_file.txt"), "test content").unwrap();

    let tool = GitStatusKit::new();
    let params = json!({
        "path": temp_dir.path().to_str().unwrap()
    });

    let result = tool.execute(params).await.unwrap();

    assert_eq!(result["branch"], "main");
    let untracked = result["untracked"].as_array().unwrap();
    let added = result["added"].as_array().unwrap();

    // The file might be either untracked or added depending on git state
    assert!(untracked.len() > 0 || added.len() > 0);
    if untracked.len() > 0 {
        assert!(untracked[0].as_str().unwrap().contains("new_file.txt"));
    } else {
        assert!(added[0].as_str().unwrap().contains("new_file.txt"));
    }
}

#[tokio::test]
async fn test_git_diff_tool() {
    let (temp_dir, _repo) = setup_test_repo().await;

    // Modify the README
    fs::write(
        temp_dir.path().join("README.md"),
        "# Test Repository\n\nModified content\n",
    )
    .unwrap();

    let tool = GitDiffKit::new();
    let params = json!({
        "path": temp_dir.path().to_str().unwrap()
    });

    let result = tool.execute(params).await.unwrap();

    let diff = result["diff"].as_str().unwrap();
    assert!(diff.contains("README.md"));
    assert!(diff.contains("Modified content") || diff.contains("+Modified content"));
}

#[tokio::test]
async fn test_git_commit_tool() {
    let (temp_dir, repo) = setup_test_repo().await;
    let _manager = GitManager::new();

    // Create a new file
    fs::write(temp_dir.path().join("test.txt"), "test content").unwrap();

    let tool = GitCommitKit::new();
    let params = json!({
        "path": temp_dir.path().to_str().unwrap(),
        "message": "Add test file"
    });

    let result = tool.execute(params).await.unwrap();

    assert_eq!(result["success"], true);
    assert_eq!(result["message"], "Add test file");

    // Verify the commit was created
    let log = repo.reflog("HEAD").unwrap();
    let entry = log.get(0).unwrap();
    assert!(entry.message().unwrap().contains("commit: Add test file"));
}

#[tokio::test]
async fn test_git_branch_tool() {
    let (temp_dir, _repo) = setup_test_repo().await;

    let tool = GitBranchKit::new();

    // Test listing branches
    let params = json!({
        "path": temp_dir.path().to_str().unwrap(),
        "action": "list"
    });

    let result = tool.execute(params).await.unwrap();
    let branches = result["branches"].as_array().unwrap();
    assert_eq!(branches.len(), 1);
    assert_eq!(branches[0]["name"], "main");
    assert_eq!(branches[0]["current"], true);

    // Test creating a branch
    let params = json!({
        "path": temp_dir.path().to_str().unwrap(),
        "action": "create",
        "name": "feature/test"
    });

    let result = tool.execute(params).await.unwrap();
    assert_eq!(result["success"], true);
    assert_eq!(result["created"], "feature/test");

    // Test switching branches
    let params = json!({
        "path": temp_dir.path().to_str().unwrap(),
        "action": "switch",
        "name": "feature/test"
    });

    let result = tool.execute(params).await.unwrap();
    assert_eq!(result["success"], true);
    assert_eq!(result["switched_to"], "feature/test");
}

#[tokio::test]
async fn test_git_log_tool() {
    let (temp_dir, repo) = setup_test_repo().await;
    let manager = GitManager::new();

    // Create additional commits
    for i in 1..=3 {
        fs::write(
            temp_dir.path().join(format!("file{}.txt", i)),
            format!("content {}", i),
        )
        .unwrap();
        manager.add_all(&repo).unwrap();
        manager
            .commit_changes(&repo, &format!("Commit {}", i))
            .unwrap();
    }

    let tool = GitLogKit::new();
    let params = json!({
        "path": temp_dir.path().to_str().unwrap(),
        "limit": 2
    });

    let result = tool.execute(params).await.unwrap();
    let commits = result["commits"].as_array().unwrap();

    assert_eq!(commits.len(), 2);
    assert_eq!(commits[0]["message"], "Commit 3");
    assert_eq!(commits[1]["message"], "Commit 2");
}

#[tokio::test]
async fn test_path_sanitization() {
    let tool = GitStatusKit::new();

    // Test path traversal attempt
    let params = json!({
        "path": "../../../etc"
    });

    let result = tool.execute(params).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Path validation failed"));
}

#[tokio::test]
async fn test_git_commit_missing_message() {
    let (temp_dir, _repo) = setup_test_repo().await;

    let tool = GitCommitKit::new();
    let params = json!({
        "path": temp_dir.path().to_str().unwrap()
    });

    let result = tool.execute(params).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Commit message is required"));
}

#[tokio::test]
async fn test_git_branch_invalid_action() {
    let (temp_dir, _repo) = setup_test_repo().await;

    let tool = GitBranchKit::new();
    let params = json!({
        "path": temp_dir.path().to_str().unwrap(),
        "action": "invalid"
    });

    let result = tool.execute(params).await;
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("Invalid action"));
}
