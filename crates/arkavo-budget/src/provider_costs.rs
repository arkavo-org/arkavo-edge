use crate::cost::TokenCost;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub model_id: String,
    pub provider: String,
    pub input_cost_per_mtok: TokenCost,
    pub output_cost_per_mtok: TokenCost,
    /// Discounted rate for tokens read from prompt cache (None = no caching support)
    pub cached_input_cost_per_mtok: Option<TokenCost>,
    /// Surcharge rate for writing tokens into prompt cache (None = no write surcharge)
    pub cache_write_cost_per_mtok: Option<TokenCost>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub effective_date: chrono::DateTime<chrono::Utc>,
}

/// JSON-serializable pricing entry for runtime loading.
///
/// Agents fetch pricing from an API endpoint and deserialize into this format.
/// All cost fields are in cents per 1M (million) tokens — the unit cloud
/// rates are published in, and the only one that keeps sub-cent-per-1K rates
/// (e.g. GLM-5.2's $1.40/MTok = 0.14c/1K) from flooring to zero.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingEntry {
    pub model_id: String,
    pub provider: String,
    /// Input cost in cents per 1M tokens
    pub input_cents_per_mtok: u64,
    /// Output cost in cents per 1M tokens
    pub output_cents_per_mtok: u64,
    /// Cached input cost in cents per 1M tokens (None = no caching)
    #[serde(default)]
    pub cached_input_cents_per_mtok: Option<u64>,
    /// Cache write cost in cents per 1M tokens (None = no write surcharge)
    #[serde(default)]
    pub cache_write_cents_per_mtok: Option<u64>,
    #[serde(default)]
    pub context_window: Option<u32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

impl PricingEntry {
    fn to_model_pricing(&self) -> ModelPricing {
        ModelPricing {
            model_id: self.model_id.clone(),
            provider: self.provider.clone(),
            input_cost_per_mtok: TokenCost::from_cents(self.input_cents_per_mtok),
            output_cost_per_mtok: TokenCost::from_cents(self.output_cents_per_mtok),
            cached_input_cost_per_mtok: self.cached_input_cents_per_mtok.map(TokenCost::from_cents),
            cache_write_cost_per_mtok: self.cache_write_cents_per_mtok.map(TokenCost::from_cents),
            context_window: self.context_window,
            max_output_tokens: self.max_output_tokens,
            effective_date: chrono::Utc::now(),
        }
    }
}

/// Runtime pricing registry. Starts empty — populated via `load_from_json()`,
/// `register()`, or the `budget.pricing` API endpoint.
///
/// Unknown models are treated as zero-cost (local) rather than returning errors.
#[derive(Debug, Clone)]
pub struct ProviderPricing {
    models: HashMap<String, ModelPricing>,
}

