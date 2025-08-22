use crate::error::{DeepSeekError, Result};
use crate::types::{
    AnthropicContent, AnthropicContentPart, AnthropicMessage, AnthropicMessageRequest,
    AnthropicTool, AnthropicToolChoice, ChatMessage, ContentPart, FunctionCall, MessageContent,
    ResponseFormat, Role, Tool, ToolCall, ToolChoice, ToolFunction,
};
use serde_json::Value;

type ConvertedRequest = (Vec<ChatMessage>, Option<Vec<Tool>>, Option<ToolChoice>);

/// Convert Anthropic message request to DeepSeek format
pub fn convert_anthropic_request(request: AnthropicMessageRequest) -> Result<ConvertedRequest> {
    let mut messages = Vec::new();

    // Handle system message
    if let Some(system) = request.system {
        messages.push(ChatMessage {
            role: Role::System,
            content: MessageContent::Text { content: system },
            name: None,
            tool_calls: None,
            tool_call_id: None,
        });
    }

    // Convert messages
    for msg in request.messages {
        messages.push(convert_anthropic_message(msg)?);
    }

    // Convert tools
    let tools = request
        .tools
        .map(|tools| tools.into_iter().map(convert_anthropic_tool).collect());

    // Convert tool choice
    let tool_choice = request.tool_choice.and_then(convert_anthropic_tool_choice);

    Ok((messages, tools, tool_choice))
}

/// Convert Anthropic message to DeepSeek format
fn convert_anthropic_message(msg: AnthropicMessage) -> Result<ChatMessage> {
    let role = match msg.role.as_str() {
        "user" => Role::User,
        "assistant" => Role::Assistant,
        "system" => Role::System,
        _ => {
            return Err(DeepSeekError::InvalidRequest {
                message: format!("Unknown role: {}", msg.role),
            });
        }
    };

    let (content, tool_calls, tool_call_id) = match msg.content {
        AnthropicContent::Text { content } => (MessageContent::Text { content }, None, None),
        AnthropicContent::Parts { content: parts } => {
            let mut text_parts = Vec::new();
            let mut tool_uses = Vec::new();
            let mut tool_result_id = None;

            for part in parts {
                match part {
                    AnthropicContentPart::Text { text } => {
                        text_parts.push(ContentPart::Text { text });
                    }
                    AnthropicContentPart::ToolUse { id, name, input } => {
                        // Convert tool use to tool call
                        let arguments = serde_json::to_string(&input).map_err(|e| {
                            DeepSeekError::InvalidRequest {
                                message: format!("Failed to serialize tool input: {e}"),
                            }
                        })?;

                        tool_uses.push(ToolCall {
                            id,
                            tool_type: "function".to_string(),
                            function: FunctionCall { name, arguments },
                        });
                    }
                    AnthropicContentPart::ToolResult {
                        tool_use_id,
                        content,
                    } => {
                        // Tool results become tool messages
                        tool_result_id = Some(tool_use_id);
                        text_parts.push(ContentPart::Text { text: content });
                    }
                }
            }

            // If we have tool uses, those become tool_calls
            let tool_calls = if !tool_uses.is_empty() {
                Some(tool_uses)
            } else {
                None
            };

            // Construct content
            let content = if text_parts.len() == 1 {
                if let ContentPart::Text { text } = &text_parts[0] {
                    MessageContent::Text {
                        content: text.clone(),
                    }
                } else {
                    MessageContent::Parts {
                        content: text_parts,
                    }
                }
            } else if !text_parts.is_empty() {
                MessageContent::Parts {
                    content: text_parts,
                }
            } else {
                MessageContent::Text {
                    content: String::new(),
                }
            };

            (content, tool_calls, tool_result_id)
        }
    };

    // If this is a tool result, change role to Tool
    let final_role = if tool_call_id.is_some() {
        Role::Tool
    } else {
        role
    };

    Ok(ChatMessage {
        role: final_role,
        content,
        name: None,
        tool_calls,
        tool_call_id,
    })
}

/// Convert Anthropic tool to DeepSeek format
fn convert_anthropic_tool(tool: AnthropicTool) -> Tool {
    Tool {
        tool_type: "function".to_string(),
        function: ToolFunction {
            name: tool.name,
            description: tool.description,
            parameters: tool.input_schema,
        },
        strict: None,
    }
}

