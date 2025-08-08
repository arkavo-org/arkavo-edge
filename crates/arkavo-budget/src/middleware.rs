use crate::TokenCost;
use crate::tracker::BudgetTracker;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use std::sync::Arc;
use tokio_stream::{Stream, StreamExt};
use tracing::{debug, warn};

/// A trait that providers can implement to report token usage
pub trait TokenUsageProvider {
    fn get_input_tokens(&self) -> Option<u32>;
    fn get_output_tokens(&self) -> Option<u32>;
}

/// Budget-aware wrapper for LLM providers
pub struct BudgetMiddleware<P> {
    inner: P,
    tracker: Arc<BudgetTracker>,
    agent_id: String,
    provider_name: String,
    model_id: String,
}

impl<P> BudgetMiddleware<P> {
    pub fn new(
        inner: P,
        tracker: Arc<BudgetTracker>,
        agent_id: String,
        provider_name: String,
        model_id: String,
    ) -> Self {
        Self {
            inner,
            tracker,
            agent_id,
            provider_name,
            model_id,
        }
    }

    async fn check_budget(&self, estimated_tokens: (u32, u32)) -> Result<TokenCost> {
        let (input_tokens, output_tokens) = estimated_tokens;

        // Estimate cost using the provider costs module
        use crate::provider_costs::ProviderPricing;
        let pricing = ProviderPricing::new();
        let estimated_cost = pricing
            .estimate_cost(
                &self.provider_name,
                &self.model_id,
                input_tokens,
                output_tokens,
            )
            .ok_or_else(|| {
                anyhow!(
                    "No pricing data for {}/{}",
                    self.provider_name,
                    self.model_id
                )
            })?;

        debug!(
            "Estimated cost for {}/{}: ${:.4} ({}+{} tokens)",
            self.provider_name,
            self.model_id,
            estimated_cost.as_dollars(),
            input_tokens,
            output_tokens
        );

        // Check if we can afford it
        let can_afford = self
            .tracker
            .can_afford(&self.agent_id, estimated_cost)
            .await?;

        if !can_afford {
            let status = self.tracker.get_status().await;
            warn!(
                "Budget exceeded for agent {}: ${:.4} spent of ${:.4} limit",
                self.agent_id,
                status.session_spent.as_dollars(),
                status.session_limit.map(|l| l.as_dollars()).unwrap_or(0.0)
            );
            return Err(anyhow!(
                "Budget exceeded: cannot afford ${:.4} for {}/{}",
                estimated_cost.as_dollars(),
                self.provider_name,
                self.model_id
            ));
        }

        Ok(estimated_cost)
    }

    async fn record_usage(&self, actual_tokens: (u32, u32)) -> Result<()> {
        let (input_tokens, output_tokens) = actual_tokens;

        // Calculate actual cost using the provider costs module
        use crate::cost::TokenUsage;
        use crate::provider_costs::ProviderPricing;
        let pricing = ProviderPricing::new();
        let actual_cost = pricing
            .estimate_cost(
                &self.provider_name,
                &self.model_id,
                input_tokens,
                output_tokens,
            )
            .ok_or_else(|| {
                anyhow!(
                    "No pricing data for {}/{}",
                    self.provider_name,
                    self.model_id
                )
            })?;

        debug!(
            "Recording spend of ${:.4} for agent {} using {}/{}",
            actual_cost.as_dollars(),
            self.agent_id,
            self.provider_name,
            self.model_id
        );

        // Record the spending
        let usage = TokenUsage::new(input_tokens, output_tokens);
        self.tracker
            .record_spending(
                self.agent_id.clone(),
                self.provider_name.clone(),
                self.model_id.clone(),
                usage,
                actual_cost,
            )
            .await?;

        Ok(())
    }

    fn estimate_tokens(messages: &[impl AsRef<str>]) -> (u32, u32) {
        // Simple estimation: ~4 characters per token
        let input_chars: usize = messages.iter().map(|m| m.as_ref().len()).sum();
        let input_tokens = (input_chars / 4) as u32;

        // Estimate output tokens based on typical response size
        let output_tokens = (input_tokens / 2).clamp(100, 2000);

        (input_tokens, output_tokens)
    }
}

