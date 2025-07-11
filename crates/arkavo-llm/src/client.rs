use crate::chat::ChatRequest;
#[cfg(feature = "llm-remote")]
use crate::ollama::OllamaClient;
use crate::{Error, Message, Provider, Result, StreamResponse};
use tokio_stream::Stream;

#[cfg(feature = "llm-local")]
use crate::local::LocalProvider;

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

        match provider_name.as_str() {
            #[cfg(feature = "llm-remote")]
            "ollama" => {
                let provider = Box::new(OllamaClient::from_env()?);
                Ok(Self::new(provider))
            }
            _ => {
                #[cfg(feature = "llm-remote")]
                return Err(Error::Config(format!("Unknown provider: {provider_name}")));

                #[cfg(not(feature = "llm-remote"))]
                return Err(Error::Config(
                    "No LLM providers available. Build with 'llm-remote' or 'llm-local' feature enabled.".to_string()
                ));
            }
        }
    }

    #[cfg_attr(not(feature = "llm-local"), allow(clippy::unused_async))]
    pub async fn from_local_model(model_name: &str, model_path: String) -> Result<Self> {
        #[cfg(feature = "llm-local")]
        {
            let provider = LocalProvider::new(model_name.to_string(), Some(model_path))?;
            // Initialize the provider to load the model
            provider.initialize().await?;
            Ok(Self::new(Box::new(provider)))
        }
        #[cfg(not(feature = "llm-local"))]
        {
            let _ = (model_name, model_path); // Suppress unused variable warnings
            Err(Error::Config(
                "Local models require the 'llm-local' feature to be enabled".to_string(),
            ))
        }
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
