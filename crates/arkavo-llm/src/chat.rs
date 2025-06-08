use crate::Message;

#[derive(Debug, Clone)]
pub struct ChatRequest {
    pub content: String,
    pub images: Vec<String>,
}

impl ChatRequest {
    pub fn new(content: impl Into<String>) -> Self {
        Self {
            content: content.into(),
            images: Vec::new(),
        }
    }

    pub fn with_images(mut self, images: Vec<String>) -> Self {
        self.images = images;
        self
    }

    pub fn add_image(mut self, image: String) -> Self {
        self.images.push(image);
        self
    }

    pub fn to_message(self) -> Message {
        if self.images.is_empty() {
            Message::user(self.content)
        } else {
            Message::user_with_images(self.content, self.images)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_request_new() {
        let req = ChatRequest::new("Hello");
        assert_eq!(req.content, "Hello");
        assert!(req.images.is_empty());
    }

    #[test]
    fn test_chat_request_with_images() {
        let images = vec!["img1".to_string(), "img2".to_string()];
        let req = ChatRequest::new("Describe these")
            .with_images(images.clone());
        
        assert_eq!(req.content, "Describe these");
        assert_eq!(req.images, images);
    }

    #[test]
    fn test_chat_request_add_image() {
        let req = ChatRequest::new("Test")
            .add_image("img1".to_string())
            .add_image("img2".to_string());
        
        assert_eq!(req.images.len(), 2);
        assert_eq!(req.images[0], "img1");
        assert_eq!(req.images[1], "img2");
    }

    #[test]
    fn test_chat_request_to_message_without_images() {
        let req = ChatRequest::new("Hello");
        let msg = req.to_message();
        
        assert_eq!(msg.content, "Hello");
        assert_eq!(msg.images, None);
    }

    #[test]
    fn test_chat_request_to_message_with_images() {
        let req = ChatRequest::new("Describe")
            .with_images(vec!["img1".to_string()]);
        let msg = req.to_message();
        
        assert_eq!(msg.content, "Describe");
        assert_eq!(msg.images, Some(vec!["img1".to_string()]));
    }
}