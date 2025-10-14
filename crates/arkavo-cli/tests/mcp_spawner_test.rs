#![allow(clippy::disallowed_methods)]
#![allow(clippy::future_not_send)]
#![allow(dead_code)]
#![allow(clippy::format_push_string)]
#![allow(clippy::significant_drop_tightening)]
#![allow(clippy::unnecessary_debug_formatting)]
#![allow(clippy::lines_filter_map_ok)]
#![allow(clippy::manual_strip)]
#![allow(clippy::needless_continue)]
#![allow(unused_imports)]
#![allow(clippy::zombie_processes)]
#![allow(clippy::unreadable_literal)]
#![allow(clippy::ignore_without_reason)]
#![allow(clippy::unnecessary_unwrap)]
#![allow(unreachable_pub)]

#[cfg(test)]
mod tests {
    use arkavo_cli::mcp_spawner::McpProcessManager;

    #[test]
    fn test_process_manager_creation() {
        let manager = McpProcessManager::new();
        // Should successfully shutdown even with no processes
        assert!(manager.shutdown_all().is_ok());
    }

    #[test]
    fn test_invalid_command_validation() {
        let manager = McpProcessManager::new();

        // Try to spawn a non-existent command
        let result = manager.spawn_mcp_server(
            "test-server".to_string(),
            "this-command-does-not-exist-12345",
            &[],
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[test]
    fn test_valid_command_spawn() {
        let manager = McpProcessManager::new();

        // Use a simple command that exists on all platforms
        let result =
            manager.spawn_mcp_server("test-echo".to_string(), "echo", &["hello".to_string()]);

        // This should succeed if echo command exists
        if result.is_ok() {
            let mut process = result.unwrap();
            // Clean up the process
            let _ = process.child.kill();
            let _ = manager.shutdown_all();
        }
    }
}
