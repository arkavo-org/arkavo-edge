use crate::ollama::OllamaClient;
use crate::{Error, Message, Provider, Result, StreamResponse, encode_image_file};
use tokio_stream::Stream;
use std::path::Path;

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

    pub async fn from_env_with_discovery() -> Result<Self> {
        // Check for provider preference
        let provider_name = std::env::var("LLM_PROVIDER")
            .unwrap_or_else(|_| "ollama".to_string())
            .to_lowercase();

        let provider: Box<dyn Provider> = match provider_name.as_str() {
            "ollama" => Box::new(OllamaClient::from_env_with_discovery().await?),
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

    pub async fn chat(&self, content: impl Into<String>) -> Result<String> {
        let message = Message::user(content);
        self.complete(vec![message]).await
    }

    pub async fn chat_with_images(
        &self,
        content: impl Into<String>,
        image_paths: Vec<impl AsRef<Path>>,
    ) -> Result<String> {
        let mut images = Vec::new();
        for path in image_paths {
            let encoded = encode_image_file(path)?;
            images.push(encoded);
        }

        let message = Message::user_with_images(content, images);
        self.complete(vec![message]).await
    }

    pub fn provider_name(&self) -> &str {
        self.provider.name()
    }

    pub async fn complete_with_images(
        &self,
        content: impl Into<String>,
        image_paths: Vec<impl AsRef<Path>>,
    ) -> Result<String> {
        let mut images = Vec::new();
        for path in image_paths {
            let encoded = encode_image_file(path)?;
            images.push(encoded);
        }

        let message = Message::user_with_images(content, images);
        self.complete(vec![message]).await
    }

    pub async fn stream_with_images(
        &self,
        content: impl Into<String>,
        image_paths: Vec<impl AsRef<Path>>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let mut images = Vec::new();
        for path in image_paths {
            let encoded = encode_image_file(path)?;
            images.push(encoded);
        }

        let message = Message::user_with_images(content, images);
        self.stream(vec![message]).await
    }

    pub async fn complete_with_encoded_images(
        &self,
        content: impl Into<String>,
        encoded_images: Vec<String>,
    ) -> Result<String> {
        let message = Message::user_with_images(content, encoded_images);
        self.complete(vec![message]).await
    }

    pub async fn stream_with_encoded_images(
        &self,
        content: impl Into<String>,
        encoded_images: Vec<String>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let message = Message::user_with_images(content, encoded_images);
        self.stream(vec![message]).await
    }
}
