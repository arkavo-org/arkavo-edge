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
                    "No LLM providers available. Build with 'llm-remote', 'kimi', 'deepseek', 'gemini', or 'llm-local' feature enabled.".to_string()
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

    #[allow(clippy::unused_async)]
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

    #[allow(clippy::unused_async)]
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
            };
            let provider =
                LlamaCppProvider::new_with_config(model_name.to_string(), model_path, config)?;
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
