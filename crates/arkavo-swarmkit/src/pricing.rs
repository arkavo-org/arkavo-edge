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
}
