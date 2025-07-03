use crate::{Error, Message, Provider, Result, StreamResponse};
use async_trait::async_trait;
use tokio_stream::Stream;

#[cfg(feature = "local")]
use super::{model_loader::ModelLoader, tokenizer::Tokenizer};

pub struct LocalProvider {
    #[cfg(feature = "local")]
    model_loader: ModelLoader,
    #[cfg(feature = "local")]
    tokenizer: Tokenizer,
    model_name: String,
    _model_path: Option<String>,
}

impl LocalProvider {
    pub fn new(_model_name: String, _model_path: Option<String>) -> Result<Self> {
        #[cfg(not(feature = "local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            let model_loader = ModelLoader::new(&_model_name, _model_path.as_deref())?;
            let tokenizer = Tokenizer::new(&_model_name)?;

            Ok(Self {
                model_loader,
                tokenizer,
                model_name: _model_name,
                _model_path,
            })
        }
    }
}

#[async_trait]
impl Provider for LocalProvider {
    async fn complete(&self, _messages: Vec<Message>) -> Result<String> {
        #[cfg(not(feature = "local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            // For now, return a placeholder implementation
            // This will be implemented with actual Candle inference
            let prompt = _messages
                .into_iter()
                .map(|m| m.content)
                .collect::<Vec<_>>()
                .join("\n");

            // TODO: Implement actual model inference
            Ok(format!(
                "Local model '{}' received prompt: {}",
                self.model_name, prompt
            ))
        }
    }

    async fn stream(
        &self,
        _messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        #[cfg(not(feature = "local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "local")]
        {
            // For now, return a simple stream implementation
            // This will be implemented with actual streaming inference
            use futures::stream;

            let response = self.complete(_messages).await?;
            let stream = stream::once(async move {
                Ok(StreamResponse {
                    content: response,
                    done: true,
                })
            });

            Ok(Box::new(stream))
        }
    }

    fn name(&self) -> &str {
        &self.model_name
    }
}
