use crate::cost::TokenCost;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub model_id: String,
    pub provider: String,
    pub input_cost_per_thousand: TokenCost,
    pub output_cost_per_thousand: TokenCost,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub effective_date: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct ProviderPricing {
    models: HashMap<String, ModelPricing>,
}

impl ProviderPricing {
    pub fn new() -> Self {
        let mut pricing = Self {
            models: HashMap::new(),
        };
        pricing.load_default_pricing();
        pricing
    }

    pub fn get_model_pricing(&self, provider: &str, model: &str) -> Option<&ModelPricing> {
        let key = format!("{provider}:{model}");
        self.models.get(&key)
    }

    pub fn add_model_pricing(&mut self, pricing: ModelPricing) {
        let key = format!("{}:{}", pricing.provider, pricing.model_id);
        self.models.insert(key, pricing);
    }

    fn load_default_pricing(&mut self) {
        let now = chrono::Utc::now();

        // OpenAI Models
        self.add_model_pricing(ModelPricing {
            model_id: "gpt-4-turbo".to_string(),
            provider: "openai".to_string(),
            input_cost_per_thousand: TokenCost::from_cents(1000), // $10.00
            output_cost_per_thousand: TokenCost::from_cents(3000), // $30.00
            context_window: Some(128000),
            max_output_tokens: Some(4096),
            effective_date: now,
        });

        self.add_model_pricing(ModelPricing {
            model_id: "gpt-4".to_string(),
            provider: "openai".to_string(),
            input_cost_per_thousand: TokenCost::from_cents(3000), // $30.00
            output_cost_per_thousand: TokenCost::from_cents(6000), // $60.00
            context_window: Some(8192),
            max_output_tokens: Some(4096),
            effective_date: now,
        });

        self.add_model_pricing(ModelPricing {
            model_id: "gpt-3.5-turbo".to_string(),
            provider: "openai".to_string(),
            input_cost_per_thousand: TokenCost::from_cents(50), // $0.50
            output_cost_per_thousand: TokenCost::from_cents(150), // $1.50
            context_window: Some(16385),
            max_output_tokens: Some(4096),
            effective_date: now,
        });

        // Anthropic Models
        self.add_model_pricing(ModelPricing {
            model_id: "claude-3-opus-20240229".to_string(),
            provider: "anthropic".to_string(),
            input_cost_per_thousand: TokenCost::from_cents(1500), // $15.00
            output_cost_per_thousand: TokenCost::from_cents(7500), // $75.00
            context_window: Some(200000),
            max_output_tokens: Some(4096),
            effective_date: now,
        });

        self.add_model_pricing(ModelPricing {
            model_id: "claude-3-sonnet-20240229".to_string(),
            provider: "anthropic".to_string(),
            input_cost_per_thousand: TokenCost::from_cents(300), // $3.00
            output_cost_per_thousand: TokenCost::from_cents(1500), // $15.00
            context_window: Some(200000),
            max_output_tokens: Some(4096),
            effective_date: now,
        });

        self.add_model_pricing(ModelPricing {
            model_id: "claude-3-haiku-20240307".to_string(),
            provider: "anthropic".to_string(),
            input_cost_per_thousand: TokenCost::from_cents(25), // $0.25
            output_cost_per_thousand: TokenCost::from_cents(125), // $1.25
            context_window: Some(200000),
            max_output_tokens: Some(4096),
            effective_date: now,
        });

        // Ollama Models (typically free/local)
        self.add_model_pricing(ModelPricing {
            model_id: "llama3.2:latest".to_string(),
            provider: "ollama".to_string(),
            input_cost_per_thousand: TokenCost::ZERO,
            output_cost_per_thousand: TokenCost::ZERO,
            context_window: Some(8192),
            max_output_tokens: Some(4096),
            effective_date: now,
        });

        self.add_model_pricing(ModelPricing {
            model_id: "devstral:latest".to_string(),
            provider: "ollama".to_string(),
            input_cost_per_thousand: TokenCost::ZERO,
            output_cost_per_thousand: TokenCost::ZERO,
            context_window: Some(32768),
            max_output_tokens: Some(8192),
            effective_date: now,
        });

        // Kimi Models
        self.add_model_pricing(ModelPricing {
            model_id: "moonshot-v1-32k".to_string(),
            provider: "kimi".to_string(),
            input_cost_per_thousand: TokenCost::from_cents(200), // $2.00
            output_cost_per_thousand: TokenCost::from_cents(600), // $6.00
            context_window: Some(32000),
            max_output_tokens: Some(8192),
            effective_date: now,
        });

        self.add_model_pricing(ModelPricing {
            model_id: "moonshot-v1-128k".to_string(),
            provider: "kimi".to_string(),
            input_cost_per_thousand: TokenCost::from_cents(600), // $6.00
            output_cost_per_thousand: TokenCost::from_cents(1200), // $12.00
            context_window: Some(128000),
            max_output_tokens: Some(8192),
            effective_date: now,
        });

        // Local Models (no cost)
        self.add_model_pricing(ModelPricing {
            model_id: "local".to_string(),
            provider: "local".to_string(),
            input_cost_per_thousand: TokenCost::ZERO,
            output_cost_per_thousand: TokenCost::ZERO,
            context_window: Some(2048),
            max_output_tokens: Some(512),
            effective_date: now,
        });
    }

