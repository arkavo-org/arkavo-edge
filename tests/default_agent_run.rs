use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn test_default_command_is_agent_run() {
    // Create a temporary directory for the test
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Change to the temp directory
    std::env::set_current_dir(&temp_path).unwrap();

    // Run arkavo without arguments
    let output = Command::new(env!("CARGO_BIN_EXE_arkavo"))
        .current_dir(&temp_path)
        .env("RUST_LOG", "debug")
        .output()
        .expect("Failed to execute arkavo");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Check that AGENTS.md was created
    assert!(
        Path::new("AGENTS.md").exists(),
        "AGENTS.md should be auto-generated"
    );

    // Read the generated AGENTS.md
    let agents_content = fs::read_to_string("AGENTS.md").unwrap();

    // Check that it contains the expected format with directory name and git hash
    let dir_name = temp_path.file_name().unwrap().to_str().unwrap();
    assert!(
        agents_content.contains(&format!("## {}-", dir_name)),
        "Agent name should start with directory name"
    );
    assert!(
        agents_content.contains("purpose: AI agent for"),
        "Should contain purpose line"
    );
    assert!(
        agents_content.contains("model:"),
        "Should contain model line"
    );
    assert!(
        agents_content.contains("listen:"),
        "Should contain listen line"
    );

    // Check that it mentions auto-generation
    assert!(
        stdout.contains("Auto-generated AGENTS.md") || stderr.contains("Auto-generated AGENTS.md"),
        "Should mention auto-generation"
    );
}

#[test]
fn test_existing_agents_md_not_overwritten() {
    // Create a temporary directory for the test
    let temp_dir = TempDir::new().unwrap();
    let temp_path = temp_dir.path();

    // Create an existing AGENTS.md
    let existing_content = r#"# AGENTS.md

## existing-agent
purpose: This should not be overwritten
model: custom-model
listen: 0.0.0.0:9999
"#;

    fs::write(temp_path.join("AGENTS.md"), existing_content).unwrap();

    // Run arkavo without arguments
    let _output = Command::new(env!("CARGO_BIN_EXE_arkavo"))
        .current_dir(&temp_path)
        .env("RUST_LOG", "debug")
        .output()
        .expect("Failed to execute arkavo");

    // Read AGENTS.md and verify it wasn't overwritten
    let agents_content = fs::read_to_string(temp_path.join("AGENTS.md")).unwrap();
    assert_eq!(
        agents_content, existing_content,
        "Existing AGENTS.md should not be overwritten"
    );
}

#[test]
fn test_agent_name_format() {
    // Create a temporary directory with a known name
    let temp_dir = TempDir::new_in("/tmp").unwrap();
    let temp_path = temp_dir.path();

    // Change to the temp directory
    std::env::set_current_dir(&temp_path).unwrap();

    // Run arkavo without arguments
    let _output = Command::new(env!("CARGO_BIN_EXE_arkavo"))
        .current_dir(&temp_path)
        .env("RUST_LOG", "debug")
        .output()
        .expect("Failed to execute arkavo");

    // Read the generated AGENTS.md
    let agents_content = fs::read_to_string("AGENTS.md").unwrap();

    // Extract the agent name from the content
    let agent_line = agents_content
        .lines()
        .find(|line| line.starts_with("## "))
        .expect("Should have agent name line");

    let agent_name = &agent_line[3..];

    // Verify format: directory-hash (7 chars)
    let parts: Vec<&str> = agent_name.rsplitn(2, '-').collect();
    assert_eq!(
        parts.len(),
        2,
        "Agent name should have format: directory-hash"
    );

    let id_part = parts[0];
    assert_eq!(id_part.len(), 7, "ID part should be 7 characters");
    // UUID format uses hexadecimal digits and hyphens, we take first 7 chars which could include a hyphen
    assert!(
        id_part.chars().all(|c| c.is_ascii_hexdigit() || c == '-'),
        "ID part should be UUID format (hex digits or hyphen)"
    );
}
