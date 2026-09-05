//! Response-local attribution, independent of mutable routing diagnostics.
use crate::{
    ModelChoice, Router,
    error::{Error, Result},
};
use arkavo_budget::{BudgetTracker, TokenCost, cost::TokenUsage};
use arkavo_llm::{Message, ProviderResponse};

#[derive(Debug, Clone)]
pub struct ModelUsage {
    pub model: ModelChoice,
    pub usage: TokenUsage,
    /// Estimated dollars before whole-cent ledger quantization.
    pub cost_usd: f64,
}

#[derive(Debug)]
pub struct RoutedResponse {
    pub response: ProviderResponse,
    pub model: ModelChoice,
    /// Every successful provider call, including rejected quality-gate attempts.
    pub attempts: Vec<ModelUsage>,
}

impl RoutedResponse {
    pub fn total_tokens(&self) -> u32 {
        self.attempts.iter().fold(0u32, |sum, call| {
            sum.saturating_add(call.usage.total_tokens())
        })
    }
}

#[derive(Clone, Copy)]
pub struct CallBudget<'a> {
    pub tracker: &'a BudgetTracker,
    pub agent_id: &'a str,
}

impl CallBudget<'_> {
    pub async fn check(&self, estimated_dollars: f64) -> Result<()> {
        // Round only the preflight bound; settled charges retain fractional cents.
        let estimate = TokenCost::from_cents((estimated_dollars * 100.0).ceil() as u64);
        if !self
            .tracker
            .can_afford(self.agent_id, estimate)
            .await
            .map_err(|e| Error::BudgetError(e.to_string()))?
        {
            return Err(Error::BudgetExceeded(format!(
                "shared budget cannot fund {}",
                self.agent_id
            )));
        }
        Ok(())
    }

    pub async fn record(&self, call: &ModelUsage) -> Result<()> {
        self.tracker
            .record_spending_precise(
                self.agent_id.to_string(),
                call.model.provider().to_string(),
                call.model.name().to_string(),
                call.usage.clone(),
                call.cost_usd,
            )
            .await
            .map_err(|e| Error::BudgetError(e.to_string()))?;
        Ok(())
    }
}

pub fn estimate_request(
    messages: &[Message],
    tools: Option<&serde_json::Value>,
    output: u32,
) -> TokenUsage {
    let bytes = serde_json::to_vec(messages)
        .map_or(0, |s| s.len())
        .saturating_add(tools.map_or(0, |t| t.to_string().len()));
    TokenUsage::new(estimate_tokens(bytes), output)
}

fn estimate_tokens(bytes: usize) -> u32 {
    u32::try_from(bytes.div_ceil(3)).unwrap_or(u32::MAX)
}

pub fn response_usage(estimated_request: &TokenUsage, response: &ProviderResponse) -> TokenUsage {
    if let Some(timing) = &response.inference_timing {
        let cached = timing
            .n_cached_prompt_eval
            .unwrap_or(0)
            .min(timing.n_prompt_eval);
        let cache_write = timing
            .n_cache_write_prompt_eval
            .unwrap_or(0)
            .min(timing.n_prompt_eval.saturating_sub(cached));
        return TokenUsage {
            input_tokens: timing
                .n_prompt_eval
                .saturating_sub(cached)
                .saturating_sub(cache_write),
            cached_input_tokens: cached,
            output_tokens: timing.n_eval,
            thinking_tokens: timing.n_thinking_eval.unwrap_or(0),
            cache_write_tokens: cache_write,
        };
    }
    let tool_bytes: usize = response
        .tool_calls
        .iter()
        .map(|t| t.arguments.to_string().len() + t.tool_name.len())
        .sum();
    TokenUsage {
        input_tokens: estimated_request.input_tokens,
        output_tokens: estimate_tokens(response.content.len().saturating_add(tool_bytes)),
        thinking_tokens: estimate_tokens(
            response.reasoning_content.as_ref().map_or(0, String::len),
        ),
        ..Default::default()
    }
}

