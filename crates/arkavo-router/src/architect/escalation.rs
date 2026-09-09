//! Model escalation ladder for architect subtask retries.
use crate::decision::ModelChoice;

/// Next rung on the escalation ladder, or `None` at a ceiling where no
/// stronger arm exists.
pub(super) fn next_rung(current: &ModelChoice) -> Option<ModelChoice> {
    let next = match current {
        // Qwen3/Ministral escalation path
        ModelChoice::LocalQwen3 => ModelChoice::LocalMinistral3B,
        ModelChoice::LocalMinistral3B => ModelChoice::LocalMinistral8B,
        ModelChoice::LocalMinistral8B => ModelChoice::LocalQwen35_9B,
        ModelChoice::LocalQwen35_9B => ModelChoice::LocalQwen35_27B,
        ModelChoice::LocalQwen35_27B => ModelChoice::LocalQwen36A3B,
        ModelChoice::LocalQwen36A3B => ModelChoice::LocalGlm47Flash,
        ModelChoice::LocalGlm47Flash => ModelChoice::GeminiFlash,
        // Gemma 4 escalation path
        ModelChoice::LocalGemma4E2B => ModelChoice::LocalGemma4E4B,
        ModelChoice::LocalGemma4E4B => ModelChoice::LocalMinistral8B,
        ModelChoice::LocalGemma4_12B => ModelChoice::LocalGemma4_26B,
        ModelChoice::LocalGemma4_26B => ModelChoice::LocalGemma4_31B,
        ModelChoice::LocalGemma4_31B => ModelChoice::GeminiFlash,
        // Legacy Gemma escalation path
        ModelChoice::LocalGemma270M => ModelChoice::LocalGemma4B,
        ModelChoice::LocalGemma4B => ModelChoice::GeminiFlash,
        ModelChoice::LocalGemma12B => ModelChoice::GeminiPro,
        // Other escalation paths
        ModelChoice::LocalDeepSeekCoder => ModelChoice::DeepSeekV32,
        ModelChoice::DeepSeekV32 => ModelChoice::ClaudeSonnet,
        ModelChoice::DeepSeekV32Speciale => ModelChoice::ClaudeOpus,
        ModelChoice::GeminiFlash => ModelChoice::Gemini35Flash,
        // Escalate within the 3.5 Flash thinking-tier ladder before
        // jumping families, so Thompson Sampling actually exercises the
        // distinct arms before falling back to Anthropic.
        ModelChoice::Gemini35FlashMinimal => ModelChoice::Gemini35Flash,
        ModelChoice::Gemini35Flash => ModelChoice::Gemini35FlashMedium,
        ModelChoice::Gemini35FlashMedium => ModelChoice::Gemini35FlashHigh,
        ModelChoice::Gemini35FlashHigh => ModelChoice::ClaudeSonnet,
        ModelChoice::ClaudeSonnet => ModelChoice::GeminiPro,
        ModelChoice::GeminiPro => ModelChoice::ClaudeOpus,
        // Fable 5 is the most capable tier — the escalation ceiling.
        ModelChoice::ClaudeOpus => ModelChoice::ClaudeFable5,
        ModelChoice::ClaudeFable5 => return None,
        ModelChoice::KimiK2 => ModelChoice::ClaudeSonnet,
        // GLM-5.2 is a low-cost cloud arm; escalate to a stronger tier
        // when it underperforms on a task.
        ModelChoice::Glm52 => ModelChoice::ClaudeSonnet,
        // Grok 4.6 climbs the effort ladder before leaving the family.
        ModelChoice::Grok46 => ModelChoice::Grok46Xhigh,
        ModelChoice::Grok46Xhigh => ModelChoice::ClaudeSonnet,
        // Astra steps down its documented fallback chain rather than
        // re-dispatching itself.
        ModelChoice::Gpt6Astra => ModelChoice::ClaudeSonnet,
    };
    (next != *current).then_some(next)
}

/// The ladder is a strictly climbing chain, so the walk below always reaches
/// a ceiling. The bound only keeps a future edit that introduced a cycle from
/// hanging a retry loop.
const MAX_LADDER_STEPS: usize = 32;

