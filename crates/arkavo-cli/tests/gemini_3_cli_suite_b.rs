use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use tempfile::TempDir;

fn get_api_key() -> String {
    std::env::var("GEMINI_API_KEY").unwrap_or_else(|_| "test_key".to_string())
}

fn should_skip_integration_tests() -> bool {
    std::env::var("GEMINI_API_KEY").is_err() || get_api_key() == "test_key"
}

fn get_arkavo_binary_path() -> std::path::PathBuf {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let mut path = std::path::PathBuf::from(manifest_dir);
    path.pop();
    path.pop();
    path.push("target");
    path.push("debug");
    path.push("arkavo");
    path
}

#[test]
fn test_cli_05_filesystem_access() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let test_folder = temp_dir.path().join("test_gen");

    let prompt = format!(
        "Create a folder at '{}' and write a file 'hello.txt' inside it with the content 'Hello from Gemini 3!'",
        test_folder.display()
    );

    let mut cmd = Command::new(get_arkavo_binary_path());
    cmd.arg("task")
        .arg(&prompt)
        .current_dir(temp_dir.path())
        .env("GEMINI_API_KEY", get_api_key())
        .env("GEMINI_MODEL", "models/gemini-3-pro-preview")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().expect("Failed to execute command");

    assert!(
        output.status.success(),
        "Command failed with status: {}. Stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        test_folder.exists(),
        "Folder should exist at {}",
        test_folder.display()
    );

    let hello_file = test_folder.join("hello.txt");
    assert!(
        hello_file.exists(),
        "File should exist at {}",
        hello_file.display()
    );

    let content = fs::read_to_string(&hello_file).expect("Failed to read file");

    assert!(
        content.to_lowercase().contains("hello"),
        "File should contain 'hello'. Got: {}",
        content
    );

    eprintln!("CLI-05a PASS: Filesystem operations successful, folder and file created");

    let restricted_path = PathBuf::from("/root/test_restricted.txt");
    let prompt_restricted = format!(
        "Write a file at '{}' with content 'test'",
        restricted_path.display()
    );

    let mut cmd_restricted = Command::new(get_arkavo_binary_path());
    cmd_restricted
        .arg("task")
        .arg(&prompt_restricted)
        .env("GEMINI_API_KEY", get_api_key())
        .env("GEMINI_MODEL", "models/gemini-3-pro-preview")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output_restricted = cmd_restricted.output().expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output_restricted.stdout);
    let stderr = String::from_utf8_lossy(&output_restricted.stderr);
    let combined = format!("{}{}", stdout, stderr).to_lowercase();

    let has_permission_error = combined.contains("permission")
        || combined.contains("denied")
        || combined.contains("cannot")
        || combined.contains("error")
        || combined.contains("failed");

    assert!(
        !restricted_path.exists() || has_permission_error,
        "Should handle permission errors gracefully or fail to create file"
    );

    eprintln!("CLI-05b PASS: Permission errors handled gracefully");
}

#[test]
fn test_cli_06_git_integration() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    Command::new("git")
        .arg("init")
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to init git repo");

    Command::new("git")
        .arg("config")
        .arg("user.email")
        .arg("test@example.com")
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to set git email");

    Command::new("git")
        .arg("config")
        .arg("user.name")
        .arg("Test User")
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to set git name");

    let test_file = temp_dir.path().join("test.txt");
    fs::write(&test_file, "Initial content\n").expect("Failed to write test file");

    Command::new("git")
        .arg("add")
        .arg("test.txt")
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to git add");

    Command::new("git")
        .arg("commit")
        .arg("-m")
        .arg("Initial commit")
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to git commit");

    fs::write(&test_file, "Initial content\nNew line added\n").expect("Failed to update test file");

    Command::new("git")
        .arg("add")
        .arg("test.txt")
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to git add updated file");

    Command::new("git")
        .arg("commit")
        .arg("-m")
        .arg("Add new line to test.txt")
        .current_dir(temp_dir.path())
        .output()
        .expect("Failed to git commit update");

    let mut cmd = Command::new(get_arkavo_binary_path());
    cmd.arg("task")
        .arg("--prompt")
        .arg("Use git to show the diff of the last commit and summarize what changed")
        .current_dir(temp_dir.path())
        .env("GEMINI_API_KEY", get_api_key())
        .env("GEMINI_MODEL", "models/gemini-3-pro-preview")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().expect("Failed to execute command");

    assert!(
        output.status.success(),
        "Command failed with status: {}. Stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout_lower = stdout.to_lowercase();

    let has_git_mention = stdout_lower.contains("git")
        || stdout_lower.contains("diff")
        || stdout_lower.contains("commit");

    let has_change_mention = stdout_lower.contains("new line")
        || stdout_lower.contains("added")
        || stdout_lower.contains("test.txt")
        || stdout_lower.contains("line");

    assert!(
        has_git_mention || has_change_mention,
        "Response should reference git diff or changes. Got: {}",
        stdout
    );

    eprintln!("CLI-06 PASS: Git integration working, diff summarized");
}
