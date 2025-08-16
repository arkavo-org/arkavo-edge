use crate::Result;
use crate::local::config::LocalConfig;
use crate::provider::Provider as CompletionProvider;
use arkavo_llama_cpp::{LlamaContext, LlamaModel};
use async_trait::async_trait;

pub struct LlamaCppProvider {
    model: LlamaModel,
    context: LlamaContext,
}

impl LlamaCppProvider {
    pub fn new(model_path: &str, _config: &LocalConfig) -> Result<Self> {
        let model = LlamaModel::from_file(model_path).map_err(|e| crate::Error::Model(e))?;
        let context = LlamaContext::new(&model).map_err(|e| crate::Error::Model(e))?;

        Ok(Self { model, context })
    }
}

#[async_trait]
impl crate::provider::Provider for LlamaCppProvider {
    async fn complete(&self, messages: Vec<Message>) -> Result<String> {
        // TODO: Implement completion generation
        Ok("".to_string())
    }

    async fn stream(
        &self,
        _messages: Vec<Message>,
    ) -> Result<Box<dyn tokio_stream::Stream<Item = Result<crate::StreamResponse>> + Send + Unpin>>
    {
        // TODO: Implement streaming
        let items = vec![Ok(crate::StreamResponse {
            content: "".to_string(),
            done: true,
        })];
        let stream = tokio_stream::iter(items);
        Ok(Box::new(stream))
    }

    fn name(&self) -> &str {
        "llama.cpp"
    }
}
