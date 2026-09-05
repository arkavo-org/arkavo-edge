use super::OpenAIResponsesConfig;
use crate::provider::InferenceTiming;
use crate::tool_parser::ParsedToolCall;
use crate::{Error, Message, ProviderResponse, Result, Role};
use serde_json::{Value, json};

pub(super) fn request(
    config: &OpenAIResponsesConfig,
    messages: Vec<Message>,
    tools: Option<Value>,
    schema: Option<Value>,
    max_tokens: Option<usize>,
    stream: bool,
) -> Result<Value> {
    let max_tokens = max_tokens.unwrap_or(config.max_output_tokens);
    if max_tokens == 0 || max_tokens > 128_000 {
        return Err(Error::Config(
            "Astra output token limit must be 1..=128000".into(),
        ));
    }
    let mut input = Vec::new();
    for message in messages {
        if message.role == Role::Assistant && !message.response_items.is_empty() {
            // Replay the provider's ordered output exactly, including encrypted
            // reasoning and call IDs. Adding reconstructed calls would duplicate them.
            input.extend(message.response_items);
            continue;
        }
        if message.role == Role::Tool {
            let call_id = message
                .tool_call_id
                .filter(|id| !id.is_empty())
                .ok_or_else(|| Error::Config("Responses tool result requires a call ID".into()))?;
            input.push(
                json!({"type":"function_call_output", "call_id":call_id, "output":message.content}),
            );
            continue;
        }
        let role = match message.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => unreachable!(),
        };
        if !message.content.is_empty() || message.images.as_ref().is_some_and(|v| !v.is_empty()) {
            let mut content = vec![json!({"type":"input_text", "text":message.content})];
            for image in message.images.unwrap_or_default() {
                if role != "user" {
                    return Err(Error::Config(
                        "Responses images require a user message".into(),
                    ));
                }
                let image_url = if image.starts_with("https://") || image.starts_with("data:") {
                    image
                } else {
                    let bytes = crate::decode_image(&image)?;
                    let mime = match crate::ImageFormat::validate_bytes(&bytes)? {
                        crate::ImageFormat::Png => "image/png",
                        crate::ImageFormat::Jpeg => "image/jpeg",
                        crate::ImageFormat::WebP => "image/webp",
                    };
                    format!("data:{mime};base64,{image}")
                };
                content.push(json!({"type":"input_image", "image_url":image_url}));
            }
            if role == "assistant" {
                // Historical assistant text is an easy input message, not a
                // fabricated output_text item lacking provider annotations/IDs.
                input.push(json!({"role":role,"content":message.content}));
            } else {
                input.push(json!({"role":role,"content":content}));
            }
        }
        for call in message.tool_calls {
            if role != "assistant" {
                return Err(Error::Config(
                    "Responses function calls require an assistant message".into(),
                ));
            }
            let id = call.id.filter(|id| !id.is_empty()).ok_or_else(|| {
                Error::Config("Responses function call requires a call ID".into())
            })?;
            input.push(json!({"type":"function_call", "call_id":id, "name":call.name,"arguments":call.arguments}));
        }
    }
    let mut body = json!({
        "model":config.model, "input":input, "store":false, "stream":stream,
        "include":["reasoning.encrypted_content"],
        "reasoning":{"effort":config.reasoning_effort}, "max_output_tokens":max_tokens
    });
    if let Some(tools) = tools {
        body["tools"] = Value::Array(convert_tools(tools)?);
    }
    if let Some(schema) = schema {
        body["text"] = json!({"format":{"type":"json_schema", "name":"response", "strict":true, "schema":super::schema::strict(schema)}});
    }
    Ok(body)
}

fn convert_tools(tools: Value) -> Result<Vec<Value>> {
    let tools = tools
        .as_array()
        .ok_or_else(|| Error::Config("Responses tools must be an array".into()))?;
    tools.iter().map(|tool| {
        if tool.get("type").is_some_and(|kind| kind != "function") {
            return Err(Error::Config("This harness supports Responses function tools only".into()));
        }
        let function = tool.get("function").unwrap_or(tool);
        let name = function["name"].as_str().filter(|v| !v.is_empty())
            .ok_or_else(|| Error::Config("Function tool requires a name".into()))?;
        // Explicitly opt out of Responses' implicit strictification: MCP schemas
        // may have optional parameters. Structured output remains strict separately.
        Ok(json!({"type":"function", "name":name,
            "description":function["description"].as_str().unwrap_or(""),
            "parameters":function.get("parameters").or_else(|| function.get("input_schema")).cloned().unwrap_or_else(|| json!({"type":"object","properties":{}})),
            "strict":false}))
    }).collect()
}

