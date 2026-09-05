use super::*;
use crate::ToolCall;

fn completed(output: Value) -> Value {
    json!({"status":"completed", "output":output,"usage":{"input_tokens":100,"output_tokens":60,"input_tokens_details":{"cached_tokens":75,"cache_write_tokens":10},"output_tokens_details":{"reasoning_tokens":40}}})
}

#[arkavo_test_macros::spec("ASTRA-001")]
#[test]
fn astra_request_omits_incompatible_sampling_and_keeps_system_order() {
    let body = request(
        &OpenAIResponsesConfig::default(),
        vec![Message::system("policy"), Message::user("task")],
        None,
        None,
        Some(42),
        false,
    )
    .unwrap();
    assert_eq!(body["model"], "gpt-6-astra");
    assert_eq!(body["reasoning"]["effort"], "medium");
    assert_eq!(body["max_output_tokens"], 42);
    assert_eq!(body["store"], false);
    assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
    assert_eq!(body["input"][0]["role"], "system");
    for field in [
        "temperature",
        "top_p",
        "logprobs",
        "max_tokens",
        "previous_response_id",
    ] {
        assert!(body.get(field).is_none(), "unexpected {field}");
    }
}

#[arkavo_test_macros::spec("ASTRA-001")]
#[test]
fn tools_are_flattened_without_losing_optional_mcp_parameters() {
    for tool in [
        json!({"type":"function","function":{"name":"read","parameters":{"type":"object","properties":{"path":{"type":"string"}}}}}),
        json!({"name":"read","input_schema":{"type":"object","properties":{"path":{"type":"string"}}}}),
    ] {
        let body = request(
            &OpenAIResponsesConfig::default(),
            vec![],
            Some(json!([tool])),
            None,
            None,
            false,
        )
        .unwrap();
        assert_eq!(body["tools"][0]["name"], "read");
        assert_eq!(body["tools"][0]["strict"], false);
        assert_eq!(
            body["tools"][0]["parameters"]["properties"]["path"]["type"],
            "string"
        );
        assert!(body["tools"][0].get("function").is_none());
    }
}

#[arkavo_test_macros::spec("ASTRA-002")]
#[test]
fn reasoning_and_parallel_call_ids_roundtrip_without_duplicate_calls() {
    let output = json!([
        {"type":"reasoning","id":"rs_1","encrypted_content":"opaque-state","summary":[]},
        {"type":"function_call","id":"fc_a","call_id":"call_a","name":"read","arguments":"{\"path\":\"a\"}"},
        {"type":"function_call","id":"fc_b","call_id":"call_b","name":"read","arguments":"{\"path\":\"b\"}"}
    ]);
    let result = response(completed(output.clone())).unwrap();
    assert!(result.content.is_empty());
    assert!(result.reasoning_content.is_none());
    assert_eq!(result.tool_calls[0].call_id.as_deref(), Some("call_a"));
    assert_eq!(result.tool_calls[1].arguments["path"], "b");
    let assistant = result.as_assistant_message();
    let serialized = serde_json::to_vec(&assistant).unwrap();
    let assistant: Message = serde_json::from_slice(&serialized).unwrap();
    let body = request(
        &OpenAIResponsesConfig::default(),
        vec![
            assistant,
            Message::tool_result("a", "call_a", "read"),
            Message::tool_result("b", "call_b", "read"),
        ],
        None,
        None,
        None,
        false,
    )
    .unwrap();
    assert_eq!(body["input"].as_array().unwrap().len(), 5);
    for i in 0..3 {
        assert_eq!(body["input"][i], output[i]);
    }
    assert_eq!(body["input"][3]["call_id"], "call_a");
    assert_eq!(body["input"][3]["type"], "function_call_output");
}

#[arkavo_test_macros::spec("ASTRA-001")]
#[test]
fn usage_subsets_are_not_double_counted() {
    let result = response(completed(json!([]))).unwrap();
    let timing = result.inference_timing.unwrap();
    assert_eq!(timing.n_prompt_eval, 100);
    assert_eq!(timing.n_cached_prompt_eval, Some(75));
    assert_eq!(timing.n_cache_write_prompt_eval, Some(10));
    assert_eq!(timing.n_eval, 20);
    assert_eq!(timing.n_thinking_eval, Some(40));
}

