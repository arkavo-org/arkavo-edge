//! Shared local-edge-model vocabulary between the kit's
//! `agent_provisioning.model` (family/size) representation and the CLI's
//! flat `model:` hint strings (e.g. `"ministral-3b"`).
//!
//! Single source of truth for both directions so `kit migrate-from-agents-md`
//! (hint → kit model, in `kit_build.rs`) and the `arkavo agent -c <kit>` run
//! path (kit model → hint, in `agent_kit.rs`) can never drift apart.
//!
//! Only covers the locally-hosted edge models this CLI actually provisions
//! (see CLAUDE.md's "Local Edge Models" section); cloud model hints (Claude,
//! Gemini, Grok, ...) are intentionally absent — `agent_provisioning.model`
//! describes a local backend, not a cloud API id.

use arkavo_router::ModelChoice;
use arkavo_swarmkit::Model;

/// (router hint arm, kit family, kit size) for every locally-hosted edge
/// model this CLI provisions.
const LOCAL_EDGE_MODELS: &[(ModelChoice, &str, &str)] = &[
    (ModelChoice::LocalMinistral3B, "ministral", "3B"),
    (ModelChoice::LocalMinistral8B, "ministral", "8B"),
    (ModelChoice::LocalGemma4E2B, "gemma", "E2B"),
    (ModelChoice::LocalGemma4E4B, "gemma", "E4B"),
    (ModelChoice::LocalGemma4_12B, "gemma", "12B"),
    (ModelChoice::LocalQwen3, "qwen", "0.8B"),
    (ModelChoice::LocalQwen35_9B, "qwen", "9B"),
    (ModelChoice::LocalQwen35_27B, "qwen", "27B"),
];

/// AGENTS.md-style `model:` hint → kit `Model`. Reuses
/// `ModelChoice::from_name` to recognize known aliases (e.g. `"qwen3-0.6b"`),
/// then re-derives family/size in the kit's own vocabulary rather than the
/// router's generic vendor family, since the two serve different purposes.
pub(super) fn hint_to_kit_model(hint: &str) -> Option<Model> {
    let choice = ModelChoice::from_name(hint)?;
    LOCAL_EDGE_MODELS
        .iter()
        .find(|(candidate, _, _)| *candidate == choice)
        .map(|(_, family, size)| Model {
            family: (*family).to_string(),
            size: Some((*size).to_string()),
            quantization: None,
            backend: Some("llama.cpp".to_string()),
            fallback: None,
        })
}

/// Kit `agent_provisioning.model` (family/size) → CLI model hint string.
/// Unknown/absent family or size returns `None`; callers fall back to an
/// empty hint string (the router then decides).
///
/// `pub(crate)`, re-exported by `kit.rs`, because `agent_kit.rs` (a sibling
/// of the `kit` module, not a descendant) needs it too. `clippy::pedantic`'s
/// `redundant_pub_crate` and `unreachable_pub` disagree on the right
/// annotation for a `pub(crate)` item inside a private module that's then
/// re-exported — this is the one `unreachable_pub` (the workspace's actual
/// `warn` lint; `redundant_pub_crate` only rides along under `pedantic`)
/// accepts as correct.
#[allow(clippy::redundant_pub_crate)]
pub(crate) fn kit_model_to_hint(family: &str, size: Option<&str>) -> Option<&'static str> {
    let size = size?;
    LOCAL_EDGE_MODELS
        .iter()
        .find(|(_, f, s)| f.eq_ignore_ascii_case(family) && s.eq_ignore_ascii_case(size))
        .map(|(choice, _, _)| choice.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_local_edge_model() {
        for (choice, family, size) in LOCAL_EDGE_MODELS {
            let hint = choice.name();
            let model = hint_to_kit_model(hint).expect("known hint should map");
            assert_eq!(&model.family, family);
            assert_eq!(model.size.as_deref(), Some(*size));
            assert_eq!(kit_model_to_hint(family, Some(size)), Some(hint));
        }
    }

    #[test]
    fn unknown_or_cloud_hints_stay_unmapped() {
        assert!(hint_to_kit_model("totally-unknown-model").is_none());
        assert!(
            hint_to_kit_model("claude-sonnet-4-5-20250929").is_none(),
            "cloud model hints must stay unmapped"
        );
    }

    #[test]
    fn unknown_kit_model_stays_unmapped() {
        assert!(kit_model_to_hint("unknown-family", Some("1B")).is_none());
        assert!(
            kit_model_to_hint("ministral", None).is_none(),
            "a size-less model has no unambiguous hint"
        );
    }
}
