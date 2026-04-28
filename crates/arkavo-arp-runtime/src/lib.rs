//! Per-agent ARP runtime: bundles `PolicyCache` and `AdaptationEngine` so the
//! conductor can update both as a single unit on every agent step.
//!
//! `ArpRuntime` is constructed from a parsed `ArpDocument` and exposes:
//!
//! * `record_tool_outcome` — called after each tool invocation in the
//!   conductor loop. Updates the AdaptationEngine prior for the tool and
//!   writes a quality-tagged entry into the PolicyCache so the cache
//!   reflects what the agent has actually learned.
//! * `cache()` / `adaptation()` — accessors for the gateway/UI to snapshot
//!   live state.
//!
//! A process-global accessor (`install` / `current`) lets the conductor
//! reach the runtime without threading an extra argument through every
//! tool-loop call site. The CLI installs an instance once at startup; the
//! standalone AG-UI gateway constructs its own when no agent is running.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use arkavo_adaptation::AdaptationEngine;
use arkavo_arp::ArpDocument;
use arkavo_policy_cache::{PolicyCache, PolicySource};
use serde_json::json;
use tokio::sync::Mutex;

/// Bundles the PolicyCache and AdaptationEngine instantiated from an ARP
/// document. Cheap to clone — backed by `Arc`s.
pub struct ArpRuntime {
    cache: Arc<PolicyCache>,
    adaptation: Arc<Mutex<AdaptationEngine>>,
    quality_threshold: f64,
    /// Monotonic counter so each cache key is unique. Required because the
    /// PolicyCache hash chain doesn't tolerate same-key overwrites.
    outcome_seq: AtomicU64,
}

impl ArpRuntime {
    /// Build a runtime from a parsed ARP document. Quality threshold is
    /// pulled from `feedback_loops.immediate.quality_gate.threshold_default`.
    pub fn from_document(doc: &ArpDocument) -> Self {
        let cache_cfg = doc.feedback_loops.short_term.policy_cache.clone();
        let cache = Arc::new(PolicyCache::new(cache_cfg));
        let adaptation = Arc::new(Mutex::new(AdaptationEngine::new(doc.adaptation.clone())));
        let quality_threshold = doc.feedback_loops.immediate.quality_gate.threshold_default;
        Self {
            cache,
            adaptation,
            quality_threshold,
            outcome_seq: AtomicU64::new(0),
        }
    }

    pub fn cache(&self) -> Arc<PolicyCache> {
        self.cache.clone()
    }

    pub fn adaptation(&self) -> Arc<Mutex<AdaptationEngine>> {
        self.adaptation.clone()
    }

    /// Quality threshold below which an outcome is treated as a failure
    /// when updating priors and the cache.
    pub fn quality_threshold(&self) -> f64 {
        self.quality_threshold
    }

    /// Record a tool's outcome into both the AdaptationEngine prior and
    /// the PolicyCache. Called once per tool invocation in the conductor.
    ///
    /// `quality` is the response/result quality in [0.0, 1.0]. Successes
    /// above the configured threshold reinforce the prior; below-threshold
    /// outcomes degrade it. The cache entry records the latest verdict so
    /// the UI can show what the agent has learned about each tool.
    pub async fn record_tool_outcome(&self, tool_name: &str, success: bool, quality: f64) {
        let q = quality.clamp(0.0, 1.0);
        let above_gate = q >= self.quality_threshold;
        let effective_success = success && above_gate;

        {
            let mut eng = self.adaptation.lock().await;
            eng.update(
                tool_name,
                arkavo_adaptation::Outcome {
                    success: effective_success,
                    quality_score: q,
                },
            );
        }

        let n = self.outcome_seq.fetch_add(1, Ordering::Relaxed);
        let key = format!("tool.outcome.{tool_name}.{n}");
        let value = json!({
            "tool": tool_name,
            "success": effective_success,
            "quality": q,
            "above_quality_gate": above_gate,
        });
        self.cache.insert(key, value, PolicySource::Automated);
    }
}

/// Process-global ARP runtime — installed by the CLI at startup, looked up
/// by the conductor tool loop and the gateway when no explicit reference
/// is available.
static GLOBAL: OnceLock<Arc<ArpRuntime>> = OnceLock::new();

/// Install the global runtime. Returns `Err` if one is already installed.
pub fn install(rt: Arc<ArpRuntime>) -> Result<(), Arc<ArpRuntime>> {
    GLOBAL.set(rt)
}

/// Current global ARP runtime, if installed.
pub fn current() -> Option<Arc<ArpRuntime>> {
    GLOBAL.get().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN_DOC: &str = r#"{
        "arp_spec": "0.1.0",
        "adl_ref": {"uri": "https://example.com/adl.json", "document_hash": "sha256:abc"},
        "adaptation": {"method": "thompson_sampling"},
        "feedback_loops": {
            "immediate": {
                "quality_gate": {
                    "threshold_default": 0.7,
                    "metric": "cosine_similarity",
                    "on_failure": "update_prior_and_log"
                }
            },
            "short_term": {
                "policy_cache": {
                    "default_ttl_sec": 3600,
                    "decay_strategy": "exponential",
                    "decay_half_life_sec": 86400
                }
            }
        },
        "budget": {
            "task_ceiling_usd": 2.5,
            "on_exhaustion": "halt_and_report",
            "velocity": {"max_spend_per_minute_usd": 0.5}
        }
    }"#;

    fn make_runtime() -> ArpRuntime {
        let doc = arkavo_arp::parse(MIN_DOC).unwrap();
        ArpRuntime::from_document(&doc)
    }

    #[tokio::test]
    async fn record_success_updates_prior_and_cache() {
        let rt = make_runtime();

        rt.record_tool_outcome("filesystem_tools", true, 0.95).await;

        // Cache now has one entry
        assert_eq!(rt.cache().len(), 1);
        let entries = rt.cache().snapshot();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].key.contains("filesystem_tools"));

        // Adaptation engine has a prior for this tool
        let snap = rt.adaptation().lock().await.snapshot();
        let prior = snap
            .iter()
            .find(|p| p.id == "filesystem_tools")
            .expect("prior exists");
        assert!(prior.alpha > 1.0, "alpha grew on success: {}", prior.alpha);
    }

    #[tokio::test]
    async fn record_below_quality_gate_increments_beta() {
        let rt = make_runtime();

        // Quality 0.4 is below the 0.7 gate; treated as failure.
        rt.record_tool_outcome("shell_exec", true, 0.4).await;

        let snap = rt.adaptation().lock().await.snapshot();
        let prior = snap.iter().find(|p| p.id == "shell_exec").unwrap();
        assert!(
            prior.beta > 1.0,
            "beta grew on below-gate outcome: {}",
            prior.beta
        );
    }

    #[tokio::test]
    async fn cache_chain_remains_valid_after_updates() {
        let rt = make_runtime();
        rt.record_tool_outcome("tool_a", true, 0.9).await;
        rt.record_tool_outcome("tool_b", false, 0.2).await;
        rt.record_tool_outcome("tool_a", true, 0.85).await;
        assert!(rt.cache().verify_chain());
    }
}
