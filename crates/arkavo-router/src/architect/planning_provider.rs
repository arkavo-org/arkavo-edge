//! Which model plans, and how its provider is built.
//!
//! Kept apart from the planner so the choice and the construction can be read
//! (and substituted) independently of the planning prompt and parsing.
use crate::decision::ModelChoice;
use crate::error::{Error, Result};
use crate::selector::ProviderAvailability;
use arkavo_llm::Provider;

/// Planning model for the configured providers, best-quality first. The
/// returned arm is what the ledger will be charged, so it must match the
/// client [`build`] hands back.
///
/// `None` means no configured provider can plan — the ordinary state of a
/// key-less local install, not an error. Callers decide what that costs them:
/// `Router::route` skips architect mode, while `create_plan` (which was asked
/// for a plan outright) turns it into [`no_planning_model`].
pub(super) fn choose_model(availability: &ProviderAvailability) -> Option<ModelChoice> {
    // Anthropic first: highest planning quality. `AnthropicProvider::from_env`
    // defaults to Sonnet 4.5.
    #[cfg(feature = "llm-remote")]
    if availability.anthropic {
        return Some(ModelChoice::ClaudeSonnet);
    }
    // DeepSeek V3.2-Speciale is reasoning-only and cost-effective for planning.
    #[cfg(feature = "deepseek")]
    if availability.deepseek {
        return Some(ModelChoice::DeepSeekV32Speciale);
    }
    #[cfg(feature = "gemini")]
    if availability.gemini {
        return Some(ModelChoice::Gemini35Flash);
    }
    #[cfg(feature = "openai")]
    if availability.openai {
        return Some(ModelChoice::Gpt6Astra);
    }
    let _ = availability;
    None
}

/// The error a caller that genuinely needs a plan reports when
/// [`choose_model`] finds nothing configured.
pub(super) fn no_planning_model() -> Error {
    Error::ModelExecution(
        "No planning model available. Set ANTHROPIC_API_KEY, DEEPSEEK_API_KEY, GEMINI_API_KEY, or OPENAI_API_KEY.".to_string(),
    )
}

/// Live client for a planning arm, for planners with no router attached.
pub(super) fn build(model: &ModelChoice) -> Result<Box<dyn Provider>> {
    match model {
        #[cfg(feature = "llm-remote")]
        ModelChoice::ClaudeSonnet => {
            use arkavo_llm::providers::anthropic::AnthropicProvider;
            AnthropicProvider::from_env()
                .map(|provider| protect(Box::new(provider)))
                .map_err(|e| Error::ModelExecution(e.to_string()))
        }
        #[cfg(feature = "deepseek")]
        ModelChoice::DeepSeekV32Speciale => arkavo_llm::DeepSeekProvider::v32_speciale()
            .map(|provider| protect(Box::new(provider)))
            .map_err(|e| Error::ModelExecution(e.to_string())),
        #[cfg(feature = "gemini")]
        ModelChoice::Gemini35Flash => arkavo_llm::GeminiProvider::new()
            .map(|provider| protect(Box::new(provider)))
            .map_err(|e| Error::ModelExecution(e.to_string())),
        #[cfg(feature = "openai")]
        ModelChoice::Gpt6Astra => {
            use arkavo_llm::providers::{OpenAIResponsesConfig, OpenAIResponsesProvider};
            OpenAIResponsesProvider::new(OpenAIResponsesConfig::default())
                .map(|provider| protect(Box::new(provider)))
                .map_err(|e| Error::ModelExecution(e.to_string()))
        }
        other => Err(Error::ModelExecution(format!(
            "No planning provider for {}",
            other.name()
        ))),
    }
}

#[cfg(any(feature = "llm-remote", feature = "deepseek", feature = "gemini"))]
fn protect(provider: Box<dyn Provider>) -> Box<dyn Provider> {
    #[cfg(feature = "sentinel")]
    {
        crate::response_policy::protect(provider)
    }
    #[cfg(not(feature = "sentinel"))]
    {
        provider
    }
}
