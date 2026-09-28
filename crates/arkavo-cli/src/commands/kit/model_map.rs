//! Shared model vocabulary between the kit's `agent_provisioning.model`
//! (family/size) representation and the CLI's flat `model:` hint strings
//! (e.g. `"ministral-3b"`, `"gpt-6-astra"`).
//!
//! Single source of truth for both directions so `kit migrate-from-agents-md`
//! (hint → kit model, in `kit_build.rs`) and the `arkavo agent -c <kit>` run
//! path (kit model → hint, in `agent_kit.rs`) can never drift apart.
//!
//! Agents run a mix of local and cloud models, so a role may name either:
//! a locally-hosted edge model as family/size (`ministral`/`3B`), or any
//! model the router knows by its id as the family with no size
//! (`gpt-6-astra`, `grok-4.7`, `kimi-k2.5`). The router's `ModelChoice`
//! registry is the arbiter; anything it does not recognise has no hint.

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

/// AGENTS.md-style `model:` hint → kit `Model`.
///
/// Local edge models keep the kit's family/size vocabulary; every other model
/// the router knows is written as its canonical id in `family` with no size,
/// which is exactly the shape [`kit_model_to_hint`] reads back.
pub(super) fn hint_to_kit_model(hint: &str) -> Option<Model> {
    let choice = ModelChoice::from_name(hint)?;
    if let Some((_, family, size)) = LOCAL_EDGE_MODELS
        .iter()
        .find(|(candidate, _, _)| *candidate == choice)
    {
        return Some(Model {
            family: (*family).to_string(),
            size: Some((*size).to_string()),
            quantization: None,
            backend: Some("llama.cpp".to_string()),
            fallback: None,
        });
    }
    Some(Model {
        family: choice.name().to_string(),
        size: None,
        quantization: None,
        backend: choice.is_local().then(|| "llama.cpp".to_string()),
        fallback: None,
    })
}

/// Kit `agent_provisioning.model` (family/size) → CLI model hint string.
///
/// Returns `None` when the router does not know the model; the `arkavo
/// agent` path turns that into an error rather than silently letting the
/// router pick something the kit did not ask for.
///
/// `pub(crate)`, re-exported by `kit.rs`, because `agent_kit.rs` (a sibling
/// of the `kit` module, not a descendant) needs it too. `clippy::pedantic`'s
/// `redundant_pub_crate` and `unreachable_pub` disagree on the right
/// annotation for a `pub(crate)` item inside a private module that's then
/// re-exported — this is the one `unreachable_pub` (the workspace's actual
/// `warn` lint; `redundant_pub_crate` only rides along under `pedantic`)
/// accepts as correct.
#[allow(clippy::redundant_pub_crate)]
pub(crate) fn kit_model_to_hint(family: &str, size: Option<&str>) -> Option<String> {
    if let Some(size) = size
        && let Some((choice, _, _)) = LOCAL_EDGE_MODELS
            .iter()
            .find(|(_, f, s)| f.eq_ignore_ascii_case(family) && s.eq_ignore_ascii_case(size))
    {
        return Some(choice.name().to_string());
    }
    let id = match size {
        Some(size) => format!("{family}-{size}"),
        None => family.to_string(),
    };
    ModelChoice::from_name(&id).map(|choice| choice.name().to_string())
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
            assert_eq!(kit_model_to_hint(family, Some(size)).as_deref(), Some(hint));
        }
    }

    #[arkavo_test_macros::spec("SK-104")]
    #[test]
    fn round_trips_every_cloud_model() {
        for choice in ModelChoice::ALL_CLOUD {
            let hint = choice.name();
            let model = hint_to_kit_model(hint).expect("cloud hint should map");
            assert_eq!(model.family, hint);
            assert_eq!(model.size, None);
            assert_eq!(model.backend, None);
            assert_eq!(
                kit_model_to_hint(&model.family, None).as_deref(),
                Some(hint),
                "kit family {hint:?} must resolve back to the same router arm"
            );
        }
    }

    #[arkavo_test_macros::spec("SK-104")]
    #[test]
    fn kit_cloud_model_maps_to_router_hint() {
        assert_eq!(
            kit_model_to_hint("gpt-6-astra", None).as_deref(),
            Some("gpt-6-astra")
        );
        assert_eq!(
            kit_model_to_hint("GPT-6-Astra", None).as_deref(),
            Some("gpt-6-astra")
        );
        assert_eq!(
            kit_model_to_hint("kimi-k2.5", None).as_deref(),
            Some(ModelChoice::KimiK2.name())
        );
        // A vendor/version split spelled as family + size resolves the same id.
        assert_eq!(
            kit_model_to_hint("grok", Some("4.7")).as_deref(),
            Some(ModelChoice::Grok47.name())
        );
    }

    #[test]
    fn unknown_hints_stay_unmapped() {
        assert!(hint_to_kit_model("totally-unknown-model").is_none());
    }

    #[arkavo_test_macros::spec("SK-104")]
    #[test]
    fn unknown_kit_model_stays_unmapped() {
        assert!(kit_model_to_hint("unknown-family", Some("1B")).is_none());
        assert!(kit_model_to_hint("gpt-7-imaginary", None).is_none());
        assert!(kit_model_to_hint("qwen3", Some("7B")).is_none());
        assert!(
            kit_model_to_hint("ministral", None).is_none(),
            "a size-less edge family has no unambiguous hint"
        );
    }
}