impl Router {
    pub(crate) fn call_budget(&self) -> Option<CallBudget<'_>> {
        self.budget_tracker.as_deref().map(|tracker| CallBudget {
            tracker,
            agent_id: self.budget_agent.as_deref().unwrap_or("router"),
        })
    }

    pub(crate) async fn account_result(
        &self,
        model: &ModelChoice,
        estimated: &TokenUsage,
        result: arkavo_llm::Result<ProviderResponse>,
        budget: Option<CallBudget<'_>>,
    ) -> Result<ProviderResponse> {
        if let Some(budget) = budget {
            let timing_only;
            let response = match &result {
                Ok(response) => Some(response),
                Err(error) => {
                    timing_only = error.inference_timing().map(|timing| ProviderResponse {
                        inference_timing: Some(timing.clone()),
                        ..Default::default()
                    });
                    timing_only.as_ref()
                }
            };
            if let Some(response) = response {
                budget
                    .record(&self.attribute_response(model.clone(), estimated, response))
                    .await?;
            }
        }
        result.map_err(Error::Provider)
    }

    pub fn attribute_response(
        &self,
        model: ModelChoice,
        estimated_request: &TokenUsage,
        response: &ProviderResponse,
    ) -> ModelUsage {
        let usage = response_usage(estimated_request, response);
        let cost_usd = self.usage_cost(&model, &usage);
        ModelUsage {
            model,
            usage,
            cost_usd,
        }
    }

    pub fn usage_cost(&self, model: &ModelChoice, usage: &TokenUsage) -> f64 {
        if let Ok(pricing) = self.pricing.read()
            && let Some(price) = pricing.get_model_pricing(model.provider(), model.name())
        {
            let input = price.input_cost_per_mtok.as_dollars();
            // Authored rates are the base tier, matching the registry's per-MTok
            // schema; Astra's documented long-context multipliers still apply.
            let (input_multiplier, output_multiplier) = if matches!(model, ModelChoice::Gpt6Astra)
                && usage
                    .input_tokens
                    .saturating_add(usage.cached_input_tokens)
                    .saturating_add(usage.cache_write_tokens)
                    > 272_000
            {
                (2.0, 1.5)
            } else {
                (1.0, 1.0)
            };
            return calculate_cost(
                usage,
                input * input_multiplier,
                price.output_cost_per_mtok.as_dollars() * output_multiplier,
                price
                    .cached_input_cost_per_mtok
                    .map_or(input, |p| p.as_dollars())
                    * input_multiplier,
                price
                    .cache_write_cost_per_mtok
                    .map_or(input, |p| p.as_dollars())
                    * input_multiplier,
            );
        }
        model.usage_cost_usd(usage)
    }
}

fn calculate_cost(usage: &TokenUsage, input: f64, output: f64, cached: f64, write: f64) -> f64 {
    let ordinary = f64::from(usage.input_tokens) * input;
    let generated = f64::from(usage.output_tokens.saturating_add(usage.thinking_tokens))
        .mul_add(output, ordinary);
    let read = f64::from(usage.cached_input_tokens).mul_add(cached, generated);
    f64::from(usage.cache_write_tokens).mul_add(write, read) / 1_000_000.0
}

