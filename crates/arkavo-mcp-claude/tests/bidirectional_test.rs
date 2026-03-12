//! Integration tests for bidirectional ClaudeSDKClient sessions.
//!
//! All tests are `#[ignore]` because they require:
//! - Claude Code CLI installed (`claude` binary in PATH)
//! - Valid authentication (OAuth via `claude login` or ANTHROPIC_API_KEY)
//!
//! Run with: `cargo test -p arkavo-mcp-claude --test bidirectional_test -- --ignored`

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arkavo_mcp_claude::{ClaudeCodeCapability, ClaudeCodeConfig};

    fn make_config(temp_dir: &tempfile::TempDir) -> ClaudeCodeConfig {
        ClaudeCodeConfig {
            enabled: true,
            workspace_root: temp_dir.path().to_path_buf(),
            use_bidirectional: true,
            max_budget_usd: Some(1.0),
            ..Default::default()
        }
    }

    fn make_event_writer() -> Arc<arkavo_events::EventWriter> {
        Arc::new(arkavo_events::EventWriter::new(
            arkavo_events::EventWriterConfig::default(),
        ))
    }

    #[tokio::test]
    #[ignore]
    async fn test_bidirectional_session_lifecycle() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = make_config(&temp_dir);
        let event_writer = make_event_writer();

        let capability =
            ClaudeCodeCapability::new(config, "test-agent".to_string(), event_writer, None, None)
                .expect("capability creation should succeed");

        // Prepare authenticates and starts the bidirectional session
        capability
            .prepare()
            .await
            .expect("prepare should succeed with valid auth");

        // Run a simple query
        let run_id = capability
            .start_run("What is 2+2? Reply with just the number.".to_string(), None)
            .await
            .expect("run should succeed");

        assert!(run_id.starts_with("run-"));

        // Shutdown gracefully
        capability
            .shutdown()
            .await
            .expect("shutdown should succeed");
    }

    #[tokio::test]
    #[ignore]
    async fn test_budget_tracking() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = make_config(&temp_dir);
        let event_writer = make_event_writer();

        let capability =
            ClaudeCodeCapability::new(config, "test-agent".to_string(), event_writer, None, None)
                .unwrap();

        capability.prepare().await.unwrap();

        // Run a query to generate some cost
        capability
            .start_run("Say hello in one word.".to_string(), None)
            .await
            .unwrap();

        // Check budget status reflects usage
        let status = capability.compute_budget_status();
        let total_cost = status["total_cost_usd"].as_f64().unwrap();
        assert!(
            total_cost > 0.0,
            "Expected non-zero cost after a run, got {total_cost}"
        );

        let remaining = status["remaining_cost_usd"].as_f64().unwrap();
        assert!(remaining < 1.0, "Remaining should be less than max budget");

        capability.shutdown().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_session_info_introspection() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = make_config(&temp_dir);
        let event_writer = make_event_writer();

        let capability =
            ClaudeCodeCapability::new(config, "test-agent".to_string(), event_writer, None, None)
                .unwrap();

        capability.prepare().await.unwrap();

        let info = capability
            .session_info()
            .await
            .expect("session_info should succeed");

        // Bidirectional sessions should report model and tools
        assert!(
            info.get("model").is_some() || info.get("status").is_some(),
            "Session info should contain model or status: {info}"
        );

        capability.shutdown().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_permission_enforcement() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = ClaudeCodeConfig {
            enabled: true,
            workspace_root: temp_dir.path().to_path_buf(),
            use_bidirectional: true,
            max_budget_usd: Some(1.0),
            disallowed_tools: vec!["Bash".to_string()],
            ..Default::default()
        };
        let event_writer = make_event_writer();

        let capability =
            ClaudeCodeCapability::new(config, "test-agent".to_string(), event_writer, None, None)
                .unwrap();

        capability.prepare().await.unwrap();

        // Run a query that would normally use Bash — it should complete but
        // with Bash denied, the SDK will skip shell commands
        let run_id = capability
            .start_run(
                "List files in the current directory. Use only allowed tools.".to_string(),
                None,
            )
            .await
            .expect("run should succeed even with tool restrictions");

        assert!(run_id.starts_with("run-"));

        capability.shutdown().await.unwrap();
    }

    #[tokio::test]
    #[ignore]
    async fn test_interrupt() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = make_config(&temp_dir);
        let event_writer = make_event_writer();

        let capability = Arc::new(
            ClaudeCodeCapability::new(config, "test-agent".to_string(), event_writer, None, None)
                .unwrap(),
        );

        capability.prepare().await.unwrap();

        // Start a potentially long-running query in the background
        let cap_clone = Arc::clone(&capability);
        let handle = tokio::spawn(async move {
            cap_clone
                .start_run(
                    "Write a detailed essay about the history of computing.".to_string(),
                    None,
                )
                .await
        });

        // Give it a moment to start, then interrupt
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        capability
            .stop_run("active".to_string())
            .await
            .expect("interrupt should succeed");

        // The run should complete (either with partial results or an error)
        let result = handle.await.unwrap();
        // We don't assert success — interrupt may cause the run to error
        drop(result);

        capability.shutdown().await.unwrap();
    }
}
