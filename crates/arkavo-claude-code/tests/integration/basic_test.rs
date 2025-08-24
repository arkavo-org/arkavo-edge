#[cfg(test)]
mod tests {
    use arkavo_claude_code::{ClaudeCodeCapability, ClaudeCodeConfig};
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_capability_creation() {
        let temp_dir = tempdir().unwrap();
        let config = ClaudeCodeConfig {
            enabled: true,
            workspace_root: temp_dir.path().to_path_buf(),
            ..Default::default()
        };
        
        // Mock event writer
        let event_writer = arkavo_events::EventWriter::new(
            arkavo_events::EventWriterConfig {
                file_path: temp_dir.path().join("events.jsonl"),
                max_file_size: 10_000_000,
                max_retention_hours: 24,
            }
        ).await.unwrap();
        
        let capability = ClaudeCodeCapability::new(
            config,
            "test-agent".to_string(),
            std::sync::Arc::new(event_writer),
            None,
            None,
        ).await;
        
        assert!(capability.is_ok());
    }
    
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
}