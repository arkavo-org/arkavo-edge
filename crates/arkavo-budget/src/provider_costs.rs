use crate::cost::TokenCost;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelPricing {
    pub model_id: String,
    pub provider: String,
    pub input_cost_per_thousand: TokenCost,
    pub output_cost_per_thousand: TokenCost,
    /// Discounted rate for tokens read from prompt cache (None = no caching support)
    pub cached_input_cost_per_thousand: Option<TokenCost>,
    /// Surcharge rate for writing tokens into prompt cache (None = no write surcharge)
    pub cache_write_cost_per_thousand: Option<TokenCost>,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
    pub effective_date: chrono::DateTime<chrono::Utc>,
}

/// JSON-serializable pricing entry for runtime loading.
///
/// Agents fetch pricing from an API endpoint and deserialize into this format.
/// All cost fields are in cents per 1K tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricingEntry {
    pub model_id: String,
    pub provider: String,
    /// Input cost in cents per 1K tokens
    pub input_cents_per_1k: u64,
    /// Output cost in cents per 1K tokens
    pub output_cents_per_1k: u64,
    /// Cached input cost in cents per 1K tokens (None = no caching)
    #[serde(default)]
    pub cached_input_cents_per_1k: Option<u64>,
    /// Cache write cost in cents per 1K tokens (None = no write surcharge)
    #[serde(default)]
    pub cache_write_cents_per_1k: Option<u64>,
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
            input_cost_per_thousand: TokenCost::from_cents(self.input_cents_per_1k),
            output_cost_per_thousand: TokenCost::from_cents(self.output_cents_per_1k),
            cached_input_cost_per_thousand: self
                .cached_input_cents_per_1k
                .map(TokenCost::from_cents),
            cache_write_cost_per_thousand: self.cache_write_cents_per_1k.map(TokenCost::from_cents),
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
                input_cents_per_1k: m.input_cost_per_thousand.as_cents(),
                output_cents_per_1k: m.output_cost_per_thousand.as_cents(),
                cached_input_cents_per_1k: m.cached_input_cost_per_thousand.map(|c| c.as_cents()),
                cache_write_cents_per_1k: m.cache_write_cost_per_thousand.map(|c| c.as_cents()),
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
            let input_cost =
                TokenCost::from_tokens(estimated_input_tokens, pricing.input_cost_per_thousand);
            let output_cost =
                TokenCost::from_tokens(estimated_output_tokens, pricing.output_cost_per_thousand);
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
                TokenCost::from_tokens(usage.input_tokens, pricing.input_cost_per_thousand);
            let output_cost =
                TokenCost::from_tokens(usage.output_tokens, pricing.output_cost_per_thousand);

            let cached_rate = pricing
                .cached_input_cost_per_thousand
                .unwrap_or(pricing.input_cost_per_thousand);
            let cached_cost = TokenCost::from_tokens(usage.cached_input_tokens, cached_rate);

            let write_rate = pricing
                .cache_write_cost_per_thousand
                .unwrap_or(pricing.input_cost_per_thousand);
            let write_cost = TokenCost::from_tokens(usage.cache_write_tokens, write_rate);

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
                input_cents_per_1k: 250,
                output_cents_per_1k: 1000,
                cached_input_cents_per_1k: Some(125),
                cache_write_cents_per_1k: None,
                context_window: Some(128000),
                max_output_tokens: Some(16384),
            },
            PricingEntry {
                model_id: "gemini-2.0-flash".into(),
                provider: "google".into(),
                input_cents_per_1k: 10,
                output_cents_per_1k: 40,
                cached_input_cents_per_1k: Some(2),
                cache_write_cents_per_1k: None,
                context_window: Some(1048576),
                max_output_tokens: Some(8192),
            },
            PricingEntry {
                model_id: "local".into(),
                provider: "local".into(),
                input_cents_per_1k: 0,
                output_cents_per_1k: 0,
                cached_input_cents_per_1k: None,
                cache_write_cents_per_1k: None,
                context_window: Some(2048),
                max_output_tokens: Some(512),
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
        assert_eq!(gpt4o.input_cost_per_thousand.as_cents(), 250);
        assert_eq!(
            gpt4o.cached_input_cost_per_thousand.unwrap().as_cents(),
            125
        );
    }

    #[test]
    fn test_load_from_json() {
        let json = serde_json::to_value(sample_entries()).unwrap();
        let mut pricing = ProviderPricing::new();
        let count = pricing.load_from_json(&json).unwrap();
        assert_eq!(count, 3);
        assert!(
            pricing
                .get_model_pricing("google", "gemini-2.0-flash")
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
        assert_eq!(p1.input_cost_per_thousand, p2.input_cost_per_thousand);
    }

    #[test]
    fn test_cost_estimation() {
        let mut pricing = ProviderPricing::new();
        pricing.load_from_entries(&sample_entries());

        let cost = pricing
            .estimate_cost("openai", "gpt-4o", 1000, 500)
            .unwrap();
        // 1000 * 250/1000 + 500 * 1000/1000 = 250 + 500 = 750
        assert_eq!(cost.as_cents(), 750);
    }

    #[test]
    fn test_cache_cost_cheaper_than_standard() {
        let mut pricing = ProviderPricing::new();
        pricing.load_from_entries(&sample_entries());

        let standard = pricing.estimate_cost("openai", "gpt-4o", 1000, 0).unwrap();
        let usage = TokenUsage::with_cache(0, 0, 1000, 0);
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
            input_cents_per_1k: 200,
            output_cents_per_1k: 800,
            cached_input_cents_per_1k: Some(100),
            cache_write_cents_per_1k: None,
            context_window: Some(128000),
            max_output_tokens: Some(16384),
        });

        let gpt4o = pricing.get_model_pricing("openai", "gpt-4o").unwrap();
        assert_eq!(gpt4o.input_cost_per_thousand.as_cents(), 200);
    }

    #[test]
    fn test_find_cheapest_model() {
        let mut pricing = ProviderPricing::new();
        pricing.load_from_entries(&sample_entries());

        let cheapest = pricing.find_cheapest_model_for_tokens(1000, 500).unwrap();
        assert_eq!(cheapest.model_id, "local");
    }
}