/// First rung above `current` that this device can actually run.
///
/// A rung it cannot serve is skipped rather than ending the climb: an
/// unprovisioned local weight would otherwise be fetched on demand — a
/// multi-gigabyte download in the middle of a retry — and a cloud arm with no
/// configured provider would only dead-end at instantiation.
pub(super) fn next_runnable_rung(
    current: &ModelChoice,
    runnable: impl Fn(&ModelChoice) -> bool,
) -> Option<ModelChoice> {
    let mut model = current.clone();
    for _ in 0..MAX_LADDER_STEPS {
        let next = next_rung(&model)?;
        if runnable(&next) {
            return Some(next);
        }
        model = next;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("ROUTER-002")]
    #[spec("ROUTER-010")]
    #[test]
    fn ladder_climbs_and_stops_at_the_ceiling() {
        assert_eq!(
            next_rung(&ModelChoice::GeminiFlash),
            Some(ModelChoice::Gemini35Flash)
        );
        assert_eq!(
            next_rung(&ModelChoice::Gemini35Flash),
            Some(ModelChoice::Gemini35FlashMedium)
        );
        assert_eq!(
            next_rung(&ModelChoice::Gemini35FlashHigh),
            Some(ModelChoice::ClaudeSonnet)
        );
        assert_eq!(
            next_rung(&ModelChoice::ClaudeSonnet),
            Some(ModelChoice::GeminiPro)
        );
        assert_eq!(
            next_rung(&ModelChoice::ClaudeOpus),
            Some(ModelChoice::ClaudeFable5)
        );
        // Fable 5 is the escalation ceiling — there is nothing above it.
        assert_eq!(next_rung(&ModelChoice::ClaudeFable5), None);
        assert_eq!(
            next_rung(&ModelChoice::Grok46),
            Some(ModelChoice::Grok46Xhigh)
        );
        assert_eq!(
            next_rung(&ModelChoice::Grok46Xhigh),
            Some(ModelChoice::ClaudeSonnet)
        );
    }

    /// Astra used to "escalate" to itself, so a failed subtask burned
    /// max_retries re-dispatching the same paid model.
    #[spec("ASTRA-005")]
    #[test]
    fn astra_escalation_leaves_astra() {
        assert_eq!(
            next_rung(&ModelChoice::Gpt6Astra),
            Some(ModelChoice::ClaudeSonnet)
        );
        assert_ne!(
            next_rung(&ModelChoice::Gpt6Astra),
            Some(ModelChoice::Gpt6Astra)
        );
    }

    /// Regression: local rungs used to be taken unfiltered, so a retry on an
    /// unprovisioned device escalated onto a weight that was not on disk and
    /// the loader downloaded it mid-plan. The climb now steps over every rung
    /// the device cannot run.
    #[spec("ROUTER-010")]
    #[test]
    fn the_climb_steps_over_rungs_this_device_cannot_run() {
        // Nothing local is provisioned; only the Gemini tier is configured.
        let runnable = |m: &ModelChoice| {
            matches!(
                m,
                ModelChoice::GeminiFlash
                    | ModelChoice::Gemini35Flash
                    | ModelChoice::Gemini35FlashMedium
                    | ModelChoice::Gemini35FlashHigh
                    | ModelChoice::GeminiPro
            )
        };
        assert_eq!(
            next_runnable_rung(&ModelChoice::LocalQwen3, runnable),
            Some(ModelChoice::GeminiFlash),
            "every local rung between Qwen3 and Gemini Flash is unprovisioned"
        );
    }

    /// With nothing runnable above it the climb ends instead of looping, and
    /// the caller reports "no available escalation target".
    #[spec("ROUTER-010")]
    #[test]
    fn a_ladder_with_no_runnable_rung_terminates() {
        assert_eq!(
            next_runnable_rung(&ModelChoice::LocalQwen3, |_| false),
            None
        );
        assert_eq!(
            next_runnable_rung(&ModelChoice::ClaudeFable5, |_| true),
            None,
            "the ceiling has no rung above it"
        );
    }
}
