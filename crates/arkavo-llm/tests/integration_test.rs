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
        // When llm-remote feature is enabled, client should be created successfully
        // even without environment variables (zero config)
        #[cfg(feature = "llm-remote")]
        {
            let result = LlmClient::from_env();
            assert!(result.is_ok(), "Client creation should succeed with zero config");

            let client = result.unwrap();
            assert_eq!(client.provider_name(), "ollama");
        }

        // When only llm-local feature is enabled, from_env() will fail
        // because it defaults to ollama which requires llm-remote
        #[cfg(all(not(feature = "llm-remote"), feature = "llm-local"))]
        {
            let result = LlmClient::from_env();
            assert!(result.is_err(), "Client creation should fail without llm-remote feature");
        }
    }
}
