use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    #[default]
    User,
    Assistant,
    Tool,
}

/// A tool call issued by the assistant in a single turn.
///
/// Carried on assistant messages so chat templates can render a faithful call
/// block (real name + arguments) instead of reconstructing it from the
/// following tool results, which only know the call id and lose the arguments.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ToolCall {
    pub name: String,
    /// Arguments as a JSON string ("{}" when the call took none).
    pub arguments: String,
    /// Provider-assigned call id, paired with the tool result's tool_call_id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
    /// Tool call ID for tool result messages (role=Tool)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// Tool name for tool result messages (role=Tool)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Tool calls the assistant issued this turn (role=Assistant). Carried so
    /// downstream chat templates render a faithful call block rather than
    /// reconstructing one with empty arguments from the following tool results.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<ToolCall>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }
    }

    /// Create an assistant message that records the tool calls it issued.
    /// The calls carry real arguments so chat templates can render a faithful
    /// call block; tool result messages later pair to them by call id.
    pub fn assistant_with_tool_calls(
        content: impl Into<String>,
        tool_calls: Vec<ToolCall>,
    ) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            images: None,
            tool_call_id: None,
            tool_name: None,
            tool_calls,
        }
    }

    /// Create a tool result message with call ID and tool name.
    /// Jinja templates expect role="tool" with these fields to maintain
    /// proper conversation alternation.
    pub fn tool_result(
        content: impl Into<String>,
        call_id: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self {
            role: Role::Tool,
            content: content.into(),
            images: None,
            tool_call_id: Some(call_id.into()),
            tool_name: Some(name.into()),
            tool_calls: Vec::new(),
        }
    }

    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            images: Some(images),
            tool_call_id: None,
            tool_name: None,
            tool_calls: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[test]
    fn test_message_creation_with_string_literals() {
        let msg = Message::system("test");
        assert_eq!(msg.role, Role::System);
        assert_eq!(msg.content, "test");
    }

    #[test]
    fn test_message_creation_with_string() {
        let content = String::from("dynamic content");
        let msg = Message::user(content.clone());
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, content);
    }

    #[test]
    fn test_message_with_empty_content() {
        let msg = Message::assistant("");
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.content, "");
    }

    #[test]
    fn test_message_with_special_characters() {
        let content = "Hello\nWorld\t🌍\r\n";
        let msg = Message::user(content);
        assert_eq!(msg.content, content);
    }

    #[test]
    fn test_message_with_unicode() {
        let content = "你好世界 مرحبا بالعالم";
        let msg = Message::system(content);
        assert_eq!(msg.content, content);
    }

    #[test]
    fn test_message_clone() {
        let original = Message::user("test");
        let cloned = original.clone();
        assert_eq!(original.role, cloned.role);
        assert_eq!(original.content, cloned.content);
    }

    #[test]
    fn test_role_serialization() {
        let msg = Message::system("test");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""role":"system"#));

        let msg = Message::user("test");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""role":"user"#));

        let msg = Message::assistant("test");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""role":"assistant"#));
    }

    #[test]
    fn test_message_deserialization() {
        let json = r#"{"role":"user","content":"Hello"}"#;
        let msg: Message = serde_json::from_str(json).unwrap();
        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "Hello");
    }

    #[spec("ROUTER-015")]
    #[test]
    fn test_user_with_images() {
        let images = vec!["base64image1".to_string(), "base64image2".to_string()];
        let msg = Message::user_with_images("Describe these images", images.clone());

        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "Describe these images");
        assert_eq!(msg.images, Some(images));
    }

    #[spec("ROUTER-015")]
    #[test]
    fn test_message_with_images_serialization() {
        let msg = Message::user_with_images("Test", vec!["image123".to_string()]);
        let json = serde_json::to_string(&msg).unwrap();

        assert!(json.contains(r#""role":"user"#));
        assert!(json.contains(r#""content":"Test"#));
        assert!(json.contains(r#""images":["image123"]"#));
    }

    #[test]
    fn test_message_without_images_serialization() {
        let msg = Message::user("Test without images");
        let json = serde_json::to_string(&msg).unwrap();

        assert!(json.contains(r#""role":"user"#));
        assert!(json.contains(r#""content":"Test without images"#));
        assert!(!json.contains(r#""images""#));
    }

    #[spec("ROUTER-015")]
    #[test]
    fn test_message_with_images_deserialization() {
        let json = r#"{"role":"user","content":"Test","images":["img1","img2"]}"#;
        let msg: Message = serde_json::from_str(json).unwrap();

        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "Test");
        assert_eq!(
            msg.images,
            Some(vec!["img1".to_string(), "img2".to_string()])
        );
    }

    #[test]
    fn test_tool_result_message() {
        let msg = Message::tool_result(r#"{"result": "success"}"#, "call_123", "get_time");
        assert_eq!(msg.role, Role::Tool);
        assert_eq!(msg.tool_call_id, Some("call_123".to_string()));
        assert_eq!(msg.tool_name, Some("get_time".to_string()));
    }

    #[test]
    fn test_tool_role_serialization() {
        let msg = Message::tool_result("result", "id1", "tool1");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""role":"tool"#));
        assert!(json.contains(r#""tool_call_id":"id1"#));
        assert!(json.contains(r#""tool_name":"tool1"#));
    }

    #[test]
    fn test_non_tool_messages_omit_tool_fields() {
        let msg = Message::user("test");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(!json.contains("tool_call_id"));
        assert!(!json.contains("tool_name"));
        assert!(!json.contains("tool_calls"));
    }

    #[test]
    fn test_assistant_with_tool_calls() {
        let msg = Message::assistant_with_tool_calls(
            "checking the weather",
            vec![ToolCall {
                name: "get_weather".to_string(),
                arguments: r#"{"location":"Paris"}"#.to_string(),
                id: Some("call_0".to_string()),
            }],
        );
        assert_eq!(msg.role, Role::Assistant);
        assert_eq!(msg.tool_calls.len(), 1);
        assert_eq!(msg.tool_calls[0].name, "get_weather");
        assert_eq!(msg.tool_calls[0].arguments, r#"{"location":"Paris"}"#);
    }

    #[test]
    fn test_tool_calls_roundtrip_serialization() {
        let msg = Message::assistant_with_tool_calls(
            "",
            vec![ToolCall {
                name: "get_time".to_string(),
                arguments: "{}".to_string(),
                id: None,
            }],
        );
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""tool_calls""#));
        // Id is omitted when absent.
        assert!(!json.contains(r#""id""#));
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back.tool_calls, msg.tool_calls);
    }
}
