use crate::chat::ChatRequest;
use crate::ollama::OllamaClient;
use crate::{Error, Message, Provider, Result, StreamResponse};
use tokio_stream::Stream;

pub struct LlmClient {
    provider: Box<dyn Provider>,
}

impl LlmClient {
    pub fn new(provider: Box<dyn Provider>) -> Self {
        Self { provider }
    }

    pub fn from_env() -> Result<Self> {
        // Check for provider preference
        let provider_name = std::env::var("LLM_PROVIDER")
            .unwrap_or_else(|_| "ollama".to_string())
            .to_lowercase();

        let provider: Box<dyn Provider> = match provider_name.as_str() {
            "ollama" => Box::new(OllamaClient::from_env()?),
            _ => {
                return Err(Error::Config(format!(
                    "Unknown provider: {}",
                    provider_name
                )));
            }
        };

        Ok(Self::new(provider))
    }

    pub async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        self.provider.complete(messages).await
    }

    pub async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        self.provider.stream(messages).await
    }

    pub fn provider_name(&self) -> &str {
        self.provider.name()
    }

    pub async fn chat(&self, request: ChatRequest) -> Result<String> {
        let message = request.to_message();
        self.complete(vec![message]).await
    }

    pub async fn chat_stream(
        &self,
        request: ChatRequest,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let message = request.to_message();
        self.stream(vec![message]).await
    }
}
