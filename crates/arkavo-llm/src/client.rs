use crate::chat::ChatRequest;
use crate::config::LlmConfig;
#[cfg(feature = "llm-remote")]
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

        match provider_name.as_str() {
            #[cfg(feature = "llm-remote")]
            "ollama" => {
                let provider = Box::new(OllamaClient::from_env()?);
                Ok(Self::new(provider))
            }
            #[cfg(feature = "kimi")]
            "kimi" => {
                use crate::KimiProvider;
                let provider = Box::new(KimiProvider::from_env()?);
                Ok(Self::new(provider))
            }
            #[cfg(feature = "deepseek")]
            "deepseek" => {
                use crate::DeepSeekProvider;
                let provider = Box::new(DeepSeekProvider::from_env()?);
                Ok(Self::new(provider))
            }
            #[cfg(feature = "gemini")]
            "gemini" => {
                use crate::GeminiProvider;
                // Try to create Gemini provider, but return error if API key is missing
                // The error message now indicates this is optional and will fallback
                let provider = Box::new(GeminiProvider::new()?);
                Ok(Self::new(provider))
            }
            _ => {
                #[cfg(any(
                    feature = "llm-remote",
                    feature = "kimi",
                    feature = "deepseek",
                    feature = "gemini"
                ))]
                return Err(Error::Config(format!("Unknown provider: {provider_name}")));

                #[cfg(not(any(feature = "llm-remote", feature = "kimi", feature = "deepseek", feature = "gemini")))]
                return Err(Error::Config(
                    "No LLM providers available. Build with 'llm-remote', 'kimi', 'deepseek', 'gemini', or 'llama-cpp' feature enabled.".to_string()
                ));
            }
        }
    }

    /// Creates an LlmClient from an LlmConfig struct.
    ///
    /// This is the preferred method as it avoids the need for unsafe env var manipulation.
    pub fn from_config(config: &LlmConfig) -> Result<Self> {
        let provider_name = config.provider.to_lowercase();

        match provider_name.as_str() {
            #[cfg(feature = "llm-remote")]
            "ollama" => {
                let provider = Box::new(OllamaClient::from_config(config)?);
                Ok(Self::new(provider))
            }
            #[cfg(feature = "kimi")]
            "kimi" => {
                use crate::KimiProvider;
                let provider = Box::new(KimiProvider::from_config(config)?);
                Ok(Self::new(provider))
            }
            #[cfg(feature = "deepseek")]
            "deepseek" => {
                use crate::DeepSeekProvider;
                let provider = Box::new(DeepSeekProvider::from_config(config)?);
                Ok(Self::new(provider))
            }
            #[cfg(feature = "gemini")]
            "gemini" => {
                use crate::GeminiProvider;
                let provider = Box::new(GeminiProvider::from_config(config)?);
                Ok(Self::new(provider))
            }
            _ => {
                #[cfg(any(
                    feature = "llm-remote",
                    feature = "kimi",
                    feature = "deepseek",
                    feature = "gemini"
                ))]
                return Err(Error::Config(format!("Unknown provider: {provider_name}")));

                #[cfg(not(any(feature = "llm-remote", feature = "kimi", feature = "deepseek", feature = "gemini")))]
                return Err(Error::Config(
                    "No LLM providers available. Build with 'llm-remote', 'kimi', 'deepseek', 'gemini', or 'llama-cpp' feature enabled.".to_string()
                ));
            }
        }
    }

    // async is kept for API stability: callers await these constructors, and
    // neither cfg branch awaits today. 1.98 added a second lint for the same shape.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
    pub async fn from_local_model(model_name: &str, model_path: String) -> Result<Self> {
        #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
        {
            use crate::LlamaCppProvider;
            let provider = LlamaCppProvider::new(model_name.to_string(), model_path)?;
            Ok(Self::new(Box::new(provider)))
        }
        #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
        {
            let _ = (model_name, model_path); // Suppress unused variable warnings
            Err(Error::Config(
                "Local models require the 'llama-cpp' feature to be enabled".to_string(),
            ))
        }
    }

    // async is kept for API stability: callers await these constructors, and
    // neither cfg branch awaits today. 1.98 added a second lint for the same shape.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
    pub async fn from_llamacpp_model(model_name: &str, model_path: String) -> Result<Self> {
        #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
        {
            use crate::LlamaCppProvider;
            let provider = LlamaCppProvider::new(model_name.to_string(), model_path)?;
            Ok(Self::new(Box::new(provider)))
        }
        #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
        {
            let _ = (model_name, model_path); // Suppress unused variable warnings
            Err(Error::Config(
                "LLama.cpp models require the 'llama-cpp' feature and are not available on musl targets".to_string(),
            ))
        }
    }

    // async is kept for API stability: callers await these constructors, and
    // neither cfg branch awaits today. 1.98 added a second lint for the same shape.
    #[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
    pub async fn from_llamacpp_model_with_config(
        model_name: &str,
        model_path: String,
        temperature: f32,
        top_p: f32,
        top_k: i32,
        max_tokens: u32,
        seed: u32,
    ) -> Result<Self> {
        #[cfg(all(feature = "llama-cpp", not(target_env = "musl")))]
        {
            use crate::{LlamaCppProvider, llamacpp_provider::SamplingConfig};
            let config = SamplingConfig {
                temperature,
                top_p,
                top_k,
                max_tokens,
                seed,
                debug: false,
                ..Default::default()
            };
            let provider = LlamaCppProvider::new_with_config(
                model_name.to_string(),
                model_path,
                None,
                config,
            )?;
            Ok(Self::new(Box::new(provider)))
        }
        #[cfg(not(all(feature = "llama-cpp", not(target_env = "musl"))))]
        {
            let _ = (
                model_name,
                model_path,
                temperature,
                top_p,
                top_k,
                max_tokens,
                seed,
            ); // Suppress unused variable warnings
            Err(Error::Config(
                "LLama.cpp models require the 'llama-cpp' feature and are not available on musl targets".to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChatRequest, Error, Message, Provider, Result, StreamResponse};
    use arkavo_test_macros::spec;
    use async_trait::async_trait;
    use futures::StreamExt;
    use std::sync::Mutex;
    use tokio_stream::Stream;

    struct MockProvider {
        response: String,
        stream_chunks: Mutex<Option<Vec<Result<StreamResponse>>>>,
    }

    impl MockProvider {
        fn new(response: impl Into<String>) -> Self {
            Self {
                response: response.into(),
                stream_chunks: Mutex::new(None),
            }
        }

        fn with_stream(stream_chunks: Vec<Result<StreamResponse>>) -> Self {
            Self {
                response: String::new(),
                stream_chunks: Mutex::new(Some(stream_chunks)),
            }
        }
    }

    #[async_trait]
    impl Provider for MockProvider {
        async fn complete_with_options(
            &self,
            _messages: Vec<Message>,
            _max_tokens: Option<usize>,
        ) -> Result<String> {
            Ok(self.response.clone())
        }

        async fn stream(
            &self,
            _messages: Vec<Message>,
        ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
            let chunks = self
                .stream_chunks
                .lock()
                .expect("mock stream mutex poisoned")
                .take()
                .unwrap_or_default();
            Ok(Box::new(tokio_stream::iter(chunks)))
        }

        fn name(&self) -> &str {
            "mock"
        }
    }

    #[spec("LLM-001")]
    #[tokio::test]
    async fn chat_sends_request_to_provider_and_returns_response() {
        let provider = MockProvider::new("Hello from mock provider");
        let client = LlmClient::new(Box::new(provider));
        let request = ChatRequest::new("Say hello");

        let response = client.chat(request).await.expect("chat should succeed");

        assert_eq!(response, "Hello from mock provider");
    }

    #[spec("LLM-002")]
    #[tokio::test]
    async fn chat_stream_receives_deltas_and_handles_errors() {
        let chunks = vec![
            Ok(StreamResponse {
                content: "Hel".to_string(),
                reasoning_content: None,
                done: false,
                inference_timing: None,
            }),
            Err(Error::Stream("mid-stream failure".to_string())),
            Ok(StreamResponse {
                content: "lo".to_string(),
                reasoning_content: None,
                done: false,
                inference_timing: None,
            }),
            Ok(StreamResponse {
                content: String::new(),
                reasoning_content: None,
                done: true,
                inference_timing: None,
            }),
        ];
        let provider = MockProvider::with_stream(chunks);
        let client = LlmClient::new(Box::new(provider));
        let request = ChatRequest::new("Stream hello");

        let mut stream = client
            .chat_stream(request)
            .await
            .expect("chat_stream should return a stream");

        let mut content = String::new();
        let mut error_seen = false;
        let mut done_seen = false;
        while let Some(result) = stream.next().await {
            match result {
                Ok(response) => {
                    content.push_str(&response.content);
                    if response.done {
                        done_seen = true;
                    }
                }
                Err(err) => {
                    assert!(matches!(err, Error::Stream(_)));
                    error_seen = true;
                }
            }
        }

        assert_eq!(content, "Hello");
        assert!(error_seen, "stream should propagate a mid-stream error");
        assert!(done_seen, "stream should end with a done response");
    }
}
