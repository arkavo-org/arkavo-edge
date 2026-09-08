//! What this device can actually run.
//!
//! Split out of [`crate::selector`] because "which arm does this category
//! want" and "which arms are on this disk, in this much RAM, on this GPU" are
//! different questions with different inputs. Everything here reads the
//! machine (or the injected stand-in for it); nothing here reads the task.
use crate::decision::ModelChoice;
use crate::model_discovery;
use crate::selector::ModelSelector;

/// Where "is this local weight already on disk" is answered from.
///
/// Production reads the HuggingFace cache; callers that must be deterministic —
/// tests, and downstream crates asserting selection without touching the
/// machine — inject a fixed answer instead.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LocalWeights {
    /// Ask the on-disk HuggingFace cache.
    HuggingFaceCache,
    /// Answer every cache question with this value.
    Fixed(bool),
    /// Answer `true` for exactly these arms. Models a device that has
    /// provisioned some weights but not others, which `Fixed` cannot express.
    Only(&'static [ModelChoice]),
}

impl ModelSelector {
    /// Check GPU acceleration status via arkavo-llm
    pub(crate) fn check_gpu_status() -> bool {
        #[cfg(feature = "llama-cpp")]
        {
            arkavo_llm::is_gpu_accelerated()
        }
        #[cfg(not(feature = "llama-cpp"))]
        {
            false
        }
    }

    /// Check if system has at least `min_gb` of RAM
    fn has_sufficient_ram(min_gb: u64) -> bool {
        #[cfg(target_os = "macos")]
        {
            use std::process::Command;
            if let Ok(output) = Command::new("sysctl").arg("-n").arg("hw.memsize").output()
                && let Ok(mem_str) = String::from_utf8(output.stdout)
                && let Ok(bytes) = mem_str.trim().parse::<u64>()
            {
                return bytes >= min_gb * 1024 * 1024 * 1024;
            }
            false
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = min_gb;
            true
        }
    }

    /// Whether this local model's weights are already on disk.
    pub(crate) fn is_local_model_cached(&self, model: &ModelChoice) -> bool {
        match self.local_weights() {
            LocalWeights::Fixed(cached) => return cached,
            LocalWeights::Only(models) => return models.contains(model),
            LocalWeights::HuggingFaceCache => {}
        }
        if !cfg!(feature = "llama-cpp") {
            return false;
        }
        match (model.repo_id(), model.gguf_filename()) {
            (Some(repo), Some(file)) => model_discovery::is_model_cached(repo, file),
            _ => false,
        }
    }

    fn memory_budget(&self) -> u64 {
        self.max_memory_bytes
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Cached local arms that fit the memory budget, smallest first.
    fn cached_local_models(&self) -> impl Iterator<Item = ModelChoice> + '_ {
        let mem_budget = self.memory_budget();
        ModelChoice::ALL_LOCAL
            .iter()
            .filter(move |m| {
                self.is_local_model_cached(m) && (mem_budget == 0 || m.size_bytes() <= mem_budget)
            })
            .cloned()
    }

    /// Best local arm this device can actually run.
    ///
    /// Every rung is checked against the cache, the memory budget and (for the
    /// big MoE weights) system RAM, and the fallback is the smallest arm that
    /// *is* on disk — not a fixed name. Naming an arm the device never
    /// downloaded only moves the failure to the loader, where it reads as a
    /// multi-gigabyte hang instead of a refusal.
    ///
    /// Without a GPU the large weights are unusably slow (20s+ per turn on
    /// CPU), so the smallest cached arm wins outright.
    pub(crate) fn best_available_local_model(&self, prefer_larger: bool) -> ModelChoice {
        let mem_budget = self.memory_budget();
        let runnable = |m: &ModelChoice| {
            self.is_local_model_cached(m) && (mem_budget == 0 || m.size_bytes() <= mem_budget)
        };

        if prefer_larger && self.gpu_available {
            // GLM-4.7-Flash: 30B MoE, highest quality local model.
            for (model, min_ram_gb) in [
                (ModelChoice::LocalGlm47Flash, 32),
                (ModelChoice::LocalQwen35_27B, 48),
                (ModelChoice::LocalMinistral8B, 0),
                (ModelChoice::LocalMinistral3B, 0),
            ] {
                if runnable(&model) && (min_ram_gb == 0 || Self::has_sufficient_ram(min_ram_gb)) {
                    return model;
                }
            }
        }

        // Preference order first, then the smallest cached arm: falling
        // straight to "smallest" would hand a legacy install its 270M weight
        // over the provisioned Qwen3 sitting next to it.
        self.fastest_local_model()
    }

