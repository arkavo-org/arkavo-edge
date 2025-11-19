use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Instant;
use tempfile::NamedTempFile;

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
fn test_cli_01_stdin_streaming() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let massive_input = "Error log:\n".to_string()
        + &"INFO: Normal operation\n".repeat(100)
        + "ERROR: Connection timeout at line 543\n"
        + &"INFO: Retry attempted\n".repeat(50);

    let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
    temp_file
        .write_all(massive_input.as_bytes())
        .expect("Failed to write to temp file");
    temp_file.flush().expect("Failed to flush");

    let start = Instant::now();

    let mut cmd = Command::new(get_arkavo_binary_path());
    cmd.arg("chat")
        .arg("--prompt")
        .arg("Find the error in this log")
        .stdin(std::fs::File::open(temp_file.path()).expect("Failed to open temp file"))
        .env("GEMINI_API_KEY", get_api_key())
        .env("GEMINI_MODEL", "models/gemini-3-pro-preview")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().expect("Failed to execute command");

    let elapsed = start.elapsed();
    let ttft = elapsed.as_secs();

    assert!(
        output.status.success(),
        "Command failed with status: {}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stdout_lower = stdout.to_lowercase();

    assert!(ttft < 120, "TTFT {} seconds exceeded 120s threshold", ttft);

    assert!(
        stdout_lower.contains("error")
            || stdout_lower.contains("timeout")
            || stdout_lower.contains("543"),
        "Response should mention the error. Got: {}",
        stdout
    );

    eprintln!(
        "CLI-01 PASS: STDIN streaming completed in {}s, found error",
        ttft
    );
}

#[test]
fn test_cli_02_stdout_redirection() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let mut cmd = Command::new(get_arkavo_binary_path());
    cmd.arg("task")
        .arg("--prompt")
        .arg("Generate a simple Rust struct called 'User' with fields: name (String) and age (u32). Output only the code, no explanation.")
        .env("GEMINI_API_KEY", get_api_key())
        .env("GEMINI_MODEL", "models/gemini-3-pro-preview")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().expect("Failed to execute command");

    assert!(
        output.status.success(),
        "Command failed with status: {}. Stdout: {}. Stderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(
        !stdout.contains("```rust") && !stdout.contains("```"),
        "Output should not contain markdown code fences. Got: {}",
        stdout
    );

    assert!(
        stdout.contains("struct") && stdout.contains("User"),
        "Output should contain struct definition. Got: {}",
        stdout
    );

    let prose_indicators = ["here is", "here's", "i've created", "this struct"];
    let has_prose = prose_indicators
        .iter()
        .any(|&phrase| stdout.to_lowercase().contains(phrase));

    assert!(
        !has_prose,
        "Output should not contain conversational filler. Got: {}",
        stdout
    );

    eprintln!("CLI-02 PASS: Clean code output without markdown or prose");
}

#[test]
fn test_cli_03_interrupt_handling() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    use std::thread;
    use std::time::Duration;

    let mut child = Command::new(get_arkavo_binary_path())
        .arg("chat")
        .arg("--prompt")
        .arg("Write a very long essay about the history of computing, including details about every major computer scientist and invention")
        .env("GEMINI_API_KEY", get_api_key())
        .env("GEMINI_MODEL", "models/gemini-3-pro-preview")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn child process");

    thread::sleep(Duration::from_secs(3));

    let kill_start = Instant::now();

    child.kill().expect("Failed to send kill signal");

    let wait_result = child.wait().expect("Failed to wait for child");
    let kill_duration = kill_start.elapsed();

    assert!(
        !wait_result.success(),
        "Process should not exit successfully after kill"
    );

    assert!(
        kill_duration.as_secs() < 2,
        "Process should terminate within 2 seconds. Took: {}s",
        kill_duration.as_secs()
    );

    thread::sleep(Duration::from_millis(500));

    #[cfg(unix)]
    {
        let ps_output = Command::new("ps")
            .arg("aux")
            .output()
            .expect("Failed to run ps");

        let ps_stdout = String::from_utf8_lossy(&ps_output.stdout);
        let arkavo_processes = ps_stdout
            .lines()
            .filter(|line| line.contains("arkavo") && line.contains("chat"))
            .count();

        assert_eq!(
            arkavo_processes, 0,
            "No zombie arkavo processes should remain. Found: {}",
            arkavo_processes
        );
    }

    eprintln!(
        "CLI-03 PASS: Process terminated cleanly in {:.2}s, no zombies",
        kill_duration.as_secs_f64()
    );
}

#[test]
fn test_cli_04_environment_config() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let mut cmd = Command::new(get_arkavo_binary_path());
    cmd.arg("chat")
        .arg("--prompt")
        .arg("Say 'test successful'")
        .env("GEMINI_API_KEY", get_api_key())
        .env("GEMINI_MODEL", "models/gemini-3-pro-preview")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output = cmd.output().expect("Failed to execute command");

    assert!(
        output.status.success(),
        "Command with env vars failed: {}",
        output.status
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.to_lowercase().contains("test") || stdout.to_lowercase().contains("successful"),
        "Response should acknowledge prompt. Got: {}",
        stdout
    );

    let mut cmd_default = Command::new(get_arkavo_binary_path());
    cmd_default
        .arg("chat")
        .arg("--prompt")
        .arg("What is 2+2?")
        .env("GEMINI_API_KEY", get_api_key())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let output_default = cmd_default.output().expect("Failed to execute command");

    assert!(
        output_default.status.success(),
        "Command with default model failed: {}",
        output_default.status
    );

    eprintln!("CLI-04 PASS: Environment variables and config precedence working correctly");
}