impl ProviderPricing {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
        }
    }

    pub fn get_model_pricing(&self, provider: &str, model: &str) -> Option<&ModelPricing> {
        let key = format!("{provider}:{model}");
        self.models.get(&key)
    }

    pub fn add_model_pricing(&mut self, pricing: ModelPricing) {
        let key = format!("{}:{}", pricing.provider, pricing.model_id);
        self.models.insert(key, pricing);
    }

    /// Register a single model from a `PricingEntry`.
    pub fn register(&mut self, entry: &PricingEntry) {
        self.add_model_pricing(entry.to_model_pricing());
    }

    /// Bulk-load pricing from a JSON array of `PricingEntry` objects.
    ///
    /// Agents call this after fetching pricing from an API endpoint:
    /// ```ignore
    /// let body: Vec<PricingEntry> = reqwest::get(url).await?.json().await?;
    /// pricing.load_from_entries(&body);
    /// ```
    pub fn load_from_entries(&mut self, entries: &[PricingEntry]) {
        for entry in entries {
            self.register(entry);
        }
    }

    /// Load pricing from a JSON value (array of pricing entries).
    pub fn load_from_json(&mut self, value: &serde_json::Value) -> Result<usize, String> {
        let entries: Vec<PricingEntry> =
            serde_json::from_value(value.clone()).map_err(|e| e.to_string())?;
        let count = entries.len();
        self.load_from_entries(&entries);
        Ok(count)
    }

    /// Export all pricing as a JSON-serializable list.
    pub fn export_entries(&self) -> Vec<PricingEntry> {
        self.models
            .values()
            .map(|m| PricingEntry {
                model_id: m.model_id.clone(),
                provider: m.provider.clone(),
                input_cents_per_mtok: m.input_cost_per_mtok.as_cents(),
                output_cents_per_mtok: m.output_cost_per_mtok.as_cents(),
                cached_input_cents_per_mtok: m.cached_input_cost_per_mtok.map(|c| c.as_cents()),
                cache_write_cents_per_mtok: m.cache_write_cost_per_mtok.map(|c| c.as_cents()),
                context_window: m.context_window,
                max_output_tokens: m.max_output_tokens,
            })
            .collect()
    }

    pub fn model_count(&self) -> usize {
        self.models.len()
    }

    pub fn estimate_cost(
        &self,
        provider: &str,
        model: &str,
        estimated_input_tokens: u32,
        estimated_output_tokens: u32,
    ) -> Option<TokenCost> {
        self.get_model_pricing(provider, model).map(|pricing| {
            let input_cost = TokenCost::from_tokens_per_million(
                estimated_input_tokens,
                pricing.input_cost_per_mtok,
            );
            let output_cost = TokenCost::from_tokens_per_million(
                estimated_output_tokens,
                pricing.output_cost_per_mtok,
            );
            input_cost + output_cost
        })
    }

    /// Estimate cost accounting for prompt-cache hits.
    pub fn estimate_cost_with_cache(
        &self,
        provider: &str,
        model: &str,
        usage: &crate::cost::TokenUsage,
    ) -> Option<TokenCost> {
        self.get_model_pricing(provider, model).map(|pricing| {
            let input_cost =
                TokenCost::from_tokens_per_million(usage.input_tokens, pricing.input_cost_per_mtok);
            // Thinking tokens bill at the output rate (Gemini pricing rule).
            let billable_output = usage.output_tokens.saturating_add(usage.thinking_tokens);
            let output_cost =
                TokenCost::from_tokens_per_million(billable_output, pricing.output_cost_per_mtok);

            let cached_rate = pricing
                .cached_input_cost_per_mtok
                .unwrap_or(pricing.input_cost_per_mtok);
            let cached_cost =
                TokenCost::from_tokens_per_million(usage.cached_input_tokens, cached_rate);

            let write_rate = pricing
                .cache_write_cost_per_mtok
                .unwrap_or(pricing.input_cost_per_mtok);
            let write_cost =
                TokenCost::from_tokens_per_million(usage.cache_write_tokens, write_rate);

            input_cost + output_cost + cached_cost + write_cost
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
                    required_context / 2,
                    required_output / 2,
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

/// Thread-safe shared pricing registry for runtime updates.
pub type SharedPricing = Arc<RwLock<ProviderPricing>>;

pub fn new_shared_pricing() -> SharedPricing {
    Arc::new(RwLock::new(ProviderPricing::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cost::TokenUsage;

    fn sample_entries() -> Vec<PricingEntry> {
        vec![
            PricingEntry {
                model_id: "gpt-4o".into(),
                provider: "openai".into(),
                input_cents_per_mtok: 250,
                output_cents_per_mtok: 1000,
                cached_input_cents_per_mtok: Some(125),
                cache_write_cents_per_mtok: None,
                context_window: Some(128000),
                max_output_tokens: Some(16384),
            },
            PricingEntry {
                model_id: "gemini-3.5-flash".into(),
                provider: "google".into(),
                // Gemini 3.5 Flash (May 2026): $1.50/MTok input = 150 cents/MTok.
                // Per-MTok integer cents represents this exactly; the real
                // registry overrides via the budget.pricing API.
                input_cents_per_mtok: 150,
                output_cents_per_mtok: 900,
                cached_input_cents_per_mtok: Some(15),
                cache_write_cents_per_mtok: None,
                context_window: Some(1_048_576),
                max_output_tokens: Some(8192),
            },
            PricingEntry {
                model_id: "local".into(),
                provider: "local".into(),
                input_cents_per_mtok: 0,
                output_cents_per_mtok: 0,
                cached_input_cents_per_mtok: None,
                cache_write_cents_per_mtok: None,
                context_window: Some(1_048_576),
                max_output_tokens: Some(16384),
            },
        ]
    }

    #[test]
    fn test_starts_empty() {
        let pricing = ProviderPricing::new();
        assert_eq!(pricing.model_count(), 0);
        assert!(pricing.get_model_pricing("openai", "gpt-4o").is_none());
    }

    #[test]
    fn test_load_from_entries() {
        let mut pricing = ProviderPricing::new();
        pricing.load_from_entries(&sample_entries());
        assert_eq!(pricing.model_count(), 3);

        let gpt4o = pricing.get_model_pricing("openai", "gpt-4o").unwrap();
        assert_eq!(gpt4o.input_cost_per_mtok.as_cents(), 250);
        assert_eq!(gpt4o.cached_input_cost_per_mtok.unwrap().as_cents(), 125);
    }

    #[test]
    fn test_load_from_json() {
        let json = serde_json::to_value(sample_entries()).unwrap();
        let mut pricing = ProviderPricing::new();
        let count = pricing.load_from_json(&json).unwrap();
        assert_eq!(count, 3);
        assert!(
            pricing
                .get_model_pricing("google", "gemini-3.5-flash")
                .is_some()
        );
    }

    #[test]
    fn test_export_roundtrip() {
        let mut pricing = ProviderPricing::new();
        pricing.load_from_entries(&sample_entries());

        let exported = pricing.export_entries();
        let mut pricing2 = ProviderPricing::new();
        pricing2.load_from_entries(&exported);

        assert_eq!(pricing.model_count(), pricing2.model_count());
        let p1 = pricing.get_model_pricing("openai", "gpt-4o").unwrap();
        let p2 = pricing2.get_model_pricing("openai", "gpt-4o").unwrap();
        assert_eq!(p1.input_cost_per_mtok, p2.input_cost_per_mtok);
    }

    #[test]
    fn test_cost_estimation() {
        let mut pricing = ProviderPricing::new();
        pricing.load_from_entries(&sample_entries());

        // gpt-4o at $2.50/MTok in, $10/MTok out: 1M in + 0.5M out =
        // 1_000_000*250/1e6 + 500_000*1000/1e6 = 250 + 500 = 750 cents.
        let cost = pricing
            .estimate_cost("openai", "gpt-4o", 1_000_000, 500_000)
            .unwrap();
        assert_eq!(cost.as_cents(), 750);
    }

    #[test]
    fn test_glm52_priced_at_per_mtok_resolution() {
        // GLM-5.2 is $1.40/$4.40 per MTok. In the old per-1K-cents unit these
        // rounded to 0 (the bug this migration fixes); per-MTok they are
        // 140/440 cents and a realistic call prices correctly.
        let mut pricing = ProviderPricing::new();
        pricing.register(&PricingEntry {
            model_id: "glm-5.2".into(),
            provider: "zhipu".into(),
            input_cents_per_mtok: 140,
            output_cents_per_mtok: 440,
            cached_input_cents_per_mtok: Some(26),
            cache_write_cents_per_mtok: None,
            context_window: Some(1_000_000),
            max_output_tokens: Some(131_072),
        });
        // 200K in + 100K out = 200_000*140/1e6 + 100_000*440/1e6 = 28 + 44 = 72c.
        let cost = pricing
            .estimate_cost("zhipu", "glm-5.2", 200_000, 100_000)
            .unwrap();
        assert_eq!(cost.as_cents(), 72);
    }

    #[test]
    fn test_grok46_priced_at_published_list_rates() {
        // Grok 4.6: $2.00/$6.00 per MTok (+ $0.50 cached input) below 200k.
        // cents/MTok: 200 input, 600 output, 50 cached.
        let mut pricing = ProviderPricing::new();
        pricing.register(&PricingEntry {
            model_id: "grok-4.6".into(),
            provider: "xai".into(),
            input_cents_per_mtok: 200,
            output_cents_per_mtok: 600,
            cached_input_cents_per_mtok: Some(50),
            cache_write_cents_per_mtok: None,
            context_window: Some(500_000),
            max_output_tokens: Some(131_072),
        });
        // 200K in + 100K out = 200_000*200/1e6 + 100_000*600/1e6 = 40 + 60 = 100c.
        let cost = pricing
            .estimate_cost("xai", "grok-4.6", 200_000, 100_000)
            .unwrap();
        assert_eq!(cost.as_cents(), 100);
    }

    #[test]
    fn test_cache_cost_cheaper_than_standard() {
        let mut pricing = ProviderPricing::new();
        pricing.load_from_entries(&sample_entries());

        // 1M tokens so per-MTok rates produce non-zero, comparable cents.
        let standard = pricing
            .estimate_cost("openai", "gpt-4o", 1_000_000, 0)
            .unwrap();
        let usage = TokenUsage::with_cache(0, 0, 1_000_000, 0);
        let cached = pricing
            .estimate_cost_with_cache("openai", "gpt-4o", &usage)
            .unwrap();

        assert!(cached.as_cents() < standard.as_cents());
    }

    #[test]
    fn test_unknown_model_returns_none() {
        let pricing = ProviderPricing::new();
        assert!(
            pricing
                .estimate_cost("openai", "nonexistent", 100, 100)
                .is_none()
        );
    }

    #[test]
    fn test_register_overwrites() {
        let mut pricing = ProviderPricing::new();
        pricing.load_from_entries(&sample_entries());

        // Update price
        pricing.register(&PricingEntry {
            model_id: "gpt-4o".into(),
            provider: "openai".into(),
            input_cents_per_mtok: 200,
            output_cents_per_mtok: 800,
            cached_input_cents_per_mtok: Some(100),
            cache_write_cents_per_mtok: None,
            context_window: Some(128000),
            max_output_tokens: Some(16384),
        });

        let gpt4o = pricing.get_model_pricing("openai", "gpt-4o").unwrap();
        assert_eq!(gpt4o.input_cost_per_mtok.as_cents(), 200);
    }

    #[test]
    fn test_find_cheapest_model() {
        let mut pricing = ProviderPricing::new();
        pricing.load_from_entries(&sample_entries());

        // Large token requirements so per-MTok costs differentiate (small
        // counts all floor to 0c). gpt-4o is filtered out by context window;
        // the free local model beats paid gemini.
        let cheapest = pricing
            .find_cheapest_model_for_tokens(500_000, 4_000)
            .unwrap();
        assert_eq!(cheapest.model_id, "local");
    }
}