/// Convert Anthropic tool choice to DeepSeek format
fn convert_anthropic_tool_choice(choice: AnthropicToolChoice) -> Option<ToolChoice> {
    match choice {
        AnthropicToolChoice::Auto => Some(ToolChoice::Auto),
        AnthropicToolChoice::None => Some(ToolChoice::None),
        AnthropicToolChoice::Any => Some(ToolChoice::Any),
        AnthropicToolChoice::Tool { name, .. } => Some(ToolChoice::Tool {
            r#type: "function".to_string(),
            function: crate::types::ToolChoiceFunction { name },
        }),
    }
}

/// Check if content contains unsupported Anthropic message parts
pub fn check_unsupported_content(content: &Value) -> Result<()> {
    if let Some(parts) = content.as_array() {
        for part in parts {
            if let Some(part_type) = part.get("type").and_then(|t| t.as_str()) {
                match part_type {
                    "text" | "thinking" | "tool_use" | "tool_result" => {}
                    "image" => {
                        return Err(DeepSeekError::UnsupportedMessagePart {
                            message: "Image content is not supported in DeepSeek's Anthropic mode"
                                .to_string(),
                            part_type: "image".to_string(),
                        });
                    }
                    "document" => {
                        return Err(DeepSeekError::UnsupportedMessagePart {
                            message:
                                "Document content is not supported in DeepSeek's Anthropic mode"
                                    .to_string(),
                            part_type: "document".to_string(),
                        });
                    }
                    "search_result" => return Err(DeepSeekError::UnsupportedMessagePart {
                        message:
                            "Search result content is not supported in DeepSeek's Anthropic mode"
                                .to_string(),
                        part_type: "search_result".to_string(),
                    }),
                    "redacted_thinking" => {
                        return Err(DeepSeekError::UnsupportedMessagePart {
                            message:
                                "Redacted thinking is not supported in DeepSeek's Anthropic mode"
                                    .to_string(),
                            part_type: "redacted_thinking".to_string(),
                        });
                    }
                    "server_tool_use" => {
                        return Err(DeepSeekError::UnsupportedMessagePart {
                            message:
                                "Server tool use is not supported in DeepSeek's Anthropic mode"
                                    .to_string(),
                            part_type: "server_tool_use".to_string(),
                        });
                    }
                    unknown => {
                        return Err(DeepSeekError::UnsupportedMessagePart {
                            message: format!("Unknown content type '{unknown}' is not supported"),
                            part_type: unknown.to_string(),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// Check if response format requires JSON instruction in prompt
pub fn needs_json_instruction(response_format: &Option<ResponseFormat>) -> bool {
    matches!(response_format, Some(ResponseFormat::JsonObject))
}

/// Add JSON instruction to the last user message if needed
pub fn add_json_instruction_if_needed(
    messages: &mut [ChatMessage],
    response_format: &Option<ResponseFormat>,
) {
    if !needs_json_instruction(response_format) {
        return;
    }

    // Find the last user message and append JSON instruction
    for msg in messages.iter_mut().rev() {
        if msg.role == Role::User {
            let instruction = "\n\nPlease respond with valid JSON.";
            match &mut msg.content {
                MessageContent::Text { content } => {
                    content.push_str(instruction);
                }
                MessageContent::Parts { content } => {
                    content.push(ContentPart::Text {
                        text: instruction.to_string(),
                    });
                }
            }
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_convert_simple_anthropic_message() {
        let msg = AnthropicMessage {
            role: "user".to_string(),
            content: AnthropicContent::Text {
                content: "Hello".to_string(),
            },
        };

        let result = convert_anthropic_message(msg).unwrap();
        assert_eq!(result.role, Role::User);
        match result.content {
            MessageContent::Text { content } => assert_eq!(content, "Hello"),
            _ => panic!("Expected text content"),
        }
    }

    #[test]
    fn test_check_unsupported_image_content() {
        let content = json!([
            {"type": "text", "text": "Look at this image:"},
            {"type": "image", "source": {"type": "base64", "data": "..."}}
        ]);

        let result = check_unsupported_content(&content);
        assert!(result.is_err());
        if let Err(DeepSeekError::UnsupportedMessagePart { part_type, .. }) = result {
            assert_eq!(part_type, "image");
        } else {
            panic!("Expected UnsupportedMessagePart error");
        }
    }

    #[test]
    fn test_add_json_instruction() {
        let mut messages = vec![ChatMessage {
            role: Role::User,
            content: MessageContent::Text {
                content: "What is 2+2?".to_string(),
            },
            name: None,
            tool_calls: None,
            tool_call_id: None,
        }];

        let response_format = Some(ResponseFormat::JsonObject);
        add_json_instruction_if_needed(&mut messages, &response_format);

        match &messages[0].content {
            MessageContent::Text { content } => {
                assert!(content.contains("Please respond with valid JSON"));
            }
            _ => panic!("Expected text content"),
        }
    }
}
