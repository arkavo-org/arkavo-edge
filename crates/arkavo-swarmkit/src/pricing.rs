//! Manifest-level model pricing table.
//!
//! Authored at SwarmKit authoring time (and, later, via the UI) and read at
//! runtime to populate the budget cost model. Prices are dynamic but **never
//! fetched from a vendor endpoint at runtime** — they are curated config that
//! travels inside the signed manifest. Rates are cents per 1M (million) tokens,
//! matching `arkavo_budget::PricingEntry` so the runtime conversion is direct.

use serde::{Deserialize, Serialize};

/// One model's published rates, in cents per 1M tokens.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelPricingEntry {
    pub model_id: String,
    pub provider: String,
    /// Input cost, cents per 1M tokens.
    pub input_cents_per_mtok: u64,
    /// Output cost, cents per 1M tokens.
    pub output_cents_per_mtok: u64,
    /// Cached-input read cost, cents per 1M tokens (None = no cache discount).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_input_cents_per_mtok: Option<u64>,
    /// Cache-write surcharge, cents per 1M tokens (None = no surcharge).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_cents_per_mtok: Option<u64>,
}

impl From<&ModelPricingEntry> for arkavo_budget::PricingEntry {
    /// Lossless on the six shared fields (same names, same cents-per-MTok
    /// units). The two `PricingEntry`-only fields (`context_window`,
    /// `max_output_tokens`) have no manifest source and default to `None`.
    fn from(entry: &ModelPricingEntry) -> Self {
        arkavo_budget::PricingEntry {
            model_id: entry.model_id.clone(),
            provider: entry.provider.clone(),
            input_cents_per_mtok: entry.input_cents_per_mtok,
            output_cents_per_mtok: entry.output_cents_per_mtok,
            cached_input_cents_per_mtok: entry.cached_input_cents_per_mtok,
            cache_write_cents_per_mtok: entry.cache_write_cents_per_mtok,
            context_window: None,
            max_output_tokens: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pricing_entry_parses_glm_rates() {
        let json = r#"{
            "model_id": "glm-5.2",
            "provider": "zhipu",
            "input_cents_per_mtok": 140,
            "output_cents_per_mtok": 440,
            "cached_input_cents_per_mtok": 26
        }"#;
        let entry: ModelPricingEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.model_id, "glm-5.2");
        assert_eq!(entry.provider, "zhipu");
        assert_eq!(entry.input_cents_per_mtok, 140);
        assert_eq!(entry.output_cents_per_mtok, 440);
        assert_eq!(entry.cached_input_cents_per_mtok, Some(26));
        assert_eq!(entry.cache_write_cents_per_mtok, None);
    }

    #[test]
    fn pricing_entry_cache_fields_optional() {
        // Minimal entry (no cache rates) must parse — cache discount is optional.
        let json = r#"{
            "model_id": "claude-opus-4-8",
            "provider": "anthropic",
            "input_cents_per_mtok": 500,
            "output_cents_per_mtok": 2500
        }"#;
        let entry: ModelPricingEntry = serde_json::from_str(json).unwrap();
        assert_eq!(entry.cached_input_cents_per_mtok, None);
        // Absent optional rates are skipped on re-serialize (canonical-form stable).
        let round = serde_json::to_string(&entry).unwrap();
        assert!(!round.contains("cached_input_cents_per_mtok"));
    }

    #[test]
    fn manifest_entry_converts_to_budget_pricing_entry() {
        // Regression for #635: the manifest→budget conversion is lossless on
        // the six shared fields and defaults the two budget-only fields.
        let manifest_entry = ModelPricingEntry {
            model_id: "glm-5.2".to_string(),
            provider: "zhipu".to_string(),
            input_cents_per_mtok: 140,
            output_cents_per_mtok: 440,
            cached_input_cents_per_mtok: Some(26),
            cache_write_cents_per_mtok: Some(70),
        };
        let pricing_entry: arkavo_budget::PricingEntry = (&manifest_entry).into();

        assert_eq!(pricing_entry.model_id, manifest_entry.model_id);
        assert_eq!(pricing_entry.provider, manifest_entry.provider);
        assert_eq!(
            pricing_entry.input_cents_per_mtok,
            manifest_entry.input_cents_per_mtok
        );
        assert_eq!(
            pricing_entry.output_cents_per_mtok,
            manifest_entry.output_cents_per_mtok
        );
        assert_eq!(
            pricing_entry.cached_input_cents_per_mtok,
            manifest_entry.cached_input_cents_per_mtok
        );
        assert_eq!(
            pricing_entry.cache_write_cents_per_mtok,
            manifest_entry.cache_write_cents_per_mtok
        );
        // No manifest source for these two; must default to None.
        assert_eq!(pricing_entry.context_window, None);
        assert_eq!(pricing_entry.max_output_tokens, None);
    }

    #[test]
    fn manifest_entry_converts_then_loads_into_provider_pricing() {
        // The full send-side path: manifest entry → PricingEntry → registry,
        // which estimate_cost must then honor as the authored rate.
        use arkavo_budget::provider_costs::ProviderPricing;

        let manifest_entry = ModelPricingEntry {
            model_id: "glm-5.2".to_string(),
            provider: "zhipu".to_string(),
            // 140 cents = $1.40 / MTok input; 440 cents = $4.40 / MTok output.
            input_cents_per_mtok: 140,
            output_cents_per_mtok: 440,
            cached_input_cents_per_mtok: None,
            cache_write_cents_per_mtok: None,
        };
        let entries: Vec<arkavo_budget::PricingEntry> = [(&manifest_entry).into()].into();
        let mut pricing = ProviderPricing::new();
        pricing.load_from_entries(&entries);

        // 1M input + 1M output → 140 + 440 = 580 cents = $5.80.
        let cost = pricing
            .estimate_cost("zhipu", "glm-5.2", 1_000_000, 1_000_000)
            .expect("authored model must be priced");
        assert!((cost.as_dollars() - 5.80).abs() < 1e-9);
    }
}
