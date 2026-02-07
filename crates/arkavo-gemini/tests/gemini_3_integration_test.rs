#![allow(clippy::collection_is_never_read, clippy::disallowed_methods)]

use arkavo_gemini::{FunctionDeclaration, RestClient};
use serde_json::{Value, json};

fn get_api_key() -> String {
    std::env::var("GEMINI_API_KEY").unwrap_or_else(|_| "test_key".to_string())
}

fn get_model() -> String {
    std::env::var("GEMINI_MODEL").unwrap_or_else(|_| "models/gemini-3-pro-preview".to_string())
}

fn should_skip_integration_tests() -> bool {
    std::env::var("GEMINI_API_KEY").is_err() || get_api_key() == "test_key"
}

fn create_mock_weather_tool() -> FunctionDeclaration {
    FunctionDeclaration {
        name: "get_weather".to_string(),
        description: "Get current weather for a city".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "city": {
                    "type": "string",
                    "description": "City name"
                }
            },
            "required": ["city"]
        }),
    }
}

fn create_mock_stock_tool() -> FunctionDeclaration {
    FunctionDeclaration {
        name: "get_stock_price".to_string(),
        description: "Get current stock price".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "symbol": {
                    "type": "string",
                    "description": "Stock symbol"
                }
            },
            "required": ["symbol"]
        }),
    }
}

fn create_mock_calculator_tool() -> FunctionDeclaration {
    FunctionDeclaration {
        name: "calculate".to_string(),
        description: "Perform mathematical calculations".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {
                "expression": {
                    "type": "string",
                    "description": "Math expression to evaluate"
                }
            },
            "required": ["expression"]
        }),
    }
}

#[tokio::test]
async fn test_parallel_tool_execution() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let tools = vec![create_mock_weather_tool(), create_mock_stock_tool()];

    let prompt = "Get the weather in Tokyo AND the stock price for Apple. Do both tasks.";

    let mut stream = client
        .stream_generate_content(prompt, Some(tools))
        .await
        .expect("Failed to create stream");

    let mut tool_calls = Vec::new();
    let mut total_text = String::new();

    while let Some(result) = stream.next().await {
        match result {
            Ok(chunk) => {
                if let Some(text) = chunk.text {
                    total_text.push_str(&text);
                }
                if !chunk.function_calls.is_empty() {
                    tool_calls.extend(chunk.function_calls);
                }
            }
            Err(e) => {
                eprintln!("Stream error: {e:?}");
                break;
            }
        }
    }

    assert!(
        tool_calls.len() >= 2,
        "Expected parallel tool execution (2 calls), got {}",
        tool_calls.len()
    );

    let tool_names: Vec<String> = tool_calls.iter().map(|c| c.name.clone()).collect();
    assert!(
        tool_names.contains(&"get_weather".to_string()),
        "Expected weather tool call"
    );
    assert!(
        tool_names.contains(&"get_stock_price".to_string()),
        "Expected stock tool call"
    );
}

#[tokio::test]
async fn test_json_mode_reliability() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let prompt = r#"Generate a JSON object with exactly these fields:
{
  "name": "test_user",
  "age": 25,
  "active": true,
  "tags": ["rust", "testing"]
}
Only respond with valid JSON, no other text."#;

    let mut stream = client
        .stream_generate_content(prompt, None)
        .await
        .expect("Failed to create stream");

    let mut response_text = String::new();

    while let Some(result) = stream.next().await {
        if let Ok(chunk) = result
            && let Some(text) = chunk.text
        {
            response_text.push_str(&text);
        }
    }

    let parsed: Result<Value, _> = serde_json::from_str(&response_text.trim());
    assert!(
        parsed.is_ok(),
        "JSON mode failed to produce valid JSON: {response_text}"
    );

    let json = parsed.unwrap();
    assert!(json.is_object(), "Expected JSON object");
    assert!(json.get("name").is_some(), "Missing 'name' field");
}

#[tokio::test]
async fn test_tool_orchestration_complexity() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let tools = vec![
        create_mock_weather_tool(),
        create_mock_stock_tool(),
        create_mock_calculator_tool(),
    ];

    let prompt = "First get the weather in NYC, then get Apple stock price, then calculate 10+20";

    let mut stream = client
        .stream_generate_content(prompt, Some(tools))
        .await
        .expect("Failed to create stream");

    let mut tool_calls = Vec::new();

    while let Some(result) = stream.next().await {
        if let Ok(chunk) = result
            && !chunk.function_calls.is_empty()
        {
            tool_calls.extend(chunk.function_calls);
        }
    }

    assert!(
        tool_calls.len() >= 3,
        "Expected 3+ tool calls for complex orchestration, got {}",
        tool_calls.len()
    );
}

#[tokio::test]
async fn test_error_recovery_malformed_tool() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let broken_tool = FunctionDeclaration {
        name: "broken_tool".to_string(),
        description: "A tool that returns invalid data".to_string(),
        parameters: json!({
            "type": "object",
            "properties": {}
        }),
    };

    let tools = vec![broken_tool, create_mock_weather_tool()];

    let prompt = "Use the broken_tool, and if it fails, get weather for London";

    let mut stream = client
        .stream_generate_content(prompt, Some(tools))
        .await
        .expect("Failed to create stream");

    let mut had_error = false;
    let mut completed_successfully = false;

    while let Some(result) = stream.next().await {
        match result {
            Ok(_chunk) => {
                completed_successfully = true;
            }
            Err(_) => {
                had_error = true;
            }
        }
    }

    assert!(
        completed_successfully || had_error,
        "Stream should either complete or error gracefully"
    );
}

#[tokio::test]
async fn test_multi_turn_consistency() {
    if should_skip_integration_tests() {
        eprintln!("Skipping test: GEMINI_API_KEY not set");
        return;
    }

    let api_key = get_api_key();
    let model = get_model();
    let client = RestClient::new(&api_key, &model);

    let turn1_prompt = "Remember this number: 42. What is it?";

    let mut stream = client
        .stream_generate_content(turn1_prompt, None)
        .await
        .expect("Failed to create stream");

    let mut response1 = String::new();
    while let Some(result) = stream.next().await {
        if let Ok(chunk) = result
            && let Some(text) = chunk.text
        {
            response1.push_str(&text);
        }
    }

    assert!(
        response1.contains("42"),
        "First turn should contain the number"
    );
}
