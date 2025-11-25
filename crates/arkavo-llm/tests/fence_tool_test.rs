//! Integration test for fence-based tool calling with Gemma-3 270M
//!
//! Run with: cargo test --features llama-cpp -p arkavo-llm --test fence_tool_test -- --ignored --nocapture

#[cfg(all(test, feature = "llama-cpp"))]
mod tests {
    use arkavo_llm::{LlamaCppProvider, LocalToolFormat, Message, Provider, Role, SamplingConfig};
    use serde_json::json;

    const TEST_TOOLS: &str = r#"[{
        "name": "get_weather",
        "description": "Get current weather for a location",
        "input_schema": {
            "type": "object",
            "properties": {
                "location": {"type": "string", "description": "City name"},
                "unit": {"type": "string", "description": "celsius or fahrenheit"}
            },
            "required": ["location"]
        }
    }, {
        "name": "read_file",
        "description": "Read contents of a file",
        "input_schema": {
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "File path to read"}
            },
            "required": ["path"]
        }
    }]"#;

    fn find_gemma_model() -> Option<String> {
        // Check common HuggingFace cache locations for Gemma-3 270M
        let home = std::env::var("HOME").ok()?;
        let cache_dir = format!("{home}/.cache/huggingface/hub");

        // Look for unsloth/gemma-3-270m-it-GGUF
        let patterns = [
            "models--unsloth--gemma-3-270m-it-GGUF",
            "models--google--gemma-3-270m-it-GGUF",
        ];

        for pattern in patterns {
            let model_dir = format!("{cache_dir}/{pattern}");
            if let Ok(entries) = std::fs::read_dir(&model_dir) {
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy().starts_with("snapshots") {
                        if let Ok(snapshots) = std::fs::read_dir(entry.path()) {
                            for snapshot in snapshots.flatten() {
                                let gguf_path = snapshot.path().join("gemma-3-270m-it-Q4_0.gguf");
                                if gguf_path.exists() {
                                    return Some(gguf_path.to_string_lossy().to_string());
                                }
                                // Also try Q4_K_M variant
                                let gguf_path =
                                    snapshot.path().join("gemma-3-270m-it-Q4_K_M.gguf");
                                if gguf_path.exists() {
                                    return Some(gguf_path.to_string_lossy().to_string());
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }

    #[tokio::test]
    #[ignore = "requires local Gemma-3 270M model"]
    async fn test_fence_format_weather() {
        let model_path = match find_gemma_model() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: Gemma-3 270M model not found in HuggingFace cache");
                return;
            }
        };

        println!("Using model: {model_path}");

        let config = SamplingConfig {
            temperature: 0.1, // Low temperature for deterministic output
            tool_format: LocalToolFormat::Fence,
            max_tokens: 256,
            debug: true,
            ..Default::default()
        };

        let provider = LlamaCppProvider::new_with_config(
            "gemma-3-270m".to_string(),
            model_path,
            None,
            config,
        )
        .expect("Failed to load model");

        let tools: serde_json::Value = serde_json::from_str(TEST_TOOLS).unwrap();

        let messages = vec![Message {
            role: Role::User,
            content: "What's the weather in New York?".to_string(),
            images: None,
        }];

        println!("\n=== Testing: What's the weather in New York? ===");

        let response = provider
            .complete_with_tools(messages, Some(tools), None)
            .await
            .expect("Failed to complete");

        println!("\nModel response:\n{}", response.content);
        println!("\nParsed tool calls: {:?}", response.tool_calls);

        // Check if we got a tool call
        if response.tool_calls.is_empty() {
            println!("\nWARNING: No tool calls parsed from response");
            println!("Expected fence format like:");
            println!("```get_weather");
            println!("location: New York");
            println!("```");
        } else {
            let call = &response.tool_calls[0];
            println!("\nSUCCESS: Parsed tool call:");
            println!("  Tool: {}", call.tool_name);
            println!("  Args: {}", call.arguments);

            assert_eq!(call.tool_name, "get_weather");
            assert!(call.arguments.get("location").is_some());
        }
    }

    #[tokio::test]
    #[ignore = "requires local Gemma-3 270M model"]
    async fn test_fence_format_file_read() {
        let model_path = match find_gemma_model() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: Gemma-3 270M model not found in HuggingFace cache");
                return;
            }
        };

        let config = SamplingConfig {
            temperature: 0.1,
            tool_format: LocalToolFormat::Fence,
            max_tokens: 256,
            debug: true,
            ..Default::default()
        };

        let provider = LlamaCppProvider::new_with_config(
            "gemma-3-270m".to_string(),
            model_path,
            None,
            config,
        )
        .expect("Failed to load model");

        let tools: serde_json::Value = serde_json::from_str(TEST_TOOLS).unwrap();

        let messages = vec![Message {
            role: Role::User,
            content: "Read the file at /tmp/test.txt".to_string(),
            images: None,
        }];

        println!("\n=== Testing: Read the file at /tmp/test.txt ===");

        let response = provider
            .complete_with_tools(messages, Some(tools), None)
            .await
            .expect("Failed to complete");

        println!("\nModel response:\n{}", response.content);
        println!("\nParsed tool calls: {:?}", response.tool_calls);

        if !response.tool_calls.is_empty() {
            let call = &response.tool_calls[0];
            println!("\nSUCCESS: Parsed tool call:");
            println!("  Tool: {}", call.tool_name);
            println!("  Args: {}", call.arguments);
        }
    }

    #[tokio::test]
    #[ignore = "requires local Gemma-3 270M model"]
    async fn test_compare_fence_vs_xml() {
        let model_path = match find_gemma_model() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: Gemma-3 270M model not found in HuggingFace cache");
                return;
            }
        };

        let tools: serde_json::Value = serde_json::from_str(TEST_TOOLS).unwrap();

        let test_prompts = vec![
            "What's the weather in London?",
            "Check the weather in Tokyo, use celsius",
            "Read /etc/hosts file",
        ];

        println!("\n=== Comparing Fence vs XML formats ===\n");

        for prompt in test_prompts {
            println!("Prompt: {prompt}");

            // Test with Fence format
            let fence_config = SamplingConfig {
                temperature: 0.1,
                tool_format: LocalToolFormat::Fence,
                max_tokens: 256,
                ..Default::default()
            };

            let fence_provider = LlamaCppProvider::new_with_config(
                "gemma-3-270m".to_string(),
                model_path.clone(),
                None,
                fence_config,
            )
            .expect("Failed to load model");

            let messages = vec![Message {
                role: Role::User,
                content: prompt.to_string(),
                images: None,
            }];

            let fence_result = fence_provider
                .complete_with_tools(messages.clone(), Some(tools.clone()), None)
                .await;

            let fence_success = fence_result
                .as_ref()
                .map(|r| !r.tool_calls.is_empty())
                .unwrap_or(false);

            // Test with XML format
            let xml_config = SamplingConfig {
                temperature: 0.1,
                tool_format: LocalToolFormat::Xml,
                max_tokens: 256,
                ..Default::default()
            };

            let xml_provider = LlamaCppProvider::new_with_config(
                "gemma-3-270m".to_string(),
                model_path.clone(),
                None,
                xml_config,
            )
            .expect("Failed to load model");

            let xml_result = xml_provider
                .complete_with_tools(messages, Some(tools.clone()), None)
                .await;

            let xml_success = xml_result
                .as_ref()
                .map(|r| !r.tool_calls.is_empty())
                .unwrap_or(false);

            println!(
                "  Fence: {} | XML: {}",
                if fence_success { "PASS" } else { "FAIL" },
                if xml_success { "PASS" } else { "FAIL" }
            );
            println!("---");
        }
    }
}
