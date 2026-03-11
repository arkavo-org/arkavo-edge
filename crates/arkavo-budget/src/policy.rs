use crate::cost::TokenCost;
use crate::provider_costs::{ModelPricing, ProviderPricing};
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionCriteria {
    pub required_capabilities: Vec<String>,
    pub preferred_providers: Vec<String>,
    pub max_acceptable_cost: Option<TokenCost>,
    pub estimated_input_tokens: u32,
    pub estimated_output_tokens: u32,
    pub priority: SelectionPriority,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SelectionPriority {
    LowestCost,
    BestPerformance,
    Balanced,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCandidate {
    pub provider: String,
    pub model_id: String,
    pub estimated_cost: TokenCost,
    pub pricing: ModelPricing,
    pub score: f64,
}

pub struct ModelSelectionPolicy {
    pricing: ProviderPricing,
}

impl ModelSelectionPolicy {
    pub fn new() -> Self {
        Self {
            pricing: ProviderPricing::new(),
        }
    }
}

impl Default for ModelSelectionPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelSelectionPolicy {
    pub fn select_model(
        &self,
        criteria: &SelectionCriteria,
        available_budget: TokenCost,
        available_models: &[ModelInfo],
    ) -> Result<Option<ModelCandidate>> {
        let mut candidates = self.filter_candidates(criteria, available_models)?;

        candidates.retain(|c| c.estimated_cost <= available_budget);

        if let Some(max_cost) = criteria.max_acceptable_cost {
            candidates.retain(|c| c.estimated_cost <= max_cost);
        }

        if candidates.is_empty() {
            return Ok(None);
        }

        self.score_candidates(&mut candidates, criteria);

        candidates.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        Ok(candidates.into_iter().next())
    }

    fn filter_candidates(
        &self,
        criteria: &SelectionCriteria,
        available_models: &[ModelInfo],
    ) -> Result<Vec<ModelCandidate>> {
        let mut candidates = Vec::new();

        for model in available_models {
            if !self.meets_capability_requirements(model, &criteria.required_capabilities) {
                continue;
            }

            // First check if model has embedded pricing
            if let Some(model_pricing) = &model.pricing {
                let input_cost = TokenCost::from_cents(model_pricing.input_cost_per_thousand_cents);
                let output_cost =
                    TokenCost::from_cents(model_pricing.output_cost_per_thousand_cents);

                let cost = TokenCost::from_tokens(criteria.estimated_input_tokens, input_cost)
                    + TokenCost::from_tokens(criteria.estimated_output_tokens, output_cost);

                let pricing = crate::provider_costs::ModelPricing {
                    model_id: model.model_id.clone(),
                    provider: model.provider_name.clone(),
                    input_cost_per_thousand: input_cost,
                    output_cost_per_thousand: output_cost,
                    cached_input_cost_per_thousand: None,
                    cache_write_cost_per_thousand: None,
                    context_window: model.context_window,
                    max_output_tokens: model.max_output_tokens,
                    effective_date: model_pricing.effective_date,
                };

                candidates.push(ModelCandidate {
                    provider: model.provider_name.clone(),
                    model_id: model.model_id.clone(),
                    estimated_cost: cost,
                    pricing,
                    score: 0.0,
                });
            } else if let Some(pricing) = self
                .pricing
                .get_model_pricing(&model.provider_name, &model.model_id)
            {
                // Fall back to budget module's pricing data
                if let Some(cost) = self.pricing.estimate_cost(
                    &model.provider_name,
                    &model.model_id,
                    criteria.estimated_input_tokens,
                    criteria.estimated_output_tokens,
                ) {
                    candidates.push(ModelCandidate {
                        provider: model.provider_name.clone(),
                        model_id: model.model_id.clone(),
                        estimated_cost: cost,
                        pricing: pricing.clone(),
                        score: 0.0,
                    });
                }
            }
        }

        Ok(candidates)
    }

    fn meets_capability_requirements(
        &self,
        model: &ModelInfo,
        required_capabilities: &[String],
    ) -> bool {
        for capability in required_capabilities {
            match capability.as_str() {
                "streaming" => {
                    if !model.capabilities.supports_streaming {
                        return false;
                    }
                }
                "function_calling" => {
                    if !model.capabilities.supports_function_calling {
                        return false;
                    }
                }
                "vision" => {
                    if !model.capabilities.supports_vision {
                        return false;
                    }
                }
                "code_execution" => {
                    if !model.capabilities.supports_code_execution {
                        return false;
                    }
                }
                _ => {
                    if !model.capabilities.specializations.contains(capability)
                        && !model.tags.contains(capability)
                    {
                        return false;
                    }
                }
            }
        }
        true
    }

    // Scoring weights by priority mode
    const W_COST_LOW: f64 = 0.7;
    const W_CTX_LOW: f64 = 0.2;
    const W_PROV_LOW: f64 = 0.1;

    const W_COST_PERF: f64 = 0.2;
    const W_CTX_PERF: f64 = 0.5;
    const W_PROV_PERF: f64 = 0.3;

    const W_COST_BAL: f64 = 0.4;
    const W_CTX_BAL: f64 = 0.4;
    const W_PROV_BAL: f64 = 0.2;

    const FREE_MODEL_BONUS: f64 = 0.2;
    const NON_PREFERRED_PROVIDER_SCORE: f64 = 0.5;

    fn score_candidates(&self, candidates: &mut [ModelCandidate], criteria: &SelectionCriteria) {
        let min_cost = candidates
            .iter()
            .map(|c| c.estimated_cost.as_cents())
            .min()
            .unwrap_or(1);

        let max_context = candidates
            .iter()
            .map(|c| c.pricing.context_window.unwrap_or(0))
            .max()
            .unwrap_or(1);

        for candidate in candidates.iter_mut() {
            let cost_score = min_cost as f64 / candidate.estimated_cost.as_cents() as f64;

            let context_score =
                candidate.pricing.context_window.unwrap_or(0) as f64 / max_context as f64;

            let provider_score = if criteria.preferred_providers.contains(&candidate.provider) {
                1.0
            } else {
                Self::NON_PREFERRED_PROVIDER_SCORE
            };

            let free_bonus = if candidate.estimated_cost.is_zero() {
                Self::FREE_MODEL_BONUS
            } else {
                0.0
            };

            candidate.score = match criteria.priority {
                SelectionPriority::LowestCost => {
                    cost_score * Self::W_COST_LOW
                        + context_score * Self::W_CTX_LOW
                        + provider_score * Self::W_PROV_LOW
                        + free_bonus
                }
                SelectionPriority::BestPerformance => {
                    cost_score * Self::W_COST_PERF
                        + context_score * Self::W_CTX_PERF
                        + provider_score * Self::W_PROV_PERF
                }
                SelectionPriority::Balanced => {
                    cost_score * Self::W_COST_BAL
                        + context_score * Self::W_CTX_BAL
                        + provider_score * Self::W_PROV_BAL
                        + free_bonus
                }
            };
        }
    }

    pub fn recommend_fallback_models(
        &self,
        original_model: &str,
        available_budget: TokenCost,
        available_models: &[ModelInfo],
    ) -> Vec<ModelCandidate> {
        let criteria = SelectionCriteria {
            required_capabilities: vec![],
            preferred_providers: vec![],
            max_acceptable_cost: Some(available_budget),
            estimated_input_tokens: 1000,
            estimated_output_tokens: 500,
            priority: SelectionPriority::LowestCost,
        };

        if let Ok(mut candidates) = self.filter_candidates(&criteria, available_models) {
            candidates.retain(|c| c.model_id != original_model);
            candidates.retain(|c| c.estimated_cost <= available_budget);
            candidates.sort_by_key(|c| c.estimated_cost.as_cents());
            candidates.truncate(3);
            candidates
        } else {
            vec![]
        }
    }
}

use arkavo_dataflow::nodes::model_registry::ModelInfo;

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_dataflow::nodes::model_registry::ModelCapabilities;
    use chrono::Utc;

    fn create_test_models() -> Vec<ModelInfo> {
        vec![
            ModelInfo {
                model_id: "gpt-4".to_string(),
                provider_name: "openai".to_string(),
                display_name: "GPT-4".to_string(),
                description: None,
                context_window: Some(8192),
                max_output_tokens: Some(4096),
                capabilities: ModelCapabilities {
                    supports_streaming: true,
                    supports_function_calling: true,
                    supports_vision: false,
                    supports_code_execution: false,
                    input_modalities: vec!["text".to_string()],
                    output_modalities: vec!["text".to_string()],
                    languages: vec!["en".to_string()],
                    specializations: vec!["general".to_string()],
                },
                tags: vec!["general".to_string()],
                version: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                metadata: None,
                pricing: Some(arkavo_dataflow::nodes::model_registry::ModelPricing {
                    input_cost_per_thousand_cents: 3000,
                    output_cost_per_thousand_cents: 6000,
                    effective_date: Utc::now(),
                    currency: "USD".to_string(),
                }),
                snpe_metadata: None,
            },
            ModelInfo {
                model_id: "gpt-3.5-turbo".to_string(),
                provider_name: "openai".to_string(),
                display_name: "GPT-3.5 Turbo".to_string(),
                description: None,
                context_window: Some(16385),
                max_output_tokens: Some(4096),
                capabilities: ModelCapabilities {
                    supports_streaming: true,
                    supports_function_calling: true,
                    supports_vision: false,
                    supports_code_execution: false,
                    input_modalities: vec!["text".to_string()],
                    output_modalities: vec!["text".to_string()],
                    languages: vec!["en".to_string()],
                    specializations: vec!["general".to_string()],
                },
                tags: vec!["general".to_string(), "fast".to_string()],
                version: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                metadata: None,
                pricing: Some(arkavo_dataflow::nodes::model_registry::ModelPricing {
                    input_cost_per_thousand_cents: 50,
                    output_cost_per_thousand_cents: 150,
                    effective_date: Utc::now(),
                    currency: "USD".to_string(),
                }),
                snpe_metadata: None,
            },
            ModelInfo {
                model_id: "claude-3-haiku-20240307".to_string(),
                provider_name: "anthropic".to_string(),
                display_name: "Claude 3 Haiku".to_string(),
                description: None,
                context_window: Some(200000),
                max_output_tokens: Some(4096),
                capabilities: ModelCapabilities {
                    supports_streaming: true,
                    supports_function_calling: false,
                    supports_vision: true,
                    supports_code_execution: false,
                    input_modalities: vec!["text".to_string(), "image".to_string()],
                    output_modalities: vec!["text".to_string()],
                    languages: vec!["en".to_string()],
                    specializations: vec!["general".to_string()],
                },
                tags: vec!["general".to_string(), "efficient".to_string()],
                version: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                metadata: None,
                pricing: Some(arkavo_dataflow::nodes::model_registry::ModelPricing {
                    input_cost_per_thousand_cents: 25,
                    output_cost_per_thousand_cents: 125,
                    effective_date: Utc::now(),
                    currency: "USD".to_string(),
                }),
                snpe_metadata: None,
            },
            ModelInfo {
                model_id: "llama3.2:latest".to_string(),
                provider_name: "ollama".to_string(),
                display_name: "Llama 3.2".to_string(),
                description: None,
                context_window: Some(8192),
                max_output_tokens: Some(4096),
                capabilities: ModelCapabilities {
                    supports_streaming: true,
                    supports_function_calling: false,
                    supports_vision: false,
                    supports_code_execution: false,
                    input_modalities: vec!["text".to_string()],
                    output_modalities: vec!["text".to_string()],
                    languages: vec!["en".to_string()],
                    specializations: vec!["general".to_string()],
                },
                tags: vec!["general".to_string(), "local".to_string()],
                version: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                metadata: None,
                pricing: Some(arkavo_dataflow::nodes::model_registry::ModelPricing {
                    input_cost_per_thousand_cents: 0,
                    output_cost_per_thousand_cents: 0,
                    effective_date: Utc::now(),
                    currency: "USD".to_string(),
                }),
                snpe_metadata: None,
            },
        ]
    }

    #[test]
    fn test_model_selection_lowest_cost() {
        let policy = ModelSelectionPolicy::new();
        let models = create_test_models();

        let criteria = SelectionCriteria {
            required_capabilities: vec!["streaming".to_string()],
            preferred_providers: vec![],
            max_acceptable_cost: None,
            estimated_input_tokens: 1000,
            estimated_output_tokens: 500,
            priority: SelectionPriority::LowestCost,
        };

        let result = policy
            .select_model(&criteria, TokenCost::from_dollars(10.0), &models)
            .unwrap();

        assert!(result.is_some());
        let selected = result.unwrap();
        // Should select the cheapest model - either Ollama (free) or cheapest paid model
        assert!(selected.estimated_cost <= TokenCost::from_dollars(1.0));
    }

    #[test]
    fn test_model_selection_with_capability_filter() {
        let policy = ModelSelectionPolicy::new();
        let models = create_test_models();

        let criteria = SelectionCriteria {
            required_capabilities: vec!["vision".to_string()],
            preferred_providers: vec![],
            max_acceptable_cost: None,
            estimated_input_tokens: 1000,
            estimated_output_tokens: 500,
            priority: SelectionPriority::Balanced,
        };

        let result = policy
            .select_model(&criteria, TokenCost::from_dollars(10.0), &models)
            .unwrap();

        assert!(result.is_some());
        let selected = result.unwrap();
        assert_eq!(selected.model_id, "claude-3-haiku-20240307");
    }

    #[test]
    fn test_model_selection_budget_constraint() {
        let policy = ModelSelectionPolicy::new();
        let models = create_test_models();

        let criteria = SelectionCriteria {
            required_capabilities: vec![],
            preferred_providers: vec!["openai".to_string()],
            max_acceptable_cost: None,
            estimated_input_tokens: 10000,
            estimated_output_tokens: 5000,
            priority: SelectionPriority::Balanced,
        };

        let result = policy
            .select_model(&criteria, TokenCost::from_cents(100), &models)
            .unwrap();

        assert!(result.is_none() || result.unwrap().estimated_cost <= TokenCost::from_cents(100));
    }
}