impl ModelChoice {
    /// Price measured usage without the reasoning multipliers used for routing priors.
    pub fn usage_cost_usd(&self, usage: &TokenUsage) -> f64 {
        let (input, output, cached, write) = match self {
            Self::Gpt6Astra => {
                if usage
                    .input_tokens
                    .saturating_add(usage.cached_input_tokens)
                    .saturating_add(usage.cache_write_tokens)
                    > 272_000
                {
                    (20.0, 75.0, 2.0, 25.0)
                } else {
                    (10.0, 50.0, 1.0, 12.5)
                }
            }
            Self::GeminiFlash => (0.30, 2.50, 0.30, 0.30),
            Self::Gemini35Flash
            | Self::Gemini35FlashMinimal
            | Self::Gemini35FlashMedium
            | Self::Gemini35FlashHigh => (1.50, 9.00, 1.50, 1.50),
            Self::GeminiPro => (1.25, 5.0, 1.25, 1.25),
            Self::ClaudeSonnet => (3.0, 15.0, 0.30, 3.75),
            Self::ClaudeOpus => (5.0, 25.0, 0.50, 6.25),
            Self::ClaudeFable5 => (10.0, 50.0, 1.0, 12.5),
            Self::DeepSeekV32 | Self::DeepSeekV32Speciale => (0.27, 1.10, 0.27, 0.27),
            Self::KimiK2 => (0.55, 2.20, 0.55, 0.55),
            Self::Glm52 => (1.40, 4.40, 1.40, 1.40),
            Self::Grok46 | Self::Grok46Xhigh => (2.0, 6.0, 0.50, 2.0),
            _ => return 0.0,
        };
        calculate_cost(usage, input, output, cached, write)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_llm::provider::InferenceTiming;
    use arkavo_test_macros::spec;

    fn measured_response() -> ProviderResponse {
        ProviderResponse {
            content: "done".into(),
            inference_timing: Some(InferenceTiming {
                n_prompt_eval: 1000,
                n_cached_prompt_eval: Some(600),
                n_eval: 100,
                n_thinking_eval: Some(200),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[spec("ASTRA-005")]
    #[test]
    fn cache_and_reasoning_are_counted_once() {
        let usage = response_usage(&TokenUsage::default(), &measured_response());
        assert_eq!(usage.input_tokens, 400);
        assert_eq!(usage.cached_input_tokens, 600);
        assert_eq!(usage.output_tokens, 100);
        assert_eq!(usage.thinking_tokens, 200);
        assert_eq!(usage.total_tokens(), 1300);
        assert!((ModelChoice::Gpt6Astra.usage_cost_usd(&usage) - 0.0196).abs() < 1e-9);
    }

    #[spec("ASTRA-005")]
    #[test]
    fn astra_long_prompt_prices_the_whole_request() {
        let short = TokenUsage::with_cache(200_000, 1000, 72_000, 0);
        let long = TokenUsage::with_cache(200_001, 1000, 72_000, 0);
        assert!((ModelChoice::Gpt6Astra.usage_cost_usd(&short) - 2.122).abs() < 1e-9);
        assert!((ModelChoice::Gpt6Astra.usage_cost_usd(&long) - 4.21902).abs() < 1e-9);
    }

    struct MeasuredProvider;

    #[async_trait::async_trait]
    impl arkavo_llm::Provider for MeasuredProvider {
        async fn complete_with_options(
            &self,
            _: Vec<Message>,
            _: Option<usize>,
        ) -> arkavo_llm::Result<String> {
            Ok("done".into())
        }
        async fn stream(
            &self,
            _: Vec<Message>,
        ) -> arkavo_llm::Result<
            Box<
                dyn futures::Stream<Item = arkavo_llm::Result<arkavo_llm::StreamResponse>>
                    + Send
                    + Unpin,
            >,
        > {
            Ok(Box::new(futures::stream::empty()))
        }
        fn name(&self) -> &str {
            "openai"
        }
        async fn complete_with_tools(
            &self,
            _: Vec<Message>,
            _: Option<serde_json::Value>,
            _: Option<usize>,
        ) -> arkavo_llm::Result<ProviderResponse> {
            Ok(measured_response())
        }
        async fn complete_with_schema_response(
            &self,
            _: Vec<Message>,
            _: Option<serde_json::Value>,
            _: Option<usize>,
        ) -> arkavo_llm::Result<ProviderResponse> {
            Ok(measured_response())
        }
    }

    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn planning_and_execution_share_spend_and_enforce_next_call() {
        use arkavo_llm::Provider;
        let mut config = arkavo_budget::BudgetConfig::default();
        config.limits.session_limit = Some(TokenCost::from_cents(4));
        let tracker = BudgetTracker::new(config).await.unwrap();
        let router = Router::new_offline().await.unwrap();
        let provider = MeasuredProvider;
        let model = ModelChoice::Gpt6Astra;
        let estimate = response_usage(&TokenUsage::default(), &measured_response());
        let planning = CallBudget {
            tracker: &tracker,
            agent_id: "github-orchestrator",
        };
        let execution = CallBudget {
            tracker: &tracker,
            agent_id: "github-orchestrator-step",
        };
        planning
            .check(model.usage_cost_usd(&estimate))
            .await
            .unwrap();
        let response = provider
            .complete_with_schema_response(vec![Message::user("plan one step")], None, None)
            .await;
        router
            .account_result(&model, &estimate, response, Some(planning))
            .await
            .unwrap();
        execution
            .check(model.usage_cost_usd(&estimate))
            .await
            .unwrap();
        let response = provider
            .complete_with_tools(vec![Message::user("execute step")], None, None)
            .await;
        router
            .account_result(&model, &estimate, response, Some(execution))
            .await
            .unwrap();
        assert!(
            execution
                .check(model.usage_cost_usd(&estimate))
                .await
                .is_err()
        );
        let history = tracker.get_spending_history(10).await;
        assert_eq!(history.len(), 2);
        assert!(
            history
                .iter()
                .all(|entry| entry.model == "gpt-6-astra" && entry.provider == "openai")
        );
        assert_eq!(
            history
                .iter()
                .map(|entry| entry.usage.total_tokens())
                .sum::<u32>(),
            2600
        );
    }

    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn rejected_attempt_is_preserved_before_subsequent_failure() {
        let tracker = BudgetTracker::new(arkavo_budget::BudgetConfig::default())
            .await
            .unwrap();
        let budget = CallBudget {
            tracker: &tracker,
            agent_id: "github-orchestrator-step",
        };
        let usage = response_usage(&TokenUsage::default(), &measured_response());
        budget
            .record(&ModelUsage {
                model: ModelChoice::Gpt6Astra,
                cost_usd: 0.02,
                usage,
            })
            .await
            .unwrap();
        assert!(budget.check(1000.0).await.is_err());
        let history = tracker.get_spending_history(10).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].usage.total_tokens(), 1300);
    }
    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn authored_astra_rates_keep_long_context_tiers() {
        use arkavo_budget::provider_costs::{PricingEntry, ProviderPricing};
        let mut pricing = ProviderPricing::new();
        pricing.register(&PricingEntry {
            model_id: "gpt-6-astra".into(),
            provider: "openai".into(),
            input_cents_per_mtok: 1000,
            output_cents_per_mtok: 5000,
            cached_input_cents_per_mtok: Some(100),
            cache_write_cents_per_mtok: Some(1250),
            context_window: None,
            max_output_tokens: None,
        });
        let router = Router::new_offline().await.unwrap().with_pricing(pricing);
        let usage = TokenUsage::with_cache(200_001, 1000, 72_000, 0);
        assert!((router.usage_cost(&ModelChoice::Gpt6Astra, &usage) - 4.21902).abs() < 1e-9);
    }

    #[spec("ASTRA-005")]
    #[test]
    fn cache_writes_are_not_charged_as_ordinary_input_too() {
        let mut response = measured_response();
        response
            .inference_timing
            .as_mut()
            .unwrap()
            .n_cache_write_prompt_eval = Some(100);
        let usage = response_usage(&TokenUsage::default(), &response);
        assert_eq!(usage.input_tokens, 300);
        assert_eq!(usage.cached_input_tokens, 600);
        assert_eq!(usage.cache_write_tokens, 100);
        assert_eq!(usage.total_tokens(), 1300);
        assert!((ModelChoice::Gpt6Astra.usage_cost_usd(&usage) - 0.01985).abs() < 1e-9);
    }
    #[spec("ASTRA-005")]
    #[tokio::test]
    async fn failed_completion_keeps_reported_usage_in_ledger() {
        let tracker = BudgetTracker::new(arkavo_budget::BudgetConfig::default())
            .await
            .unwrap();
        let router = Router::new_offline().await.unwrap();
        let failure = arkavo_llm::Error::ProviderResponseFailure {
            message: "response incomplete".into(),
            inference_timing: measured_response().inference_timing,
        };
        let result = router
            .account_result(
                &ModelChoice::Gpt6Astra,
                &TokenUsage::default(),
                Err(failure),
                Some(CallBudget {
                    tracker: &tracker,
                    agent_id: "github-orchestrator-step",
                }),
            )
            .await;
        assert!(result.is_err());
        let history = tracker.get_spending_history(10).await;
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].usage.total_tokens(), 1300);
        assert_eq!(history[0].model, "gpt-6-astra");
    }
}
