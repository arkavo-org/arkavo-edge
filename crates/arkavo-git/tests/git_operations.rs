use arkavo_git::{DiffOptions, GitManager};
use std::fs;
use tempfile::TempDir;

#[test]
fn test_init_repo() {
    let temp_dir = TempDir::new().unwrap();
    let manager = GitManager::new();

    manager.init_repo(temp_dir.path()).unwrap();

    assert!(temp_dir.path().join(".git").exists());
}

#[test]
fn test_commit_and_status() {
    let temp_dir = TempDir::new().unwrap();
    let manager = GitManager::new();

    manager.init_repo(temp_dir.path()).unwrap();
    let repo = manager.open_repo(temp_dir.path()).unwrap();

    // Set up git config for tests
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Test User").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();

    fs::write(temp_dir.path().join("test.txt"), "Hello, Git!").unwrap();

    let status = manager.status(&repo).unwrap();
    println!(
        "Status: untracked={:?}, added={:?}, modified={:?}",
        status.untracked, status.added, status.modified
    );
    assert!(!status.untracked.is_empty() || !status.added.is_empty());
    assert!(
        status.untracked.iter().any(|f| f == "test.txt")
            || status.added.iter().any(|f| f == "test.txt")
    );

    manager.add_all(&repo).unwrap();
    let oid = manager.commit_changes(&repo, "Initial commit").unwrap();

    let status = manager.status(&repo).unwrap();
    assert!(status.untracked.is_empty());
    assert!(status.modified.is_empty());

    let head = repo.head().unwrap().target().unwrap();
    assert_eq!(head, oid);
}

#[test]
fn test_branch_operations() {
    let temp_dir = TempDir::new().unwrap();
    let manager = GitManager::new();

    manager.init_repo(temp_dir.path()).unwrap();
    let repo = manager.open_repo(temp_dir.path()).unwrap();

    // Set up git config for tests
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Test User").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();

    fs::write(temp_dir.path().join("test.txt"), "content").unwrap();
    manager.add_all(&repo).unwrap();
    manager.commit_changes(&repo, "Initial commit").unwrap();

    manager.create_branch(&repo, "feature/test").unwrap();

    let branches = manager.list_branches(&repo).unwrap();
    assert_eq!(branches.len(), 2);
    assert!(
        branches
            .iter()
            .any(|(name, _)| name == "main" || name == "master")
    );
    assert!(branches.iter().any(|(name, _)| name == "feature/test"));

    manager.checkout_branch(&repo, "feature/test").unwrap();

    let current = manager.get_current_branch(&repo).unwrap();
    assert_eq!(current, "feature/test");
}

#[test]
fn test_diff() {
    let temp_dir = TempDir::new().unwrap();
    let manager = GitManager::new();

    manager.init_repo(temp_dir.path()).unwrap();
    let repo = manager.open_repo(temp_dir.path()).unwrap();

    // Set up git config for tests
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Test User").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();

    fs::write(temp_dir.path().join("test.txt"), "line1\nline2\n").unwrap();
    manager.add_all(&repo).unwrap();
    manager.commit_changes(&repo, "Initial commit").unwrap();

    fs::write(temp_dir.path().join("test.txt"), "line1\nmodified\nline2\n").unwrap();

    let diff = manager.diff(&repo, &DiffOptions::default()).unwrap();
    assert!(diff.contains("line1"));
    assert!(diff.contains("modified"));
    assert!(diff.contains('+'));
}

#[test]
fn test_undo_commit() {
    let temp_dir = TempDir::new().unwrap();
    let manager = GitManager::new();

    manager.init_repo(temp_dir.path()).unwrap();
    let repo = manager.open_repo(temp_dir.path()).unwrap();

    // Set up git config for tests
    let mut config = repo.config().unwrap();
    config.set_str("user.name", "Test User").unwrap();
    config.set_str("user.email", "test@example.com").unwrap();

    fs::write(temp_dir.path().join("file1.txt"), "content1").unwrap();
    manager.add_all(&repo).unwrap();
    let oid1 = manager.commit_changes(&repo, "First commit").unwrap();

    fs::write(temp_dir.path().join("file2.txt"), "content2").unwrap();
    manager.add_all(&repo).unwrap();
    manager.commit_changes(&repo, "Second commit").unwrap();

    manager.undo_last_commit(&repo).unwrap();

    let head = repo.head().unwrap().target().unwrap();
    assert_eq!(head, oid1);
    assert!(!temp_dir.path().join("file2.txt").exists());
}
