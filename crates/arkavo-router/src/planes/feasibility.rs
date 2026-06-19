//! Feasibility plane — "can the local model run this *now*?"
//!
//! This plane consumes llama.cpp / runtime statistics ONLY. It answers what is
//! *possible*, never what is *advisable* or *allowed*. There is deliberately no
//! "use cloud" conclusion in [`FeasibilityVerdict`]: a slow laptop is not a
//! budget-authorization event.
//!
//! Allowed conclusions: local can run now, local can run slowly, local cannot
//! run, local needs chunking, local needs a smaller context. A feasibility
//! failure may make the router ask the user, queue, retry locally, or return a
//! partial answer — it may **never** silently spend cloud dollars.

use serde::{Deserialize, Serialize};

/// Runtime telemetry sampled from the local llama.cpp engine for one request.
///
/// Every field is a measurement of *cost/feasibility*, not answer quality.
#[derive(Debug, Clone)]
pub struct RuntimeStats {
    /// Prompt token count for this request.
    pub prompt_tokens: u32,
    /// Context window of the loaded model (`0` if unknown).
    pub n_ctx: u32,
    /// Whether a local model is loaded and ready to serve.
    pub local_model_available: bool,
    /// Free KV-cache slots in the context pool (`0` = saturated).
    pub kv_cache_slots_free: usize,
    /// Requests queued ahead of this one.
    pub queue_depth: usize,
    /// Recent decode throughput (tokens/sec). `None` if not yet measured.
    pub tokens_per_sec: Option<f32>,
    /// Device is thermally throttled or under heavy load.
    pub thermal_throttling: bool,
    /// The previous attempt hit OOM / context overflow.
    pub context_overflow: bool,
}

impl RuntimeStats {
    /// Below this decode rate the experience is "slow but working".
    const SLOW_TOKENS_PER_SEC: f32 = 8.0;
    /// More than this many requests queued ahead makes the wait noticeable.
    const SLOW_QUEUE_DEPTH: usize = 2;
    /// Once the prompt crosses this fraction of context, too little headroom
    /// remains for a useful completion.
    const CONTEXT_PRESSURE: f32 = 0.9;
}

/// What the feasibility plane concluded about local execution.
///
/// None of these variants name the cloud. Mapping a verdict to a *cloud* action
/// is not this plane's job — and in particular `LocalCannotRun` is not a spend
/// authorization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeasibilityVerdict {
    /// Local can run this request now at an acceptable speed.
    LocalCanRunNow,
    /// Local can run, but slowly (queue depth, low tok/s, thermal throttle).
    LocalCanRunSlowly,
    /// The prompt alone exceeds the context window; it must be chunked first.
    LocalNeedsChunking,
    /// The prompt is near the context limit; retry with a smaller context.
    LocalNeedsSmallerContext,
    /// Local cannot run right now (no model, KV saturated, OOM/overflow).
    LocalCannotRun,
}

impl FeasibilityVerdict {
    /// Whether local execution can proceed (now or slowly) without reshaping.
    pub fn can_run_local(self) -> bool {
        matches!(self, Self::LocalCanRunNow | Self::LocalCanRunSlowly)
    }

    /// Whether the request must be reshaped (chunk / smaller context) before
    /// local can run.
    pub fn needs_reshape(self) -> bool {
        matches!(
            self,
            Self::LocalNeedsChunking | Self::LocalNeedsSmallerContext
        )
    }
}

