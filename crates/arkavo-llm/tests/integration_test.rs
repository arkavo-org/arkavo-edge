#[cfg(test)]
mod tests {
    use arkavo_llm::{LlmClient, Message};

    #[test]
    fn test_message_creation() {
        let system_msg = Message::system("You are a helpful assistant");
        assert_eq!(system_msg.role, arkavo_llm::Role::System);
        assert_eq!(system_msg.content, "You are a helpful assistant");

        let user_msg = Message::user("Hello");
        assert_eq!(user_msg.role, arkavo_llm::Role::User);
        assert_eq!(user_msg.content, "Hello");

        let assistant_msg = Message::assistant("Hi there!");
        assert_eq!(assistant_msg.role, arkavo_llm::Role::Assistant);
        assert_eq!(assistant_msg.content, "Hi there!");
    }

    #[test]
    fn test_client_creation() {
        // Test that client can be created without panicking
        let result = LlmClient::from_env();
        assert!(result.is_ok());

        let client = result.unwrap();
        assert_eq!(client.provider_name(), "ollama");
    }
}
