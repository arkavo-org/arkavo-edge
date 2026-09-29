//! Wall-clock budget for a single chat inference.
//!
//! The budget exists to stop a hung provider from parking a session forever,
//! not to police slow hardware: a local model on a modest machine can spend
//! minutes on prompt evaluation alone. The defaults are therefore generous,
//! and operators can set their own with [`CHAT_TIMEOUT_ENV`] without a
//! rebuild, the same way `ARKAVO_STEP_TIMEOUT_SECS` and
//! `ARKAVO_COMMAND_TIMEOUT_SECS` tune the orchestrator.

use arkavo_router::ModelSpec;
use std::time::Duration;
use tracing::warn;

/// Environment variable that replaces the default budget, in whole seconds.
pub const CHAT_TIMEOUT_ENV: &str = "ARKAVO_CHAT_TIMEOUT_SECS";

/// Budget for anything that runs on this machine. A named local model loads
/// and evaluates exactly like a GGUF path does, so the two share one budget:
/// the 60s a named model used to get was exceeded by a 2,300-token prompt on
/// Gemma 4 12B.
const LOCAL_TIMEOUT_SECS: u64 = 180;

/// Cloud turns can include long provider-side reasoning and tool loops.
const CLOUD_TIMEOUT_SECS: u64 = 3600;

/// Below this an override cannot complete even a trivial turn.
const MIN_OVERRIDE_SECS: u64 = 5;

/// A day; anything longer is indistinguishable from having no budget.
const MAX_OVERRIDE_SECS: u64 = 86_400;

/// The budget for one inference against `spec`, honoring the override.
pub(crate) fn chat_inference_timeout(spec: &ModelSpec) -> Duration {
    let configured = std::env::var(CHAT_TIMEOUT_ENV).ok();
    Duration::from_secs(resolve_secs(default_secs(spec), configured.as_deref()))
}

fn default_secs(spec: &ModelSpec) -> u64 {
    match spec.as_named() {
        Some(model) if model.is_cloud() => CLOUD_TIMEOUT_SECS,
        _ => LOCAL_TIMEOUT_SECS,
    }
}

/// Apply an override to a default. An unparseable or out-of-range value is
/// reported and ignored rather than clamped, so a typo cannot silently turn
/// into a five-second budget.
fn resolve_secs(default_secs: u64, configured: Option<&str>) -> u64 {
    let Some(value) = configured else {
        return default_secs;
    };
    match value.trim().parse::<u64>() {
        Ok(secs) if (MIN_OVERRIDE_SECS..=MAX_OVERRIDE_SECS).contains(&secs) => secs,
        _ => {
            warn!(
                var = CHAT_TIMEOUT_ENV,
                value = %value,
                default = default_secs,
                "invalid env duration; using default"
            );
            default_secs
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_router::ModelChoice;
    use std::path::PathBuf;

    /// Regression: a named local model got 60s where a GGUF path got 180s,
    /// although both run the same local inference.
    #[test]
    fn named_local_model_gets_the_gguf_path_budget() {
        let gguf = default_secs(&ModelSpec::GgufPath(PathBuf::from("adapter.gguf")));
        assert_eq!(gguf, 180);
        for model in [
            ModelChoice::LocalGemma4_12B,
            ModelChoice::LocalGemma4E2B,
            ModelChoice::LocalMinistral3B,
        ] {
            assert_eq!(default_secs(&ModelSpec::Named(model)), gguf);
        }
    }

    #[test]
    fn cloud_model_keeps_its_longer_budget() {
        assert_eq!(
            default_secs(&ModelSpec::Named(ModelChoice::GeminiFlash)),
            3600
        );
    }

    #[test]
    fn override_replaces_the_default_for_every_model_kind() {
        assert_eq!(resolve_secs(180, Some("600")), 600);
        assert_eq!(resolve_secs(3600, Some("600")), 600);
        assert_eq!(resolve_secs(180, Some(" 900 ")), 900);
    }

    #[test]
    fn unset_override_leaves_the_default() {
        assert_eq!(resolve_secs(180, None), 180);
    }

    #[test]
    fn invalid_override_falls_back_to_the_default() {
        for value in ["", "soon", "-5", "1.5", "0", "4", "86401"] {
            assert_eq!(resolve_secs(180, Some(value)), 180, "{value:?}");
        }
    }

    #[test]
    fn override_bounds_are_inclusive() {
        assert_eq!(resolve_secs(180, Some("5")), 5);
        assert_eq!(resolve_secs(180, Some("86400")), 86_400);
    }
}
