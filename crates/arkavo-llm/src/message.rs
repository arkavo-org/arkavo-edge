use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
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
}