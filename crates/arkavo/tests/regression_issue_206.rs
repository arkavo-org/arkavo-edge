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
