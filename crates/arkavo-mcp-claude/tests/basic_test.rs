#[cfg(test)]
mod tests {
    use arkavo_mcp_claude::ClaudeCodeConfig;
    use std::path::PathBuf;

    #[test]
    fn test_config_validation() {
        let config = ClaudeCodeConfig {
            enabled: true,
            workspace_root: PathBuf::from("/nonexistent"),
            ..Default::default()
        };

        let result = config.validate();
        assert!(result.is_err());
    }

    #[test]
    fn test_path_filtering() {
        let config = ClaudeCodeConfig {
            allow_globs: vec!["src/**".to_string()],
            deny_globs: vec!["**/.secrets/**".to_string()],
            ..Default::default()
        };

        assert!(config.is_path_allowed("src/main.rs"));
        assert!(!config.is_path_allowed(".secrets/api_key"));
        assert!(!config.is_path_allowed("other/file.txt"));
    }

    #[test]
    fn test_config_budget_validation() {
        let config = ClaudeCodeConfig {
            max_budget_usd: Some(-1.0),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_config_permission_mode_validation() {
        let config = ClaudeCodeConfig {
            permission_mode: Some("invalid".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let config = ClaudeCodeConfig {
            permission_mode: Some("plan".to_string()),
            ..Default::default()
        };
        assert!(config.validate().is_ok());
    }

    #[tokio::test]
    async fn test_capability_creation() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = ClaudeCodeConfig {
            enabled: true,
            workspace_root: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let event_writer = std::sync::Arc::new(arkavo_events::EventWriter::new(
            arkavo_events::EventWriterConfig::default(),
        ));

        let capability = arkavo_mcp_claude::ClaudeCodeCapability::new(
            config,
            "test-agent".to_string(),
            event_writer,
            None,
            None,
        );

        assert!(capability.is_ok());
    }

    #[tokio::test]
    async fn test_capability_tools_list() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = ClaudeCodeConfig {
            enabled: true,
            workspace_root: temp_dir.path().to_path_buf(),
            ..Default::default()
        };

        let event_writer = std::sync::Arc::new(arkavo_events::EventWriter::new(
            arkavo_events::EventWriterConfig::default(),
        ));

        let capability = arkavo_mcp_claude::ClaudeCodeCapability::new(
            config,
            "test-agent".to_string(),
            event_writer,
            None,
            None,
        )
        .unwrap();

        let tools = capability.list_tools();
        assert_eq!(tools.len(), 4);

        let tool_names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert!(tool_names.contains(&"claude_code_run"));
        assert!(tool_names.contains(&"claude_code_plan"));
        assert!(tool_names.contains(&"claude_code_session_info"));
        assert!(tool_names.contains(&"claude_code_interrupt"));
    }

    #[tokio::test]
    async fn test_capability_budget_status() {
        let temp_dir = tempfile::tempdir().unwrap();
        let config = ClaudeCodeConfig {
            enabled: true,
            workspace_root: temp_dir.path().to_path_buf(),
            max_budget_usd: Some(5.0),
            ..Default::default()
        };

        let event_writer = std::sync::Arc::new(arkavo_events::EventWriter::new(
            arkavo_events::EventWriterConfig::default(),
        ));

        let capability = arkavo_mcp_claude::ClaudeCodeCapability::new(
            config,
            "test-agent".to_string(),
            event_writer,
            None,
            None,
        )
        .unwrap();

        let status = capability.compute_budget_status();
        assert_eq!(status["total_cost_usd"], 0.0);
        assert_eq!(status["max_budget_usd"], 5.0);
        assert_eq!(status["remaining_cost_usd"], 5.0);
        assert_eq!(status["used_tokens"], 0);
    }
}