#[arkavo_test_macros::spec("ASTRA-001")]
#[test]
fn refusal_incomplete_invalid_calls_and_usage_fail_visibly() {
    for output in [
        json!([{"type":"message","content":[{"type":"refusal","refusal":"no"}]}]),
        json!([{"type":"function_call","name":"read","arguments":"invalid","call_id":"call_1"}]),
        json!([{"type":"function_call","name":"read","arguments":"[]","call_id":"call_1"}]),
        json!([{"type":"function_call","name":"read","arguments":"{}"}]),
    ] {
        assert!(response(completed(output)).is_err());
    }
    let mut incomplete = completed(json!([]));
    incomplete["status"] = json!("incomplete");
    assert!(response(incomplete).is_err());
    let mut inconsistent = completed(json!([]));
    inconsistent["usage"]["output_tokens"] = json!(1);
    assert!(response(inconsistent).is_err());
}

#[arkavo_test_macros::spec("ASTRA-002")]
#[test]
fn replay_without_call_ids_fails_before_network() {
    let assistant = Message::assistant_with_tool_calls(
        "",
        vec![ToolCall {
            name: "read".into(),
            arguments: "{}".into(),
            id: None,
        }],
    );
    assert!(
        request(
            &OpenAIResponsesConfig::default(),
            vec![assistant],
            None,
            None,
            None,
            false
        )
        .is_err()
    );
}

#[arkavo_test_macros::spec("ASTRA-001")]
#[test]
fn schema_uses_responses_text_format() {
    let body = request(&OpenAIResponsesConfig::default(), vec![], None, Some(json!({"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"]})), None, false).unwrap();
    assert_eq!(body["text"]["format"]["type"], "json_schema");
    assert_eq!(body["text"]["format"]["strict"], true);
    assert_eq!(
        body["text"]["format"]["schema"]["additionalProperties"],
        false
    );
    assert!(body.get("response_format").is_none());
}

#[arkavo_test_macros::spec("ASTRA-001")]
#[test]
fn only_documented_efforts_and_secure_configuration_are_accepted() {
    for effort in ["low", "medium", "high", "xhigh", "max"] {
        assert!(
            serde_json::from_value::<super::super::OpenAIReasoningEffort>(json!(effort)).is_ok()
        );
    }
    for effort in ["none", "ultra"] {
        assert!(
            serde_json::from_value::<super::super::OpenAIReasoningEffort>(json!(effort)).is_err()
        );
    }
    let mut config = OpenAIResponsesConfig {
        api_key: Some("test-credential".into()),
        ..Default::default()
    };
    assert!(!format!("{config:?}").contains("test-credential"));
    for url in [
        "http://api.openai.com/v1",
        "https://user:pass@api.openai.com/v1",
        "https://api.openai.com/v1?key=bad",
    ] {
        config.base_url = url.into();
        assert!(config.validate().is_err());
    }
}

#[arkavo_test_macros::spec("ASTRA-001")]
#[test]
fn incomplete_and_refusal_retain_billable_usage() {
    let mut value = completed(json!([]));
    value["status"] = json!("incomplete");
    let error = response(value).unwrap_err();
    assert_eq!(error.inference_timing().unwrap().n_prompt_eval, 100);
    let refused =
        completed(json!([{"type":"message","content":[{"type":"refusal","refusal":"no"}]}]));
    let error = response(refused).unwrap_err();
    assert_eq!(error.inference_timing().unwrap().n_thinking_eval, Some(40));
}

#[arkavo_test_macros::spec("ASTRA-002")]
#[test]
fn opaque_state_is_persisted_but_not_debug_logged() {
    let result = response(completed(
        json!([{"type":"reasoning","encrypted_content":"opaque-state-canary","summary":[]}]),
    ))
    .unwrap();
    let message = result.as_assistant_message();
    assert!(!format!("{result:?}").contains("opaque-state-canary"));
    assert!(!format!("{message:?}").contains("opaque-state-canary"));
    assert!(
        serde_json::to_string(&message)
            .unwrap()
            .contains("opaque-state-canary")
    );
}

#[arkavo_test_macros::spec("ASTRA-002")]
#[test]
fn historical_assistant_text_uses_easy_input_message() {
    let body = request(
        &OpenAIResponsesConfig::default(),
        vec![Message::assistant("previous answer")],
        None,
        None,
        None,
        false,
    )
    .unwrap();
    assert_eq!(
        body["input"][0],
        json!({"role":"assistant","content":"previous answer"})
    );
}

#[arkavo_test_macros::spec("ASTRA-001")]
#[test]
fn raw_png_images_retain_their_actual_media_type() {
    let encoded = crate::encode_image_bytes(b"\x89PNG\r\n\x1a\n").unwrap();
    let body = request(
        &OpenAIResponsesConfig::default(),
        vec![Message::user_with_images("describe", vec![encoded.clone()])],
        None,
        None,
        None,
        false,
    )
    .unwrap();
    assert_eq!(body["input"][0]["content"][1]["type"], "input_image");
    assert_eq!(
        body["input"][0]["content"][1]["image_url"],
        format!("data:image/png;base64,{encoded}")
    );
}