pub(super) fn response(value: Value) -> Result<ProviderResponse> {
    let usage = value
        .get("usage")
        .filter(|u| !u.is_null())
        .map(timing)
        .transpose()?;
    parse_response(&value)
        .map(|mut response| {
            response.inference_timing.clone_from(&usage);
            response
        })
        .map_err(|error| Error::ProviderResponseFailure {
            message: error.to_string(),
            inference_timing: usage,
        })
}

fn parse_response(value: &Value) -> Result<ProviderResponse> {
    if value["status"] != "completed" {
        return Err(Error::Provider(
            "OpenAI Responses did not complete successfully".into(),
        ));
    }
    let output = value["output"]
        .as_array()
        .ok_or_else(|| Error::Provider("OpenAI Responses is missing output items".into()))?;
    let mut result = ProviderResponse::default();
    for item in output {
        match item["type"].as_str() {
            Some("message") => {
                let content = item["content"].as_array().ok_or_else(|| {
                    Error::Provider("Responses message is missing content".into())
                })?;
                for part in content {
                    match part["type"].as_str() {
                        Some("output_text") => result.content.push_str(required_str(part, "text")?),
                        Some("refusal") => {
                            return Err(Error::Provider("OpenAI refused the request".into()));
                        }
                        _ => {
                            return Err(Error::Provider(
                                "Unsupported Responses message content".into(),
                            ));
                        }
                    }
                }
            }
            Some("function_call") => {
                let arguments: Value = serde_json::from_str(required_str(item, "arguments")?)?;
                if !arguments.is_object() {
                    return Err(Error::Provider(
                        "Responses function arguments must be an object".into(),
                    ));
                }
                result.tool_calls.push(ParsedToolCall {
                    tool_name: required_str(item, "name")?.into(),
                    call_id: Some(required_str(item, "call_id")?.into()),
                    arguments,
                });
            }
            Some("reasoning") => {} // Opaque encrypted content must never become visible text.
            _ => {
                return Err(Error::Provider(
                    "Unsupported OpenAI Responses output item".into(),
                ));
            }
        }
    }
    result.response_items.clone_from(output);
    result.finish_reason = Some(
        if result.tool_calls.is_empty() {
            "stop"
        } else {
            "tool_calls"
        }
        .into(),
    );
    result.inference_timing = value
        .get("usage")
        .filter(|u| !u.is_null())
        .map(timing)
        .transpose()?;
    Ok(result)
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| Error::Provider(format!("Responses item is missing {key}")))
}

fn timing(usage: &Value) -> Result<InferenceTiming> {
    let count = |value: &Value| -> Result<u32> {
        value
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| Error::Provider("Invalid Responses usage count".into()))
    };
    let input = count(&usage["input_tokens"])?;
    let output = count(&usage["output_tokens"])?;
    let cached = usage
        .pointer("/input_tokens_details/cached_tokens")
        .map(count)
        .transpose()?
        .unwrap_or(0);
    let thinking = usage
        .pointer("/output_tokens_details/reasoning_tokens")
        .map(count)
        .transpose()?
        .unwrap_or(0);
    let cache_write = usage
        .pointer("/input_tokens_details/cache_write_tokens")
        .map(count)
        .transpose()?
        .unwrap_or(0);
    if cached > input || cache_write > input.saturating_sub(cached) || thinking > output {
        return Err(Error::Provider(
            "Inconsistent Responses usage counts".into(),
        ));
    }
    Ok(InferenceTiming {
        n_prompt_eval: input,
        n_cached_prompt_eval: Some(cached),
        n_cache_write_prompt_eval: Some(cache_write),
        n_eval: output - thinking,
        n_thinking_eval: Some(thinking),
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod tests;