    pub fn estimate_cost(
        &self,
        provider: &str,
        model: &str,
        estimated_input_tokens: u32,
        estimated_output_tokens: u32,
    ) -> Option<TokenCost> {
        self.get_model_pricing(provider, model).map(|pricing| {
            let input_cost =
                TokenCost::from_tokens(estimated_input_tokens, pricing.input_cost_per_thousand);
            let output_cost =
                TokenCost::from_tokens(estimated_output_tokens, pricing.output_cost_per_thousand);
            input_cost + output_cost
        })
    }

    pub fn list_models_by_provider(&self, provider: &str) -> Vec<&ModelPricing> {
        self.models
            .values()
            .filter(|m| m.provider == provider)
            .collect()
    }

    pub fn find_cheapest_model_for_tokens(
        &self,
        required_context: u32,
        required_output: u32,
    ) -> Option<&ModelPricing> {
        self.models
            .values()
            .filter(|m| {
                m.context_window.unwrap_or(0) >= required_context
                    && m.max_output_tokens.unwrap_or(0) >= required_output
            })
            .min_by_key(|m| {
                let cost = self.estimate_cost(
                    &m.provider,
                    &m.model_id,
                    required_context / 2, // Estimate half context usage
                    required_output / 2,  // Estimate half output usage
                );
                cost.unwrap_or(TokenCost::from_cents(u64::MAX)).as_cents()
            })
    }
}

impl Default for ProviderPricing {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_pricing_defaults() {
        let pricing = ProviderPricing::new();

        let gpt4 = pricing.get_model_pricing("openai", "gpt-4").unwrap();
        assert_eq!(gpt4.input_cost_per_thousand.as_cents(), 3000);
        assert_eq!(gpt4.output_cost_per_thousand.as_cents(), 6000);

        let claude = pricing
            .get_model_pricing("anthropic", "claude-3-haiku-20240307")
            .unwrap();
        assert_eq!(claude.input_cost_per_thousand.as_cents(), 25);
        assert_eq!(claude.output_cost_per_thousand.as_cents(), 125);

        let llama = pricing
            .get_model_pricing("ollama", "llama3.2:latest")
            .unwrap();
        assert_eq!(llama.input_cost_per_thousand.as_cents(), 0);
    }

    #[test]
    fn test_cost_estimation() {
        let pricing = ProviderPricing::new();

        let cost = pricing
            .estimate_cost("openai", "gpt-3.5-turbo", 1000, 500)
            .unwrap();
        assert_eq!(cost.as_cents(), 125); // (1000 * 0.50 + 500 * 1.50) / 1000 = 1.25

        let cost = pricing
            .estimate_cost("anthropic", "claude-3-opus-20240229", 2000, 1000)
            .unwrap();
        assert_eq!(cost.as_cents(), 10500); // (2000 * 15 + 1000 * 75) / 1000 = 105.00
    }

    #[test]
    fn test_find_cheapest_model() {
        let pricing = ProviderPricing::new();

        let cheapest = pricing.find_cheapest_model_for_tokens(4000, 2000).unwrap();
        assert!(
            cheapest.model_id == "llama3.2:latest"
                || cheapest.model_id == "devstral:latest"
                || cheapest.model_id == "local"
        );
    }
}
