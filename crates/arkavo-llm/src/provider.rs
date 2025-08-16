use crate::{Message, Result, StreamResponse};
use async_trait::async_trait;
use tokio_stream::Stream;

#[async_trait]
pub trait Provider: Send + Sync {
    async fn complete(&self, messages: Vec<Message>) -> Result<String>;

    async fn stream(
        &self,
        messages: Vec<Message>,
    ) -> Result<Box<dyn Stream<Item = Result<StreamResponse>> + Send + Unpin>>;

    fn name(&self) -> &str;
}
