use super::OpenAIResponsesConfig;
use crate::provider::InferenceTiming;
use crate::provider_state::{ProviderState, ProviderStateTag};
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
    super::config::check_max_tokens(max_tokens)?;
    let mut input = Vec::new();
    for message in messages {
        if message.role == Role::Assistant
            && let Some(items) = message
                .provider_state
                .replay_items_for(ProviderStateTag::OpenAiResponses)
        {
            // Replay the provider's ordered output exactly, including encrypted
            // reasoning and call IDs. Adding reconstructed calls would duplicate them.
            input.extend(items);
            continue;
        }
        let role = match message.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            // A tool result is an input item of its own, not a role: it answers
            // the call the provider recorded, by that call's id.
            Role::Tool => {
                let call_id = message
                    .tool_call_id
                    .filter(|id| !id.is_empty())
                    .ok_or_else(|| {
                        Error::Config("Responses tool result requires a call ID".into())
                    })?;
                input.push(
                    json!({"type":"function_call_output", "call_id":call_id, "output":message.content}),
                );
                continue;
            }
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

/// A tool this build cannot declare is an error, not a silent omission: the
/// model would be asked to work without a capability the caller believes it has.
fn convert_tools(tools: Value) -> Result<Vec<Value>> {
    let tools = tools
        .as_array()
        .ok_or_else(|| Error::Config("Responses tools must be an array".into()))?;
    tools
        .iter()
        .map(|tool| {
            if tool.get("type").is_some_and(|kind| kind != "function") {
                return Err(Error::Config(
                    "This harness supports Responses function tools only".into(),
                ));
            }
            let mut tool = crate::common::responses::function_tool(tool)
                .ok_or_else(|| Error::Config("Function tool requires a name".into()))?;
            // Explicitly opt out of Responses' implicit strictification: MCP schemas
            // may have optional parameters. Structured output remains strict separately.
            tool["strict"] = Value::Bool(false);
            Ok(tool)
        })
        .collect()
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
        // Attach the billed usage without flattening the failure: a structured
        // refusal has to reach the caller with its code intact.
        .map_err(|error| error.with_inference_timing(usage))
}

/// Name the reason a response did not complete.
///
/// A truncation reports `incomplete_details.reason` (`max_output_tokens`,
/// `content_filter`); a failure reports `error.code`/`error.type`. Either is a
/// better answer than the bare status, and none of them is the `message` text.
fn refusal(value: &Value) -> Error {
    let reason = value
        .pointer("/incomplete_details/reason")
        .and_then(Value::as_str);
    let code = value.pointer("/error/code").and_then(Value::as_str);
    let kind = value.pointer("/error/type").and_then(Value::as_str);
    let fallback = match value["status"].as_str() {
        Some("failed") => "response_failed",
        Some("incomplete") => "response_incomplete",
        _ => "response_not_completed",
    };
    Error::provider_refusal(
        crate::error::first_wire_code(&[reason, code, kind]),
        kind,
        fallback,
    )
}

fn parse_response(value: &Value) -> Result<ProviderResponse> {
    if value["status"] != "completed" {
        return Err(refusal(value));
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
                            return Err(Error::provider_refusal(None, None, "refusal"));
                        }
                        // A part this build does not render is still replayed
                        // verbatim through `provider_state`, so ignoring it
                        // loses nothing while a new part type would otherwise
                        // break every turn.
                        _ => {}
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
            // Unknown item types (a built-in tool, a future annotation) carry
            // nothing this build can act on, and replay preserves them exactly.
            // A malformed `function_call` above still fails: acting on half a
            // call is worse than refusing the turn.
            _ => {}
        }
    }
    result.provider_state = ProviderState::openai_responses(output.clone());
    result.finish_reason = Some(
        if result.tool_calls.is_empty() {
            "stop"
        } else {
            "tool_calls"
        }
        .into(),
    );
    // Usage is read once, by the caller, and applied to both the success and
    // the failure path.
    Ok(result)
}

fn required_str<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| Error::Provider(format!("Responses item is missing {key}")))
}

/// Read the billed usage, refusing a report that cannot be priced.
///
/// A subset larger than its total would either over-bill the caller or hide
/// spend, and this provider serves the budgeted cloud path: failing the turn is
/// the honest answer, and the caller still receives the counts with the error.
fn timing(usage: &Value) -> Result<InferenceTiming> {
    let usage = crate::common::responses::Usage::parse(usage)?;
    if !usage.is_consistent() {
        return Err(Error::Provider(
            "Inconsistent Responses usage counts".into(),
        ));
    }
    Ok(usage.timing())
}

#[cfg(test)]
#[path = "convert_tests.rs"]
mod tests;
