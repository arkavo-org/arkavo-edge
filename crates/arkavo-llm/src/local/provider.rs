use crate::provider::Provider;
use crate::{Error, Message, Result, StreamResponse};
use async_trait::async_trait;
use tokio_stream::Stream;

#[cfg(feature = "llm-local")]
use super::model_loader::ModelLoader;

#[cfg(feature = "llm-local")]
use std::sync::Arc;

#[cfg(feature = "llm-local")]
use tokio::sync::Mutex;

#[cfg(feature = "llm-local")]
struct Inner {
    provider: Box<dyn crate::provider::Provider>,
}

pub struct LocalProvider {
    #[cfg(feature = "llm-local")]
    inner: Arc<Mutex<Inner>>,
    model_name: String,
}

impl LocalProvider {
    pub fn new(model_name: String, model_path: Option<String>) -> Result<Self> {
        #[cfg(not(feature = "llm-local"))]
        {
            return Err(Error::Config(
                "Local provider requires 'llm-local' feature to be enabled".to_string(),
            ));
        }

        #[cfg(feature = "llm-local")]
        {
            Ok(Self {
                inner: Arc::new(Mutex::new(Inner {
                    provider: Box::new(super::candle_provider::CandleProvider::new(
                        model_name.clone(),
                        model_path,
                        &Default::default(),
                    )?),
                })),
                model_name,
            })
        }
    }

    #[cfg(feature = "llm-local")]
    #[allow(clippy::missing_panics_doc)]
    #[allow(clippy::significant_drop_tightening)]
    pub async fn initialize(&mut self, model_path: Option<String>) -> Result<()> {
        let mut model_loader = ModelLoader::new(&self.model_name, model_path.as_deref())?;
        model_loader.load_model()?;

        let provider = model_loader
            .get_provider()
            .ok_or_else(|| Error::Model("Model not loaded".to_string()))?;

        let mut guard = self.inner.lock().await;
        guard.provider = provider.clone();

        Ok(())
    }
}

#[async_trait]
impl Provider for LocalProvider {
    #[allow(clippy::significant_drop_tightening)]
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        let guard = self.inner.lock().await;
        guard.provider.complete(messages).await
    }

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>> {
        let guard = self.inner.lock().await;
        guard.provider.stream(messages).await
    }

    fn name(&self) -> &str {
        &self.model_name
    }
}