    /// Smallest cached local arm that fits the memory budget.
    pub(crate) fn smallest_cached_local_model(&self) -> Option<ModelChoice> {
        self.cached_local_models().next()
    }

    /// Preference order for the fast internal model (judging, synthesis, classification),
    /// most-preferred first. Gemma 4 E2B leads because first-run setup provisions it as the
    /// "Small (fast routing)" model; the legacy entries keep older installs working without a
    /// download. The fallback when none are cached MUST stay a setup-provisioned model.
    const FAST_LOCAL_PREFERENCE: [ModelChoice; 3] = [
        ModelChoice::LocalGemma4E2B,
        ModelChoice::LocalMinistral3B,
        ModelChoice::LocalQwen3,
    ];

    /// Fastest available local model for internal tasks (judging, synthesis, classification).
    /// Prefers a cached model from [`Self::FAST_LOCAL_PREFERENCE`], then any other cached arm
    /// (smallest first), and only then falls back to Gemma 4 E2B — the model first-run setup
    /// downloads — so chat never silently pulls an un-provisioned model the user never opted
    /// into.
    ///
    /// The fallback names an arm the device may not have. Callers that are
    /// about to *dispatch* must ask [`Self::fastest_cached_local_model`] (or
    /// `Router::require_provisioned`) instead, so an unprovisioned install
    /// refuses rather than downloading weights mid-request.
    pub fn fastest_local_model(&self) -> ModelChoice {
        self.fastest_cached_local_model()
            .unwrap_or(ModelChoice::LocalGemma4E2B)
    }

    /// Fastest local arm whose weights are already on disk, or `None` when the
    /// device has provisioned none of them.
    ///
    /// The setup-provisioned arms come first; a device that holds only, say,
    /// Gemma 4 E4B still gets E4B rather than a refusal naming a model it
    /// never asked for.
    pub(crate) fn fastest_cached_local_model(&self) -> Option<ModelChoice> {
        Self::pick_fast_local_model(|m| self.is_local_model_cached(m))
            .or_else(|| self.smallest_cached_local_model())
    }

    /// The harness's baseline execution model is always local. Cloud credentials
    /// do not replace provisioning the device's local models.
    pub fn default_execution_model(&self) -> ModelChoice {
        self.fastest_local_model()
    }

    /// Policy half of [`Self::fastest_cached_local_model`], split out so the preference order
    /// can be unit-tested without touching the on-disk model cache.
    fn pick_fast_local_model(is_cached: impl Fn(&ModelChoice) -> bool) -> Option<ModelChoice> {
        Self::FAST_LOCAL_PREFERENCE
            .into_iter()
            .find(|m| is_cached(m))
    }

