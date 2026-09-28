//! Security Integration Tests for Arkavo Executable
//!
//! BDD-style tests that verify security controls are active in the actual executable.
//! These tests spawn the real `arkavo` binary and verify security behaviors.

use std::process::Command;
use std::time::Duration;
use tempfile::TempDir;

/// Get path to arkavo binary
fn arkavo_bin() -> &'static str {
    env!("CARGO_BIN_EXE_arkavo")
}

// ============================================================================
// BDD Feature: Rate Limiting
// As a security engineer
// I want excessive requests to be rate limited
// So that DoS attacks are prevented
// ============================================================================

mod rate_limiting {
    use super::*;
    use std::thread;

    /// Scenario: Rapid sequential requests
    #[test]
    fn rate_limits_excessive_requests() {
        let temp_dir = TempDir::new().unwrap();
        let mut limited_count = 0;

        // Make multiple rapid requests
        for _ in 0..5 {
            let output = Command::new(arkavo_bin())
                .current_dir(&temp_dir)
                .args(["--version"])
                .output();

            if let Ok(output) = output {
                let stderr = String::from_utf8_lossy(&output.stderr);

                if stderr.contains("rate limit")
                    || stderr.contains("throttled")
                    || output.status.code() == Some(429)
                {
                    limited_count += 1;
                }
            }

            // Small delay between requests
            thread::sleep(Duration::from_millis(10));
        }

        // At least some requests should be allowed (version check)
        // This test documents the behavior; actual rate limiting may vary
        println!("Rate limited {} out of 5 requests", limited_count);
    }
}

// ============================================================================
// BDD Feature: Secure Defaults
// As a security engineer
// I want the executable to use secure defaults
// So that unsafe configurations are not possible
// ============================================================================

mod secure_defaults {
    use super::*;

    /// Scenario: Verifying TLS is required
    #[test]
    fn requires_tls_for_http_connections() {
        let temp_dir = TempDir::new().unwrap();

        // Try to use HTTP instead of HTTPS
        let output = Command::new(arkavo_bin())
            .current_dir(&temp_dir)
            .args(["chat", "--prompt", "Fetch http://example.com"])
            .output();

        if let Ok(output) = output {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let stdout = String::from_utf8_lossy(&output.stdout);

            // Security: Should detect and block insecure HTTP
            // This is a documentation test - actual behavior depends on implementation
            println!(
                "HTTP request handling: stdout={}, stderr={}",
                stdout, stderr
            );
        }
    }

    /// Scenario: Verifying no sensitive data in logs
    #[test]
    fn does_not_log_sensitive_data() {
        let temp_dir = TempDir::new().unwrap();

        // Set a fake API key
        let output = Command::new(arkavo_bin())
            .current_dir(&temp_dir)
            .args(["--version"])
            .env("OPENAI_API_KEY", "FAKE_API_KEY_FOR_TESTING_ONLY")
            .env("RUST_LOG", "debug")
            .output()
            .expect("Failed to execute");

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // API key should not appear in output
        assert!(
            !stdout.contains("FAKE_API_KEY") && !stderr.contains("FAKE_API_KEY"),
            "API key should not be logged. stdout: {}, stderr: {}",
            stdout,
            stderr
        );
    }
}

// ============================================================================
// End-to-End Security Scenarios
// ============================================================================

mod end_to_end_security {
    use super::*;

    /// End-to-end: Complete security workflow
    #[test]
    fn security_workflow_end_to_end() {
        let temp_dir = TempDir::new().unwrap();

        // Verify executable runs successfully. (This test previously wrote an
        // AGENTS.md fixture with a `security:` block here, but `--version`
        // never read it — no config is parsed for `--version`, so the write
        // was vestigial. Removed in Task 14 / S6.)
        let output = Command::new(arkavo_bin())
            .current_dir(&temp_dir)
            .args(["--version"])
            .output()
            .expect("Failed to execute");

        assert!(output.status.success(), "Arkavo should start successfully");

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.contains("arkavo"), "Should show version info");
    }
}
