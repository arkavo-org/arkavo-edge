//! Integration test for Nemotron 3.5 Bolt 74B-A9B, the registered build
//! (unsloth/NVIDIA-Nemotron-3.5-Bolt-74B-A9B-GGUF, UD-IQ2_XXS).
//!
//! Bolt is a Mamba-2/attention hybrid MoE that routes each token to 9
//! experts, uses a ChatML template with qwen3_coder-style XML tool calls, and
//! switches reasoning with the template's `enable_thinking`. These tests drive
//! all of that through the provider: prompt processing (which needs the Metal
//! 9-expert kernel patch), native PEG tool-call parsing, and a tool result
//! carried back into the conversation.
//!
//! Run with: cargo test --features llama-cpp -p arkavo-llm --test nemotron_bolt_test -- --ignored --nocapture

#![allow(clippy::disallowed_methods)] // tokio::test uses block_on internally

#[cfg(all(test, feature = "llama-cpp"))]
mod tests {
    use arkavo_llm::{
        LlamaCppProvider, LocalToolFormat, Message, Provider, Role, SamplingConfig, ThinkingMode,
    };

    const REPO_DIR: &str = "models--unsloth--NVIDIA-Nemotron-3.5-Bolt-74B-A9B-GGUF";
    const FILE: &str = "NVIDIA-Nemotron-3.5-Bolt-74B-A9B-UD-IQ2_XXS.gguf";

    const TOOLS: &str = r#"[
        {"name":"get_weather","description":"Get current weather for a city","input_schema":{"type":"object","properties":{"location":{"type":"string","description":"City name"}},"required":["location"]}},
        {"name":"get_current_time","description":"Get the current wall-clock time in a timezone or city","input_schema":{"type":"object","properties":{"timezone":{"type":"string","description":"IANA timezone or city name"}},"required":["timezone"]}},
        {"name":"calculate","description":"Evaluate an arithmetic expression","input_schema":{"type":"object","properties":{"expression":{"type":"string","description":"Arithmetic expression, e.g. 15 * 7"}},"required":["expression"]}}
    ]"#;

    fn find_model() -> Option<String> {
        let home = std::env::var("HOME").ok()?;
        let snapshots = std::fs::read_dir(format!(
            "{home}/.cache/huggingface/hub/{REPO_DIR}/snapshots"
        ))
        .ok()?;
        snapshots
            .flatten()
            .map(|s| s.path().join(FILE))
            .find(|p| p.exists())
            .map(|p| p.to_string_lossy().to_string())
    }

    fn load(thinking: ThinkingMode, max_tokens: u32) -> Option<LlamaCppProvider> {
        let Some(path) = find_model() else {
            eprintln!("Skipping: {FILE} not found in the HuggingFace cache");
            return None;
        };
        let config = SamplingConfig {
            temperature: 0.1,
            tool_format: LocalToolFormat::Xml,
            thinking_mode: Some(thinking),
            max_tokens,
            ..Default::default()
        };
        Some(
            LlamaCppProvider::new_with_config("nemotron-3.5-bolt".to_string(), path, None, config)
                .expect("Failed to load model"),
        )
    }

    fn user(content: &str) -> Message {
        Message {
            role: Role::User,
            content: content.to_string(),
            ..Default::default()
        }
    }

    fn tools() -> serde_json::Value {
        serde_json::from_str(TOOLS).unwrap()
    }

    #[tokio::test]
    #[ignore = "requires local Nemotron 3.5 Bolt model (~22GB)"]
    async fn bolt_answers_a_plain_question() {
        let Some(provider) = load(ThinkingMode::Off, 64) else {
            return;
        };
        // Long enough that prompt processing runs the batched MoE kernel,
        // which crashed on Metal before the 9-expert patch.
        let filler = "Arkavo Edge runs agents on local hardware. ".repeat(40);
        let response = provider
            .complete(vec![user(&format!(
                "{filler}\nWhat is the capital of France? Answer in one word."
            ))])
            .await
            .expect("completion failed");
        println!("response: {response}");
        assert!(response.to_lowercase().contains("paris"), "got: {response}");
    }

    #[tokio::test]
    #[ignore = "requires local Nemotron 3.5 Bolt model (~22GB)"]
    async fn bolt_selects_the_matching_tool_with_its_argument() {
        let Some(provider) = load(ThinkingMode::Off, 256) else {
            return;
        };
        let response = provider
            .complete_with_tools(
                vec![user("What time is it in Tokyo right now?")],
                Some(tools()),
                None,
            )
            .await
            .expect("completion failed");
        println!(
            "content: {}\ncalls: {:?}",
            response.content, response.tool_calls
        );
        let call = response.tool_calls.first().expect("expected a tool call");
        assert_eq!(call.tool_name, "get_current_time");
        let tz = call.arguments["timezone"].as_str().unwrap_or_default();
        assert!(tz.to_lowercase().contains("tokyo"), "timezone was {tz:?}");
    }

    #[tokio::test]
    #[ignore = "requires local Nemotron 3.5 Bolt model (~22GB)"]
    async fn bolt_answers_from_a_tool_result_without_calling_again() {
        let Some(provider) = load(ThinkingMode::Off, 256) else {
            return;
        };
        let first = provider
            .complete_with_tools(
                vec![user("What's the weather in Paris?")],
                Some(tools()),
                None,
            )
            .await
            .expect("completion failed");
        let call = first
            .tool_calls
            .first()
            .filter(|c| c.tool_name == "get_weather")
            .expect("expected a get_weather call");
        let id = call.call_id.clone().unwrap_or_else(|| "call_0".to_string());

        let second = provider
            .complete_with_tools(
                vec![
                    user("What's the weather in Paris?"),
                    Message::assistant(&first.content),
                    Message::tool_result("Sunny, 18 degrees Celsius", &id, "get_weather"),
                ],
                Some(tools()),
                None,
            )
            .await
            .expect("completion failed");
        println!("answer: {}\ncalls: {:?}", second.content, second.tool_calls);
        let answer = second.content.to_lowercase();
        assert!(second.tool_calls.is_empty(), "called a tool again");
        assert!(
            answer.contains("18") || answer.contains("sunny"),
            "got: {answer}"
        );
    }

    #[tokio::test]
    #[ignore = "requires local Nemotron 3.5 Bolt model (~22GB)"]
    async fn bolt_reasons_when_thinking_is_on() {
        let Some(provider) = load(ThinkingMode::On, 4096) else {
            return;
        };
        let response = provider
            .complete_with_tools(
                vec![user(
                    "A train travels 150 km in 2 hours, then 60 km in 1 hour. What is its \
                     average speed over the whole trip in km/h? Reply with only the number.",
                )],
                None,
                None,
            )
            .await
            .expect("completion failed");
        println!(
            "reasoning: {:?}\nanswer: {}",
            response.reasoning_content, response.content
        );
        assert!(response.content.contains("70"), "got: {}", response.content);
        assert!(
            !response.content.contains("</think>"),
            "reasoning leaked into the answer: {}",
            response.content
        );
    }
}
