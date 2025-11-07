#![allow(clippy::disallowed_methods)]
#![allow(clippy::zombie_processes)]

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

#[cfg(all(target_os = "macos", feature = "mcp-tools"))]
#[test]
fn test_mcp_server_starts_without_panic() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_arkavo"))
        .arg("serve")
        .env("ARKAVO_NO_TERMINAL_RELAUNCH", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start MCP server");

    // Send an initialize request
    let stdin = child.stdin.as_mut().expect("Failed to get stdin");
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
        .expect("Failed to write to stdin");
    stdin.flush().expect("Failed to flush stdin");

    // Wait for a short time to see if it panics
    std::thread::sleep(Duration::from_secs(2));

    // Check if the process is still running
    match child.try_wait() {
        Ok(None) => {
            // Process is still running, good!
            // Kill it cleanly
            child.kill().expect("Failed to kill MCP server");
        }
        Ok(Some(status)) => {
            // Process exited, check if it was successful
            if !status.success() {
                // Read stderr to get the panic message if any
                let output = child.wait_with_output().expect("Failed to get output");
                let stderr = String::from_utf8_lossy(&output.stderr);

                // Check for the specific panic we're looking for
                if stderr.contains("Cannot start a runtime from within a runtime") {
                    panic!("MCP server panicked with tokio runtime error:\n{stderr}");
                } else if stderr.contains("panicked") {
                    panic!("MCP server panicked:\n{stderr}");
                } else {
                    panic!("MCP server exited with error:\n{stderr}");
                }
            }
        }
        Err(e) => {
            panic!("Failed to check MCP server status: {e}");
        }
    }
}

#[cfg(all(target_os = "macos", feature = "mcp-tools"))]
#[test]
fn test_mcp_server_responds_to_initialize() {
    use std::io::{BufRead, BufReader};

    let mut child = Command::new(env!("CARGO_BIN_EXE_arkavo"))
        .arg("serve")
        .env("ARKAVO_NO_TERMINAL_RELAUNCH", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to start MCP server");

    // Send an initialize request
    let stdin = child.stdin.as_mut().expect("Failed to get stdin");
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n")
        .expect("Failed to write to stdin");
    stdin.flush().expect("Failed to flush stdin");

    // Read the response
    let stdout = child.stdout.take().expect("Failed to get stdout");
    let reader = BufReader::new(stdout);
    let mut lines = reader.lines();

    // Wait for response with timeout
    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(5);

    let response = loop {
        if start.elapsed() > timeout {
            child.kill().expect("Failed to kill MCP server");
            panic!("Timeout waiting for MCP server response");
        }

        if let Some(Ok(line)) = lines.next() {
            break line;
        }

        std::thread::sleep(Duration::from_millis(100));
    };

    // Parse and validate the response
    let json: serde_json::Value =
        serde_json::from_str(&response).expect("Failed to parse MCP server response as JSON");

    // Check that it's a valid JSON-RPC response
    assert_eq!(json["jsonrpc"], "2.0", "Invalid JSON-RPC version");
    assert_eq!(json["id"], 1, "Response ID doesn't match request");
    assert!(
        json["result"].is_object(),
        "Response should have a result object"
    );
    assert!(
        json["result"]["protocolVersion"].is_string(),
        "Response should have protocolVersion"
    );
    assert!(
        json["result"]["serverInfo"].is_object(),
        "Response should have serverInfo"
    );

    // Clean up
    child.kill().expect("Failed to kill MCP server");
}
