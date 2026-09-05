//! Integration test for Gemma-4-12B (ggml-org/gemma-4-12B-it-GGUF)
//!
//! Verifies the model loads, generates coherent text, and drives the
//! Gemma-4 tool-call grammar path. The model is registered in
//! `arkavo_router::decision::ModelChoice::LocalGemma4_12B`.
//!
//! Run with: cargo test --features llama-cpp -p arkavo-llm --test gemma4_12b_test -- --ignored --nocapture

#![allow(clippy::disallowed_methods)] // tokio::test uses block_on internally

#[cfg(all(test, feature = "llama-cpp"))]
mod tests {
    use arkavo_llm::{LlamaCppProvider, LocalToolFormat, Message, Provider, Role, SamplingConfig};

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
    }]"#;

    /// Resolve the cached GGUF for ggml-org/gemma-4-12B-it-GGUF.
    fn find_gemma4_12b_model() -> Option<String> {
        let home = std::env::var("HOME").ok()?;
        let model_dir =
            format!("{home}/.cache/huggingface/hub/models--ggml-org--gemma-4-12B-it-GGUF");
        let snapshots = std::fs::read_dir(format!("{model_dir}/snapshots")).ok()?;
        for snapshot in snapshots.flatten() {
            let gguf = snapshot.path().join("gemma-4-12B-it-Q4_K_M.gguf");
            if gguf.exists() {
                return Some(gguf.to_string_lossy().to_string());
            }
        }
        None
    }

    #[tokio::test]
    #[ignore = "requires local Gemma-4-12B model (~7GB)"]
    async fn test_gemma4_12b_plain_generation() {
        let model_path = match find_gemma4_12b_model() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: Gemma-4-12B model not found in HuggingFace cache");
                return;
            }
        };
        println!("Using model: {model_path}");

        let config = SamplingConfig {
            temperature: 0.1,
            max_tokens: 64,
            debug: true,
            ..Default::default()
        };

        let provider =
            LlamaCppProvider::new_with_config("gemma-4-12b".to_string(), model_path, None, config)
                .expect("Failed to load model");

        let messages = vec![Message {
            response_items: Vec::new(),
            role: Role::User,
            content: "What is the capital of France? Answer in one word.".to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }];

        let response = provider
            .complete(messages)
            .await
            .expect("completion failed");
        println!("\nModel response:\n{response}");

        assert!(
            response.to_lowercase().contains("paris"),
            "expected 'Paris' in response, got: {response}"
        );
    }

    #[tokio::test]
    #[ignore = "requires local Gemma-4-12B model (~7GB)"]
    async fn test_gemma4_12b_tool_call() {
        let model_path = match find_gemma4_12b_model() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: Gemma-4-12B model not found in HuggingFace cache");
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

        let provider =
            LlamaCppProvider::new_with_config("gemma-4-12b".to_string(), model_path, None, config)
                .expect("Failed to load model");

        let tools: serde_json::Value = serde_json::from_str(TEST_TOOLS).unwrap();

        let messages = vec![Message {
            response_items: Vec::new(),
            role: Role::User,
            content: "What's the weather in New York?".to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }];

        let response = provider
            .complete_with_tools(messages, Some(tools), None)
            .await
            .expect("completion failed");

        println!("\nModel response:\n{}", response.content);
        println!("Parsed tool calls: {:?}", response.tool_calls);

        assert!(
            !response.tool_calls.is_empty(),
            "expected a tool call, got none. content: {}",
            response.content
        );
        let call = &response.tool_calls[0];
        assert_eq!(call.tool_name, "get_weather");
        assert!(
            call.arguments.get("location").is_some(),
            "expected a 'location' argument, got: {}",
            call.arguments
        );
    }

    /// Three tools with distinct purposes — verify the model selects the
    /// correct one (and the right argument) for a time query rather than
    /// defaulting to the first tool.
    const MULTI_TOOLS: &str = r#"[
        {"name":"get_weather","description":"Get current weather for a city","input_schema":{"type":"object","properties":{"location":{"type":"string","description":"City name"}},"required":["location"]}},
        {"name":"get_current_time","description":"Get the current wall-clock time in a timezone or city","input_schema":{"type":"object","properties":{"timezone":{"type":"string","description":"IANA timezone or city name"}},"required":["timezone"]}},
        {"name":"calculate","description":"Evaluate an arithmetic expression","input_schema":{"type":"object","properties":{"expression":{"type":"string","description":"Arithmetic expression, e.g. 15 * 7"}},"required":["expression"]}}
    ]"#;

    fn load_provider() -> Option<LlamaCppProvider> {
        let model_path = find_gemma4_12b_model()?;
        let config = SamplingConfig {
            temperature: 0.1,
            tool_format: LocalToolFormat::Fence,
            max_tokens: 512,
            debug: false,
            ..Default::default()
        };
        Some(
            LlamaCppProvider::new_with_config("gemma-4-12b".to_string(), model_path, None, config)
                .expect("Failed to load model"),
        )
    }

    #[tokio::test]
    #[ignore = "requires local Gemma-4-12B model (~7GB)"]
    async fn test_gemma4_12b_tool_selection() {
        let provider = match load_provider() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: Gemma-4-12B model not found in HuggingFace cache");
                return;
            }
        };
        let tools: serde_json::Value = serde_json::from_str(MULTI_TOOLS).unwrap();

        let messages = vec![Message {
            response_items: Vec::new(),
            role: Role::User,
            content: "What time is it in Tokyo right now?".to_string(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }];

        let response = provider
            .complete_with_tools(messages, Some(tools), None)
            .await
            .expect("completion failed");
        println!("Parsed tool calls: {:?}", response.tool_calls);

        assert!(
            !response.tool_calls.is_empty(),
            "expected a tool call, got none. content: {}",
            response.content
        );
        let call = &response.tool_calls[0];
        assert_eq!(
            call.tool_name, "get_current_time",
            "expected get_current_time to be selected over weather/calculate"
        );
        let tz = call
            .arguments
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_lowercase();
        assert!(
            tz.contains("tokyo"),
            "expected timezone argument to reference Tokyo, got: {}",
            call.arguments
        );
    }

    /// Full multi-turn loop: model calls a tool, we feed the result back as a
    /// `Role::Tool` message, then the model must produce a grounded final
    /// answer. Round 1+ is where the Gemma-4 PEG content rule historically
    /// ate tool-call tokens, so this exercises the regression-prone path.
    #[tokio::test]
    #[ignore = "requires local Gemma-4-12B model (~7GB)"]
    async fn test_gemma4_12b_multi_turn() {
        use arkavo_llm::Message as Msg;

        let provider = match load_provider() {
            Some(p) => p,
            None => {
                eprintln!("Skipping: Gemma-4-12B model not found in HuggingFace cache");
                return;
            }
        };
        let tools: serde_json::Value = serde_json::from_str(MULTI_TOOLS).unwrap();

        let mut conversation = vec![Msg::user("What's the weather in Paris?")];

        // --- Round 0: expect a get_weather tool call ---
        let r0 = provider
            .complete_with_tools(conversation.clone(), Some(tools.clone()), None)
            .await
            .expect("round 0 failed");
        println!("Round 0 tool calls: {:?}", r0.tool_calls);
        assert!(
            !r0.tool_calls.is_empty(),
            "round 0: expected a tool call, got none. content: {}",
            r0.content
        );
        let call = &r0.tool_calls[0];
        assert_eq!(call.tool_name, "get_weather");
        let call_id = call.call_id.clone().unwrap_or_else(|| "call_0".to_string());

        // --- Feed the tool result back exactly as the conductor loop does ---
        // (arkavo-server/src/server/conductor_tool_loop.rs:444 + 554): the
        // assistant turn is pushed as `Message::assistant(response.content)`.
        // For Gemma 4 the parsed tool-call markup has already been stripped out
        // of `content` (it lives in `tool_calls`), and the llama chat template
        // conversion only carries role+content strings — there is no structured
        // tool_calls field (llamacpp_provider.rs:788). So the assistant's call
        // is lost. The provider's Gemma-4 tool-call synthesis (llamacpp_provider
        // meta-building) reconstructs the assistant's structured tool_calls from
        // the following tool-role message so the Jinja template's
        // convert_tool_responses_gemma4 path renders both the call and the
        // response. Without that, the tool result is orphaned and dropped.
        conversation.push(Msg::assistant(&r0.content));
        conversation.push(Msg::tool_result(
            "Sunny, 18 degrees Celsius",
            &call_id,
            "get_weather",
        ));

        // --- Round 1: the model must answer using the tool result ---
        let r1 = provider
            .complete_with_tools(conversation, Some(tools), None)
            .await
            .expect("round 1 failed");
        println!("Round 1 content:\n{}", r1.content);
        println!("Round 1 tool calls: {:?}", r1.tool_calls);

        // The tool result reached the model: it produces a grounded final answer
        // (sunny / 18C) instead of re-issuing the same get_weather call.
        let answer = r1.content.to_lowercase();
        assert!(
            answer.contains("18") || answer.contains("sunny"),
            "round 1: expected a grounded answer using the tool result \
             (sunny / 18C); got tool_calls={:?} content={}",
            r1.tool_calls,
            r1.content
        );
        assert!(
            r1.tool_calls.is_empty(),
            "round 1: expected no further tool call once the result is known, got {:?}",
            r1.tool_calls
        );
        // Generation must stop at the <turn|> EOG token, not run to max_tokens
        // repeating the answer. A clean single answer is well under 500 chars;
        // the pre-fix repetition ran to ~2000+ chars.
        assert!(
            r1.content.len() < 500,
            "round 1: answer looks repeated (EOG not honored), {} chars: {}",
            r1.content.len(),
            r1.content
        );
    }
}
