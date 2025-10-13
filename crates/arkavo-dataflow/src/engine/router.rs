use crate::dsl::blueprint::{MergeStrategy, SplitStrategy};
use crate::engine::Message;
use anyhow::Result;
use std::collections::HashMap;
use tokio::sync::mpsc;

// MessageRouter provides advanced routing capabilities for dataflow pipelines.
// It supports split/merge strategies, load balancing, and conditional routing.
// Currently reserved for future pipeline topology enhancements.
pub struct MessageRouter {
    split_strategies: HashMap<String, Box<dyn SplitRouter>>,
    merge_strategies: HashMap<String, Box<dyn MergeRouter>>,
}

#[async_trait::async_trait]
pub trait SplitRouter: Send + Sync {
    async fn route(&self, message: Message, targets: &[mpsc::Sender<Message>]) -> Result<()>;
}

#[async_trait::async_trait]
pub trait MergeRouter: Send + Sync {
    async fn merge(&mut self, messages: Vec<Message>) -> Result<Option<Message>>;
}

pub struct RoundRobinRouter {
    current_index: std::sync::atomic::AtomicUsize,
}

#[async_trait::async_trait]
impl SplitRouter for RoundRobinRouter {
    async fn route(&self, message: Message, targets: &[mpsc::Sender<Message>]) -> Result<()> {
        if targets.is_empty() {
            return Err(anyhow::anyhow!("No targets available for routing"));
        }

        let index = self
            .current_index
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            % targets.len();
        targets[index]
            .send(message)
            .await
            .map_err(|_| anyhow::anyhow!("Failed to send message to target"))?;

        Ok(())
    }
}

pub struct BroadcastRouter;

#[async_trait::async_trait]
impl SplitRouter for BroadcastRouter {
    async fn route(&self, message: Message, targets: &[mpsc::Sender<Message>]) -> Result<()> {
        let mut send_futures = Vec::new();

        for target in targets {
            send_futures.push(target.send(message.clone()));
        }

        // Send to all targets concurrently
        let results = futures::future::join_all(send_futures).await;

        // Check if any sends failed
        let failed_count = results.iter().filter(|r| r.is_err()).count();

        if failed_count > 0 {
            Err(anyhow::anyhow!("Failed to send to {failed_count} targets"))
        } else {
            Ok(())
        }
    }
}

pub struct FirstMerger;

#[async_trait::async_trait]
impl MergeRouter for FirstMerger {
    async fn merge(&mut self, messages: Vec<Message>) -> Result<Option<Message>> {
        Ok(messages.into_iter().next())
    }
}

pub struct ConcatMerger;

#[async_trait::async_trait]
impl MergeRouter for ConcatMerger {
    async fn merge(&mut self, messages: Vec<Message>) -> Result<Option<Message>> {
        if messages.is_empty() {
            return Ok(None);
        }

        // Concatenate payloads from all messages
        let mut combined_payload = serde_json::json!([]);
        let combined_array = combined_payload.as_array_mut().unwrap();

        for message in messages {
            if let Message::Data { payload, .. } = message {
                combined_array.push(payload);
            }
        }

        Ok(Some(Message::new_data(combined_payload, "merger")))
    }
}

impl MessageRouter {
    pub fn new() -> Self {
        let mut router = Self {
            split_strategies: HashMap::new(),
            merge_strategies: HashMap::new(),
        };

        // Register default strategies
        router.register_split_strategy(
            "round_robin".to_string(),
            Box::new(RoundRobinRouter {
                current_index: std::sync::atomic::AtomicUsize::new(0),
            }),
        );
        router.register_split_strategy("broadcast".to_string(), Box::new(BroadcastRouter));

        router.register_merge_strategy("first".to_string(), Box::new(FirstMerger));
        router.register_merge_strategy("concat".to_string(), Box::new(ConcatMerger));

        router
    }

    pub fn register_split_strategy(&mut self, name: String, strategy: Box<dyn SplitRouter>) {
        self.split_strategies.insert(name, strategy);
    }

    pub fn register_merge_strategy(&mut self, name: String, strategy: Box<dyn MergeRouter>) {
        self.merge_strategies.insert(name, strategy);
    }

    pub fn get_split_router(&self, strategy: &SplitStrategy) -> Option<&dyn SplitRouter> {
        let name = match strategy {
            SplitStrategy::RoundRobin => "round_robin",
            SplitStrategy::Broadcast => "broadcast",
            SplitStrategy::Hash => "hash", // Not implemented yet
            SplitStrategy::Conditional => "conditional", // Not implemented yet
        };

        self.split_strategies
            .get(name)
            .map(std::convert::AsRef::as_ref)
    }

    pub fn get_merge_router(&self, strategy: &MergeStrategy) -> Option<&dyn MergeRouter> {
        let name = match strategy {
            MergeStrategy::First => "first",
            MergeStrategy::Concat => "concat",
            MergeStrategy::All => "all",   // Not implemented yet
            MergeStrategy::Vote => "vote", // Not implemented yet
        };

        self.merge_strategies
            .get(name)
            .map(std::convert::AsRef::as_ref)
    }
}

impl Default for MessageRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_round_robin_routing() {
        let router = RoundRobinRouter {
            current_index: std::sync::atomic::AtomicUsize::new(0),
        };

        let (tx1, mut rx1) = mpsc::channel(10);
        let (tx2, mut rx2) = mpsc::channel(10);
        let targets = vec![tx1, tx2];

        // Send 4 messages
        for i in 0..4 {
            let msg = Message::new_data(serde_json::json!({"id": i}), "test");
            router.route(msg, &targets).await.unwrap();
        }

        // Should receive 2 messages each
        assert!(rx1.try_recv().is_ok());
        assert!(rx1.try_recv().is_ok());
        assert!(rx1.try_recv().is_err());

        assert!(rx2.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
        assert!(rx2.try_recv().is_err());
    }

    #[tokio::test]
    async fn test_broadcast_routing() {
        let router = BroadcastRouter;

        let (tx1, mut rx1) = mpsc::channel(10);
        let (tx2, mut rx2) = mpsc::channel(10);
        let targets = vec![tx1, tx2];

        let msg = Message::new_data(serde_json::json!({"test": true}), "test");
        router.route(msg, &targets).await.unwrap();

        // Both should receive the message
        assert!(rx1.try_recv().is_ok());
        assert!(rx2.try_recv().is_ok());
    }
}
