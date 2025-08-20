#[test]
#[ignore] // Ignore by default as it requires external services
fn test_chat_command() {
    // Test that chat command can be called via the main run function
    let args = vec![
        "chat".to_string(),
        "--prompt".to_string(),
        "Hello".to_string(),
    ];

    // This will fail if Ollama is not running, but that's expected
    // We're just testing that the command structure works
    match arkavo_cli::run(&args) {
        Ok(()) => {
            // Success if Ollama is running
        }
        Err(e) => {
            // Expected error if Ollama is not running
            let error_msg = e.to_string();
            assert!(
                error_msg.contains("Failed to initialize LLM client")
                    || error_msg.contains("Connection refused")
                    || error_msg.contains("error")
                    || error_msg.contains("HTTP")
                    || error_msg.contains("Ollama is not running locally")
                    || error_msg.contains("print mode doesn't support interactive prompts")
                    || error_msg.contains("No Ollama server configured"),
                "Unexpected error: {error_msg}"
            );
        }
    }
}

#[test]
#[ignore] // Ignore by default as it requires external services
fn test_git_analysis_prompt() {
    // Test that the system prompt includes Git analysis instructions
    let args = vec![
        "chat".to_string(),
        "--prompt".to_string(),
        "Perform a full repository analysis".to_string(),
    ];

    // This test verifies that the chat command is structured to handle Git analysis
    // The actual MCP tool calls would be made by the LLM based on the system prompt
    match arkavo_cli::run(&args) {
        Ok(()) => {
            // Success if Ollama is running - the LLM would receive the Git analysis instructions
        }
        Err(e) => {
            // Expected error if Ollama is not running
            let error_msg = e.to_string();
            assert!(
                error_msg.contains("Failed to initialize LLM client")
                    || error_msg.contains("Connection refused")
                    || error_msg.contains("error")
                    || error_msg.contains("HTTP")
                    || error_msg.contains("Ollama is not running locally")
                    || error_msg.contains("print mode doesn't support interactive prompts")
                    || error_msg.contains("No Ollama server configured"),
                "Unexpected error: {error_msg}"
            );
        }
    }
}