/// Assess local feasibility from runtime statistics.
///
/// The check order is deliberate: a request that does not *fit* is infeasible
/// until reshaped (not merely "slow"), so context sizing is decided before
/// throughput.
pub fn assess(stats: &RuntimeStats) -> FeasibilityVerdict {
    // No engine to run on at all.
    if !stats.local_model_available {
        return FeasibilityVerdict::LocalCannotRun;
    }

    // Context sizing comes before throughput.
    if stats.n_ctx > 0 {
        if stats.prompt_tokens >= stats.n_ctx {
            // The prompt alone does not fit — it must be split.
            return FeasibilityVerdict::LocalNeedsChunking;
        }
        let pressure = stats.prompt_tokens as f32 / stats.n_ctx as f32;
        if pressure >= RuntimeStats::CONTEXT_PRESSURE || stats.context_overflow {
            // Fits, but leaves too little room for output (or just overflowed).
            return FeasibilityVerdict::LocalNeedsSmallerContext;
        }
    } else if stats.context_overflow {
        return FeasibilityVerdict::LocalNeedsSmallerContext;
    }

    // KV cache fully saturated: the request cannot be placed right now.
    if stats.kv_cache_slots_free == 0 {
        return FeasibilityVerdict::LocalCannotRun;
    }

    // Feasible. Is it fast enough to be pleasant?
    let throttled = stats.thermal_throttling
        || stats.queue_depth > RuntimeStats::SLOW_QUEUE_DEPTH
        || stats
            .tokens_per_sec
            .is_some_and(|t| t < RuntimeStats::SLOW_TOKENS_PER_SEC);

    if throttled {
        FeasibilityVerdict::LocalCanRunSlowly
    } else {
        FeasibilityVerdict::LocalCanRunNow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A healthy, idle runtime with plenty of headroom.
    fn healthy() -> RuntimeStats {
        RuntimeStats {
            prompt_tokens: 1_000,
            n_ctx: 8_192,
            local_model_available: true,
            kv_cache_slots_free: 4,
            queue_depth: 0,
            tokens_per_sec: Some(40.0),
            thermal_throttling: false,
            context_overflow: false,
        }
    }

    #[test]
    fn healthy_runtime_runs_now() {
        assert_eq!(assess(&healthy()), FeasibilityVerdict::LocalCanRunNow);
    }

    #[test]
    fn no_local_model_cannot_run() {
        let stats = RuntimeStats {
            local_model_available: false,
            ..healthy()
        };
        assert_eq!(assess(&stats), FeasibilityVerdict::LocalCannotRun);
    }

    #[test]
    fn oversized_prompt_needs_chunking() {
        let stats = RuntimeStats {
            prompt_tokens: 9_000,
            n_ctx: 8_192,
            ..healthy()
        };
        assert_eq!(assess(&stats), FeasibilityVerdict::LocalNeedsChunking);
    }

    #[test]
    fn near_full_context_needs_smaller_context() {
        let stats = RuntimeStats {
            prompt_tokens: 7_900, // ~96% of 8192
            n_ctx: 8_192,
            ..healthy()
        };
        assert_eq!(assess(&stats), FeasibilityVerdict::LocalNeedsSmallerContext);
    }

    #[test]
    fn overflow_flag_needs_smaller_context() {
        let stats = RuntimeStats {
            context_overflow: true,
            ..healthy()
        };
        assert_eq!(assess(&stats), FeasibilityVerdict::LocalNeedsSmallerContext);
    }

    #[test]
    fn saturated_kv_cache_cannot_run() {
        let stats = RuntimeStats {
            kv_cache_slots_free: 0,
            ..healthy()
        };
        assert_eq!(assess(&stats), FeasibilityVerdict::LocalCannotRun);
    }

    // Each of these is a *feasibility* degradation, not a quality degradation:
    // the request still runs, just slowly. None may authorize cloud spend.
    #[test]
    fn low_throughput_runs_slowly() {
        let stats = RuntimeStats {
            tokens_per_sec: Some(3.0),
            ..healthy()
        };
        assert_eq!(assess(&stats), FeasibilityVerdict::LocalCanRunSlowly);
    }

    #[test]
    fn deep_queue_runs_slowly() {
        let stats = RuntimeStats {
            queue_depth: 5,
            ..healthy()
        };
        assert_eq!(assess(&stats), FeasibilityVerdict::LocalCanRunSlowly);
    }

    #[test]
    fn thermal_throttle_runs_slowly() {
        let stats = RuntimeStats {
            thermal_throttling: true,
            ..healthy()
        };
        assert_eq!(assess(&stats), FeasibilityVerdict::LocalCanRunSlowly);
    }

    #[test]
    fn verdict_helpers() {
        assert!(FeasibilityVerdict::LocalCanRunNow.can_run_local());
        assert!(FeasibilityVerdict::LocalCanRunSlowly.can_run_local());
        assert!(!FeasibilityVerdict::LocalCannotRun.can_run_local());
        assert!(FeasibilityVerdict::LocalNeedsChunking.needs_reshape());
        assert!(!FeasibilityVerdict::LocalCanRunNow.needs_reshape());
    }
}
