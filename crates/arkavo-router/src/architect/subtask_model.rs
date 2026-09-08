//! Which arm executes each planned subtask, and what that arm is expected to cost.
//!
//! Split out of the planner because assignment answers two questions the plan
//! itself does not: what the category wants, and what this device can actually
//! run. The second one is why an unprovisioned local weight is never assigned —
//! the executor would otherwise have to download it in the middle of a plan.
use crate::Router;
use crate::classifier::TaskCategory;
use crate::decision::ModelChoice;
use crate::selector::ProviderAvailability;
use std::sync::Arc;

/// Arm for a subtask in `category`.
///
/// The category's preference is taken from the configured providers, then
/// filtered against what the device has: an unprovisioned local weight gives
/// way to cloud augmentation where a cloud provider is configured, and
/// otherwise to the fastest local weight that *is* on disk. A planner with no
/// router has nothing to ask, so its preference stands unfiltered and the
/// executor's own provisioning check is the backstop.
pub(super) fn select_model_for_category(
    availability: &ProviderAvailability,
    router: Option<&Arc<Router>>,
    category: TaskCategory,
) -> ModelChoice {
    let preferred = preferred_for_category(availability, category);
    if !preferred.is_local() {
        return preferred;
    }
    // A planner with no router has no selector to ask, so it must assume the
    // weight is absent rather than assign one the executor may have to fetch.
    if router.is_some_and(|router| router.require_provisioned(&preferred).is_ok()) {
        return preferred;
    }
    // Cloud augmentation must not displace a provisioned local subtask model,
    // so it is only reached once the preferred local arm is known missing.
    crate::ModelSelector::with_availability(availability.clone(), false)
        .cloud_augmentation_model()
        .or_else(|| router.and_then(|router| router.selector.fastest_cached_local_model()))
        .unwrap_or(preferred)
}

/// The arm the category wants from the providers this planner is configured
/// with, before any check of what the device has on disk.
fn preferred_for_category(
    availability: &ProviderAvailability,
    category: TaskCategory,
) -> ModelChoice {
    match category {
        // Frontend tasks: Use cheaper, fast models
        TaskCategory::FrontendUI => {
            if availability.gemini {
                ModelChoice::Gemini35Flash
            } else if availability.anthropic {
                ModelChoice::ClaudeSonnet
            } else {
                ModelChoice::LocalMinistral3B
            }
        }

        // Backend/Security/Tests/Review: Use more capable models
        TaskCategory::BackendAPI
        | TaskCategory::SecurityScan
        | TaskCategory::TestGeneration
        | TaskCategory::CodeReview => {
            if availability.anthropic {
                ModelChoice::ClaudeOpus
            } else if availability.gemini {
                ModelChoice::GeminiPro
            } else {
                ModelChoice::LocalMinistral8B
            }
        }

        // Documentation: Use cheaper models
        TaskCategory::Documentation => {
            if availability.gemini {
                ModelChoice::Gemini35Flash
            } else {
                ModelChoice::LocalQwen3
            }
        }

        // Refactoring: Use balanced models
        TaskCategory::Refactoring | TaskCategory::CodeGeneration => {
            if availability.anthropic {
                ModelChoice::ClaudeSonnet
            } else if availability.gemini {
                ModelChoice::GeminiPro
            } else {
                ModelChoice::LocalMinistral3B
            }
        }

        // Code search: Local model is sufficient
        TaskCategory::CodeSearch => ModelChoice::LocalQwen3,

        // Vision: Needs multimodal
        TaskCategory::VisionAnalysis => {
            if availability.gemini {
                ModelChoice::Gemini35Flash
            } else if availability.anthropic {
                ModelChoice::ClaudeSonnet
            } else {
                ModelChoice::LocalMinistral3B
            }
        }

        // Game/simulation and general: Use balanced default
        TaskCategory::GameSimulation | TaskCategory::General => {
            if availability.anthropic {
                ModelChoice::ClaudeSonnet
            } else if availability.gemini {
                ModelChoice::Gemini35Flash
            } else {
                ModelChoice::LocalQwen3
            }
        }
    }
}

/// What one subtask on `model` is expected to cost, priced per arm where the
/// published rate differs from the generic estimate.
pub(super) fn estimate_subtask_cost(model: &ModelChoice, category: TaskCategory) -> f64 {
    let token_estimate = category.estimated_tokens();
    let per_mtok = |input: f64, output: f64| {
        let input_cost = (token_estimate.input as f64 / 1_000_000.0) * input;
        let output_cost = (token_estimate.output as f64 / 1_000_000.0) * output;
        input_cost + output_cost
    };

    match model {
        ModelChoice::GeminiFlash => per_mtok(0.30, 2.50),
        ModelChoice::Gemini35Flash => per_mtok(1.50, 9.00),
        ModelChoice::GeminiPro => per_mtok(1.25, 5.00),
        ModelChoice::ClaudeSonnet => per_mtok(3.00, 15.00),
        // Opus 4.8 pricing ($5/$25); the old $15/$75 was Opus 4.1.
        ModelChoice::ClaudeOpus => per_mtok(5.00, 25.00),
        ModelChoice::ClaudeFable5 => per_mtok(10.00, 50.00),
        _ => crate::RoutingDecision::estimate_cost(model, category),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::only;
    use arkavo_test_macros::spec;

    /// Regression: assignment used to hand back the category's preferred local
    /// arm whether or not its weights were on disk, so the executor reached
    /// `load_local_model` and downloaded gigabytes mid-plan. With cloud
    /// configured, augmentation takes the subtask instead.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn an_unprovisioned_local_subtask_gives_way_to_cloud_augmentation() {
        let availability = only("gemini");
        let router = Arc::new(
            Router::new_offline()
                .await
                .unwrap()
                .with_selector(crate::ModelSelector::with_availability(
                    availability.clone(),
                    false,
                ))
                .await,
        );

        // CodeSearch prefers LocalQwen3 regardless of configured providers.
        let model =
            select_model_for_category(&availability, Some(&router), TaskCategory::CodeSearch);
        assert!(!model.is_local(), "got {model:?}");
    }

    /// The same assignment on a provisioned device keeps the local arm: cloud
    /// augments local inference, it does not replace it.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn a_provisioned_local_subtask_is_kept() {
        let availability = only("gemini");
        let router = Arc::new(
            Router::new_offline()
                .await
                .unwrap()
                .with_selector(crate::ModelSelector::with_availability(
                    availability.clone(),
                    true,
                ))
                .await,
        );

        assert_eq!(
            select_model_for_category(&availability, Some(&router), TaskCategory::CodeSearch),
            ModelChoice::LocalQwen3
        );
    }

    /// With no cloud configured and nothing on disk there is no runnable arm to
    /// name; assignment falls back to the preference and the executor's
    /// provisioning check refuses the dispatch.
    #[spec("ASTRA-004")]
    #[tokio::test]
    async fn a_bare_device_keeps_the_preference_for_the_executor_to_refuse() {
        let availability = ProviderAvailability::default();
        let router = Arc::new(
            Router::new_offline()
                .await
                .unwrap()
                .with_selector(crate::ModelSelector::with_availability(
                    availability.clone(),
                    false,
                ))
                .await,
        );

        assert_eq!(
            select_model_for_category(&availability, Some(&router), TaskCategory::CodeSearch),
            ModelChoice::LocalQwen3
        );
    }
}
