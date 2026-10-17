//! Native tool-call parsing through the parser a template application
//! returns. llama.cpp v0.7.0 dropped PEG parser serialization and began
//! prepending the generation prompt itself, so these tests pin both: the
//! parser handle outlives the template result, and output passed without the
//! prompt prefix still parses. One model per PEG format family.
//!
//! Run with: cargo test -p arkavo-llama-cpp --test chat_parser -- --ignored

use arkavo_llama_cpp::{
    ffi, parse_tool_calls, ChatInputs, ChatParser, ChatTool, LlamaModel, ToolChoice,
};
use std::ffi::CString;

fn cached(repo_dir: &str, file: &str) -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    std::fs::read_dir(format!(
        "{home}/.cache/huggingface/hub/{repo_dir}/snapshots"
    ))
    .ok()?
    .flatten()
    .map(|s| s.path().join(file))
    .find(|p| p.exists())
    .map(|p| p.to_string_lossy().to_string())
}

/// The parser from rendering a one-tool weather conversation.
fn parser_for(model_path: &str, enable_thinking: bool) -> ChatParser {
    let model = LlamaModel::from_file(model_path).expect("load model");
    let templates = model.chat_templates().expect("chat templates");
    let role = CString::new("user").unwrap();
    let content = CString::new("What's the weather in Paris?").unwrap();
    let messages = [ffi::llama_chat_message {
        role: role.as_ptr(),
        content: content.as_ptr(),
    }];
    let inputs = ChatInputs {
        tools: vec![ChatTool {
            name: "get_weather".to_string(),
            description: "Get current weather for a city".to_string(),
            parameters_json: r#"{"type":"object","properties":{"location":{"type":"string"}},"required":["location"]}"#
                .to_string(),
        }],
        tool_choice: ToolChoice::Auto,
        enable_thinking,
        add_generation_prompt: true,
        parallel_tool_calls: false,
    };
    let parser = templates
        .apply(&messages, &inputs)
        .expect("apply template")
        .parser
        .expect("a tool-calling template yields a parser");
    // Dropping the template set and model first proves the handle owns its parser.
    drop(templates);
    drop(model);
    parser
}

/// Parses `output` as the one tool call it must contain.
fn parse_with_template(model_path: &str, output: &str) -> (String, String) {
    let parser = parser_for(model_path, false);
    let (_, _, calls) = parse_tool_calls(output, &parser);
    assert_eq!(
        calls.len(),
        1,
        "expected one call from {output:?}, got {calls:?}"
    );
    (calls[0].name.clone(), calls[0].arguments.clone())
}

/// The arguments' JSON spacing is llama.cpp's choice, so compare without it.
fn compact(arguments: &str) -> String {
    arguments.chars().filter(|c| !c.is_whitespace()).collect()
}

#[test]
#[ignore = "requires unsloth/Qwen3.5-0.8B-GGUF in the HuggingFace cache"]
fn a_qwen_xml_tool_call_parses_without_the_generation_prompt() {
    let Some(path) = cached(
        "models--unsloth--Qwen3.5-0.8B-GGUF",
        "Qwen3.5-0.8B-Q4_K_M.gguf",
    ) else {
        eprintln!("skipping: Qwen3.5-0.8B GGUF not cached");
        return;
    };
    let output = "<tool_call>\n<function=get_weather>\n<parameter=location>\nParis\n</parameter>\n</function>\n</tool_call>";
    let (name, arguments) = parse_with_template(&path, output);
    assert_eq!(name, "get_weather");
    assert_eq!(compact(&arguments), r#"{"location":"Paris"}"#);
}

#[test]
#[ignore = "requires unsloth/gemma-4-E2B-it-GGUF in the HuggingFace cache"]
fn a_gemma4_tool_call_parses_without_the_generation_prompt() {
    let Some(path) = cached(
        "models--unsloth--gemma-4-E2B-it-GGUF",
        "gemma-4-E2B-it-Q4_K_M.gguf",
    ) else {
        eprintln!("skipping: Gemma 4 E2B GGUF not cached");
        return;
    };
    let output = "<|tool_call>call:get_weather{location:<|\"|>Paris<|\"|>}<tool_call|>";
    let (name, arguments) = parse_with_template(&path, output);
    assert_eq!(name, "get_weather");
    assert_eq!(compact(&arguments), r#"{"location":"Paris"}"#);
}

/// Regression: templates were applied with reasoning_format NONE, so a
/// thinking model's reasoning came back inline in the answer.
#[test]
#[ignore = "requires unsloth/Qwen3.5-0.8B-GGUF in the HuggingFace cache"]
fn reasoning_is_split_from_the_answer() {
    let Some(path) = cached(
        "models--unsloth--Qwen3.5-0.8B-GGUF",
        "Qwen3.5-0.8B-Q4_K_M.gguf",
    ) else {
        eprintln!("skipping: Qwen3.5-0.8B GGUF not cached");
        return;
    };
    let parser = parser_for(&path, true);
    let (content, reasoning, calls) =
        parse_tool_calls("Paris is in France.\n</think>\n\nIt is in France.", &parser);
    assert!(calls.is_empty(), "{calls:?}");
    assert_eq!(content.trim(), "It is in France.");
    assert_eq!(
        reasoning.as_deref().map(str::trim),
        Some("Paris is in France.")
    );
}
