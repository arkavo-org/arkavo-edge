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
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            images: None,
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            images: None,
            tool_call_id: None,
            tool_name: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            images: None,
            tool_call_id: None,
            tool_name: None,
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
        }
    }

    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            images: Some(images),
            tool_call_id: None,
            tool_name: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_user_with_images() {
        let images = vec!["base64image1".to_string(), "base64image2".to_string()];
        let msg = Message::user_with_images("Describe these images", images.clone());

        assert_eq!(msg.role, Role::User);
        assert_eq!(msg.content, "Describe these images");
        assert_eq!(msg.images, Some(images));
    }

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
    }
}