    /// All models currently feasible (cached local + API keys for cloud).
    ///
    /// When `max_memory_bytes` is set (> 0), local models whose weight files
    /// exceed the budget are excluded so Thompson Sampling never selects a
    /// model that would blow the agent's memory allocation.
    pub fn feasible_models(&self) -> Vec<ModelChoice> {
        let mut models = Vec::new();
        let mem_budget = self.memory_budget();

        // Local models (smallest first)
        if self.is_local_model_cached(&ModelChoice::LocalQwen3) {
            models.push(ModelChoice::LocalQwen3);
        }
        if self.is_local_model_cached(&ModelChoice::LocalGemma4E2B) {
            models.push(ModelChoice::LocalGemma4E2B);
        }
        if self.gpu_available {
            if self.is_local_model_cached(&ModelChoice::LocalMinistral3B) {
                models.push(ModelChoice::LocalMinistral3B);
            }
            // Gemma-4-E4B excluded: 1/8 tool accuracy (benchmark), needs grammar-constrained
            // generation. Re-enable when PEG output parser lands.
            if self.is_local_model_cached(&ModelChoice::LocalMinistral8B) {
                models.push(ModelChoice::LocalMinistral8B);
            }
            if self.is_local_model_cached(&ModelChoice::LocalGemma4_26B) {
                models.push(ModelChoice::LocalGemma4_26B);
            }
            if self.is_local_model_cached(&ModelChoice::LocalGemma4_31B) {
                models.push(ModelChoice::LocalGemma4_31B);
            }
            if self.is_local_model_cached(&ModelChoice::LocalGemma4_12B) {
                models.push(ModelChoice::LocalGemma4_12B);
            }
            if self.is_local_model_cached(&ModelChoice::LocalQwen35_27B)
                && Self::has_sufficient_ram(48)
            {
                models.push(ModelChoice::LocalQwen35_27B);
            }
            if self.is_local_model_cached(&ModelChoice::LocalGlm47Flash)
                && Self::has_sufficient_ram(32)
            {
                models.push(ModelChoice::LocalGlm47Flash);
            }
        }

        // Per-agent memory budget: exclude local models that exceed the allocation
        if mem_budget > 0 {
            let before = models.len();
            models.retain(|m| m.size_bytes() == 0 || m.size_bytes() <= mem_budget);
            if models.len() < before {
                tracing::info!(
                    budget_mb = mem_budget / (1024 * 1024),
                    kept = models.len(),
                    excluded = before - models.len(),
                    "Memory budget: excluded models exceeding per-agent allocation"
                );
            }
        }

        models.extend(self.feasible_cloud_models());

        // Fallback: always include Qwen3 as baseline
        if models.is_empty() {
            models.push(ModelChoice::LocalQwen3);
        }

        models
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::selector::ProviderAvailability;
    use arkavo_test_macros::spec;

    fn selector(cached: &'static [ModelChoice]) -> ModelSelector {
        ModelSelector::with_parts(ProviderAvailability::default(), LocalWeights::Only(cached))
    }

    /// Regression: the fast pick only ever considered Gemma 4 E2B, Ministral 3B
    /// and Qwen3, so a device holding only Gemma 4 E4B was told
    /// "gemma-4-e2b is not provisioned" — naming a model it never chose.
    #[spec("ROUTER-003")]
    #[test]
    fn a_device_holding_only_an_unlisted_arm_still_gets_that_arm() {
        let selector = selector(&[ModelChoice::LocalGemma4E4B]);
        assert_eq!(
            selector.fastest_cached_local_model(),
            Some(ModelChoice::LocalGemma4E4B)
        );
        assert_eq!(selector.fastest_local_model(), ModelChoice::LocalGemma4E4B);
    }

    /// Among unlisted arms the smallest wins, so the fast path stays fast.
    #[spec("ROUTER-003")]
    #[test]
    fn the_smallest_cached_arm_serves_the_fast_path() {
        let selector = selector(&[ModelChoice::LocalGemma4_12B, ModelChoice::LocalGemma4E4B]);
        assert_eq!(selector.fastest_local_model(), ModelChoice::LocalGemma4E4B);
    }

    /// The setup-provisioned preference still wins when it is on disk.
    #[spec("ROUTER-003")]
    #[test]
    fn the_provisioned_preference_beats_a_smaller_unlisted_arm() {
        let selector = selector(&[ModelChoice::LocalGemma270M, ModelChoice::LocalGemma4E2B]);
        assert_eq!(selector.fastest_local_model(), ModelChoice::LocalGemma4E2B);
    }

    #[spec("ROUTER-003")]
    #[test]
    fn an_unprovisioned_device_has_no_cached_arm() {
        assert_eq!(selector(&[]).fastest_cached_local_model(), None);
    }

    /// Regression: the "best local" pick fell through to Qwen3 whether or not
    /// its weights were on disk, so classification handed the dispatch an arm
    /// the device would have had to download.
    #[spec("ROUTER-003")]
    #[test]
    fn the_best_local_arm_is_one_that_is_actually_on_disk() {
        let selector = selector(&[ModelChoice::LocalGemma4E4B]);
        assert_eq!(
            selector.best_available_local_model(false),
            ModelChoice::LocalGemma4E4B
        );
        assert_eq!(
            selector.best_available_local_model(true),
            ModelChoice::LocalGemma4E4B
        );
    }

    /// A cached large arm is still preferred when the caller asks for one.
    #[spec("ROUTER-003")]
    #[test]
    fn a_cached_large_arm_wins_when_larger_is_preferred() {
        let selector = selector(&[ModelChoice::LocalQwen3, ModelChoice::LocalMinistral8B]);
        assert_eq!(
            selector.best_available_local_model(true),
            ModelChoice::LocalMinistral8B
        );
        assert_eq!(
            selector.best_available_local_model(false),
            ModelChoice::LocalQwen3
        );
    }

    // Regression: a fresh install provisions Gemma 4 E2B + Gemma 4 12B (no Ministral/Qwen).
    // The fast-model selector must not fall through to a hardcoded Ministral 3B, which made
    // `arkavo chat` silently download an un-provisioned model on first use.
    #[spec("ROUTER-003")]
    #[test]
    fn test_fast_local_model_falls_back_to_provisioned_gemma() {
        let nothing_cached = |_: &ModelChoice| false;
        // Nothing on disk is reported as such, so a dispatching caller can
        // refuse; only the naming helper substitutes the setup model.
        assert_eq!(ModelSelector::pick_fast_local_model(nothing_cached), None);
        assert_eq!(
            ModelSelector::with_availability(ProviderAvailability::default(), false)
                .fastest_local_model(),
            ModelChoice::LocalGemma4E2B,
        );
    }

    /// Regression: `fastest_local_model` names Gemma 4 E2B even on a device
    /// that has never downloaded it, so `route_fast` used to hand that name
    /// straight to the loader and start a multi-gigabyte fetch inside the
    /// caller's timeout. The cached-only accessor is what a dispatch asks.
    #[spec("ROUTER-003")]
    #[test]
    fn an_unprovisioned_device_reports_no_cached_fast_model() {
        let selector = ModelSelector::with_availability(ProviderAvailability::default(), false);
        assert_eq!(selector.fastest_cached_local_model(), None);

        let provisioned = ModelSelector::with_availability(ProviderAvailability::default(), true);
        assert_eq!(
            provisioned.fastest_cached_local_model(),
            Some(ModelChoice::LocalGemma4E2B)
        );
    }

    #[spec("ROUTER-003")]
    #[test]
    fn test_fast_local_model_uses_cached_gemma_e2b() {
        // Fresh install: only the two setup models are present.
        let gemma_cached = |m: &ModelChoice| {
            matches!(
                m,
                ModelChoice::LocalGemma4E2B | ModelChoice::LocalGemma4_12B
            )
        };
        assert_eq!(
            ModelSelector::pick_fast_local_model(gemma_cached),
            Some(ModelChoice::LocalGemma4E2B),
        );
    }

    #[spec("ROUTER-003")]
    #[test]
    fn test_fast_local_model_honors_legacy_ministral_install() {
        // Older install with only Ministral 3B cached still resolves to it (no download).
        let ministral_cached = |m: &ModelChoice| matches!(m, ModelChoice::LocalMinistral3B);
        assert_eq!(
            ModelSelector::pick_fast_local_model(ministral_cached),
            Some(ModelChoice::LocalMinistral3B),
        );
    }
}