// Example implementation for a generic provider
// In practice, each provider would need its own implementation
#[async_trait]
impl<P> BudgetAwareProvider for BudgetMiddleware<P>
where
    P: Provider + Send + Sync,
    P::Message: std::fmt::Debug,
    P::StreamResponse: std::fmt::Debug + 'static,
{
    type Message = P::Message;
    type StreamResponse = P::StreamResponse;

    async fn complete(&self, messages: Vec<Self::Message>) -> Result<String> {
        // Estimate tokens from messages
        let message_contents: Vec<String> = messages
            .iter()
            .map(|m| format!("{m:?}")) // Provider-specific serialization
            .collect();
        let estimated_tokens = Self::estimate_tokens(&message_contents);

        // Check budget before making request
        self.check_budget(estimated_tokens).await?;

        // Make the actual request
        let result = self.inner.complete(messages).await?;

        // Record actual usage (if available from response)
        // For now, estimate based on response length
        let output_tokens = (result.len() / 4) as u32;
        let actual_tokens = (estimated_tokens.0, output_tokens);
        self.record_usage(actual_tokens).await?;

        Ok(result)
    }

    async fn stream(
        &self,
        messages: Vec<Self::Message>,
    ) -> Result<Box<dyn Stream<Item = Result<Self::StreamResponse>> + Send + Unpin>> {
        // Estimate tokens from messages
        let message_contents: Vec<String> = messages.iter().map(|m| format!("{m:?}")).collect();
        let estimated_tokens = Self::estimate_tokens(&message_contents);

        // Check budget before making request
        self.check_budget(estimated_tokens).await?;

        // Get the stream from inner provider
        let mut inner_stream = self.inner.stream(messages).await?;
        let tracker = Arc::clone(&self.tracker);
        let agent_id = self.agent_id.clone();
        let provider_name = self.provider_name.clone();
        let model_id = self.model_id.clone();

        // Wrap the stream to track tokens
        let mut accumulated_output = String::new();
        let input_tokens = estimated_tokens.0;

        let tracked_stream = async_stream::stream! {
            while let Some(item) = inner_stream.next().await {
                if let Ok(response) = &item {
                    // Accumulate output for token counting
                    accumulated_output.push_str(&format!("{response:?}"));
                }
                yield item;
            }

            // After stream completes, record usage
            let output_tokens = (accumulated_output.len() / 4) as u32;
            use crate::provider_costs::ProviderPricing;
            use crate::cost::TokenUsage;
            let pricing = ProviderPricing::new();
            if let Some(cost) = pricing.estimate_cost(&provider_name, &model_id, input_tokens, output_tokens) {
                let usage = TokenUsage::new(input_tokens, output_tokens);
                let _ = tracker
                    .record_spending(agent_id, provider_name, model_id, usage, cost)
                    .await;
            }
        };

        Ok(Box::new(Box::pin(tracked_stream)))
    }

    fn name(&self) -> &str {
        self.inner.name()
    }
}

// Provider trait that budget middleware wraps
#[async_trait]
pub trait Provider: Send + Sync {
    type Message: Send + Sync;
    type StreamResponse: Send + Sync;

    async fn complete(&self, messages: Vec<Self::Message>) -> Result<String>;
    async fn stream(
        &self,
        messages: Vec<Self::Message>,
    ) -> Result<Box<dyn Stream<Item = Result<Self::StreamResponse>> + Send + Unpin>>;
    fn name(&self) -> &str;
}

// Budget-aware version of the provider trait
#[async_trait]
pub trait BudgetAwareProvider: Send + Sync {
    type Message: Send + Sync;
    type StreamResponse: Send + Sync;

    async fn complete(&self, messages: Vec<Self::Message>) -> Result<String>;
    async fn stream(
        &self,
        messages: Vec<Self::Message>,
    ) -> Result<Box<dyn Stream<Item = Result<Self::StreamResponse>> + Send + Unpin>>;
    fn name(&self) -> &str;
}

/// Builder for creating budget-aware providers
pub struct BudgetProviderBuilder {
    tracker: Arc<BudgetTracker>,
    agent_id: String,
}

impl BudgetProviderBuilder {
    pub fn new(tracker: Arc<BudgetTracker>, agent_id: String) -> Self {
        Self { tracker, agent_id }
    }

    pub fn wrap<P>(
        &self,
        provider: P,
        provider_name: String,
        model_id: String,
    ) -> BudgetMiddleware<P> {
        BudgetMiddleware::new(
            provider,
            Arc::clone(&self.tracker),
            self.agent_id.clone(),
            provider_name,
            model_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_token_estimation() {
        let messages = vec!["Hello world", "This is a test message"];
        let (input, output) = BudgetMiddleware::<()>::estimate_tokens(&messages);

        // "Hello world" (11) + "This is a test message" (22) = 33 chars
        // 33 / 4 = 8 tokens
        assert_eq!(input, 8);

        // Output should be between 100 and 2000, and half of input (4) would be below min
        assert_eq!(output, 100);
    }
}
