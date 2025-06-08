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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub images: Option<Vec<String>>,
}

impl Message {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
            images: None,
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            images: None,
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
            images: None,
        }
    }

    pub fn user_with_images(content: impl Into<String>, images: Vec<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
            images: Some(images),
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
}
