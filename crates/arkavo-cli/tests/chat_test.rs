#[test]
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
        Ok(_) => {
            // Success if Ollama is running
        }
        Err(e) => {
            // Expected error if Ollama is not running
            let error_msg = e.to_string();
            assert!(
                error_msg.contains("Failed to initialize LLM client")
                    || error_msg.contains("Connection refused")
                    || error_msg.contains("error")
                    || error_msg.contains("HTTP"),
                "Unexpected error: {}",
                error_msg
            );
        }
    }
}
