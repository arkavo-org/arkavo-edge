//! Per-configuration runtime-cost baseline for the feasibility plane.
//!
//! llama.cpp statistics measure *observed computational cost*, and that cost is
//! only comparable within one `(model, quantization, context, hardware)`
//! configuration — the same prompt looks "fast" on an M3 and "slow" on a Pi 5.
//! So a single absolute tokens/sec threshold (the cold-start floor in
//! [`super::feasibility::assess`]) is wrong for at least one of them.
//!
//! This store keeps a rolling window of observed decode throughput per config
//! and classifies a new sample with a robust z-score (median / MAD, not
//! mean / stddev, because LLM latency is heavy-tailed). It only ever refines the
//! *running* verdict (`LocalCanRunNow` ↔ `LocalCanRunSlowly`); structural
//! verdicts — chunking, smaller-context, cannot-run — stay with `assess`, and
//! nothing here can authorize cloud spend.

use super::feasibility::{FeasibilityVerdict, RuntimeStats, assess};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::RwLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// Serializable form of the baseline for persistence across restarts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FeasibilityBaselineSnapshot {
    /// Config key (`model@n_ctx`) → recent throughput samples.
    pub windows: HashMap<String, Vec<f32>>,
}

/// Rolling per-config throughput baseline.
#[derive(Debug, Default)]
pub struct FeasibilityBaseline {
    windows: RwLock<HashMap<String, VecDeque<f32>>>,
    /// Where the baseline autosaves, if persistence is enabled.
    path: Option<PathBuf>,
    /// Records observed since the last autosave.
    writes_since_save: AtomicU64,
}

impl FeasibilityBaseline {
    /// Samples retained per config. Small enough to track recent hardware/load
    /// state, large enough for a stable median/MAD.
    const WINDOW: usize = 32;
    /// Minimum samples before the baseline has an opinion; below this the
    /// absolute cold-start floor is used instead.
    const MIN_SAMPLES: usize = 8;
    /// A sample this many robust-sigma below the config median is "slow for
    /// this config", even if it clears the absolute floor.
    const SLOW_Z: f32 = 1.5;
    /// Autosave cadence (records). A few-KB JSON write every N inferences is
    /// negligible next to an LLM call.
    const SAVE_EVERY: u64 = 16;

    pub fn new() -> Self {
        Self::default()
    }

    /// Default persistence path (`<cache>/arkavo/feasibility_baseline.json`),
    /// or `None` if no cache dir is available.
    pub fn default_path() -> Option<PathBuf> {
        dirs::cache_dir().map(|d| d.join("arkavo").join("feasibility_baseline.json"))
    }

    /// Load a persisted baseline from `path` (if present and valid) and retain
    /// `path` for autosave. A missing or corrupt file yields an empty baseline.
    pub fn load(path: PathBuf) -> Self {
        let snapshot = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<FeasibilityBaselineSnapshot>(&s).ok())
            .unwrap_or_default();
        Self {
            windows: RwLock::new(snapshot_to_windows(snapshot)),
            path: Some(path),
            writes_since_save: AtomicU64::new(0),
        }
    }

    /// Restore from an in-memory snapshot (no autosave path attached).
    pub fn restore(snapshot: FeasibilityBaselineSnapshot) -> Self {
        Self {
            windows: RwLock::new(snapshot_to_windows(snapshot)),
            path: None,
            writes_since_save: AtomicU64::new(0),
        }
    }

    /// Serializable snapshot of the current state.
    pub fn snapshot(&self) -> FeasibilityBaselineSnapshot {
        let windows = self
            .windows
            .read()
            .ok()
            .map(|w| {
                w.iter()
                    .map(|(k, v)| (k.clone(), v.iter().copied().collect()))
                    .collect()
            })
            .unwrap_or_default();
        FeasibilityBaselineSnapshot { windows }
    }

    /// Write the snapshot to the autosave path. No-op when no path is set.
    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string(&self.snapshot())
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
    }

    /// Canonical config key. Throughput is comparable only within a
    /// `(model, context-window)` pair on a given host, so both are in the key.
    pub fn key(model_name: &str, n_ctx: u32) -> String {
        format!("{model_name}@{n_ctx}")
    }

    /// Record an observed decode throughput for a config, autosaving every
    /// [`Self::SAVE_EVERY`] records when persistence is enabled.
    pub fn record(&self, key: &str, tokens_per_sec: f32) {
        if !tokens_per_sec.is_finite() || tokens_per_sec <= 0.0 {
            return;
        }
        if let Ok(mut windows) = self.windows.write() {
            let window = windows.entry(key.to_string()).or_default();
            window.push_back(tokens_per_sec);
            while window.len() > Self::WINDOW {
                window.pop_front();
            }
        }
        // Autosave outside the write lock (`save` takes the read lock).
        if self.path.is_some()
            && self.writes_since_save.fetch_add(1, Ordering::Relaxed) + 1 >= Self::SAVE_EVERY
        {
            self.writes_since_save.store(0, Ordering::Relaxed);
            let _ = self.save();
        }
    }

    /// Number of samples held for a config (for telemetry / tests).
    pub fn sample_count(&self, key: &str) -> usize {
        self.windows
            .read()
            .ok()
            .and_then(|w| w.get(key).map(VecDeque::len))
            .unwrap_or(0)
    }

    /// Whether `tokens_per_sec` is slow *relative to this config's own history*.
    ///
    /// Returns `None` until the window has [`Self::MIN_SAMPLES`] samples, so the
    /// caller falls back to the absolute floor during cold start.
    pub fn is_slow(&self, key: &str, tokens_per_sec: f32) -> Option<bool> {
        let windows = self.windows.read().ok()?;
        let window = windows.get(key)?;
        if window.len() < Self::MIN_SAMPLES {
            return None;
        }
        let samples: Vec<f32> = window.iter().copied().collect();
        let z = robust_z(tokens_per_sec, &samples);
        // Negative z = below the config median (slower than usual).
        Some(z <= -Self::SLOW_Z)
    }

    /// Classify feasibility, refining the running verdict with the per-config
    /// baseline.
    ///
    /// Structural verdicts (`assess` decided chunking / smaller-context /
    /// cannot-run) pass through untouched. For a *running* verdict the baseline
    /// can both downgrade (`Now` → `Slowly`, when throughput is far below this
    /// config's median) and upgrade (`Slowly` → `Now`, when a normally-slow
    /// device is running at its usual rate and the absolute floor mislabeled
    /// it). With no baseline opinion the absolute-floor verdict stands.
    pub fn classify(&self, stats: &RuntimeStats, key: &str) -> FeasibilityVerdict {
        let structural = assess(stats);
        if !structural.can_run_local() {
            return structural;
        }
        let Some(tps) = stats.tokens_per_sec else {
            return structural;
        };
        match self.is_slow(key, tps) {
            Some(true) => FeasibilityVerdict::LocalCanRunSlowly,
            Some(false) => FeasibilityVerdict::LocalCanRunNow,
            None => structural,
        }
    }
}

/// Rebuild the bounded windows from a snapshot, dropping non-positive samples
/// and capping each window to [`FeasibilityBaseline::WINDOW`].
fn snapshot_to_windows(snapshot: FeasibilityBaselineSnapshot) -> HashMap<String, VecDeque<f32>> {
    snapshot
        .windows
        .into_iter()
        .map(|(key, samples)| {
            let mut window: VecDeque<f32> = samples
                .into_iter()
                .filter(|x| x.is_finite() && *x > 0.0)
                .collect();
            while window.len() > FeasibilityBaseline::WINDOW {
                window.pop_front();
            }
            (key, window)
        })
        .collect()
}

/// Median of a non-empty slice. Caller guarantees non-empty.
fn median(sorted: &[f32]) -> f32 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        f32::midpoint(sorted[n / 2 - 1], sorted[n / 2])
    }
}

/// Robust z-score: `(x - median) / (1.4826 * MAD)`. The 1.4826 scales MAD to a
/// standard-deviation-equivalent for normal data.
///
/// When a config's history has little spread the MAD collapses to ~0 (common
/// once more than half the retained samples are near-identical), and a bare
/// `1.4826 * MAD` denominator would amplify a marginal dip into an enormous
/// z-score — flagging a 79.9 tok/s sample against an all-80 history as "slow".
/// To keep the scale meaningful, floor it at a small fraction of the median so
/// only a *materially* lower throughput trips the threshold.
fn robust_z(x: f32, samples: &[f32]) -> f32 {
    const EPS: f32 = 1e-6;
    /// Minimum scale as a fraction of the median, used when MAD is ~0.
    const MIN_SCALE_FRAC: f32 = 0.05;
    let mut sorted: Vec<f32> = samples.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let med = median(&sorted);
    let mut deviations: Vec<f32> = sorted.iter().map(|v| (v - med).abs()).collect();
    deviations.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mad = median(&deviations);
    let scale = (1.4826 * mad).max(med.abs() * MIN_SCALE_FRAC).max(EPS);
    (x - med) / scale
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::*;

    fn stats_with_tps(tps: f32) -> RuntimeStats {
        RuntimeStats {
            prompt_tokens: 500,
            n_ctx: 8_192,
            local_model_available: true,
            kv_cache_slots_free: 4,
            tokens_per_sec: Some(tps),
            context_overflow: false,
        }
    }

    #[test]
    fn key_includes_model_and_context() {
        assert_eq!(
            FeasibilityBaseline::key("qwen3.5-9b", 8192),
            "qwen3.5-9b@8192"
        );
    }

    #[test]
    fn no_opinion_until_min_samples() {
        let b = FeasibilityBaseline::new();
        for _ in 0..(FeasibilityBaseline::MIN_SAMPLES - 1) {
            b.record("m@8192", 80.0);
        }
        assert_eq!(b.is_slow("m@8192", 5.0), None);
        // With no opinion, classify falls back to the absolute floor: 5 tok/s
        // is below the 8 tok/s floor, so it reads as slow.
        assert_eq!(
            b.classify(&stats_with_tps(5.0), "m@8192"),
            FeasibilityVerdict::LocalCanRunSlowly
        );
    }

    #[test]
    fn downgrades_fast_config_when_throughput_collapses() {
        let b = FeasibilityBaseline::new();
        // An M3-class config that normally sustains ~80 tok/s.
        for _ in 0..16 {
            b.record("m3@8192", 80.0);
        }
        // 30 tok/s clears the absolute 8 tok/s floor but is far below this
        // config's own median, so it is "slow for this config".
        assert_eq!(b.is_slow("m3@8192", 30.0), Some(true));
        assert_eq!(
            b.classify(&stats_with_tps(30.0), "m3@8192"),
            FeasibilityVerdict::LocalCanRunSlowly
        );
    }

    #[test]
    fn upgrades_slow_device_running_at_its_normal_rate() {
        let b = FeasibilityBaseline::new();
        // A Pi-5-class config whose normal rate is ~5 tok/s (below the floor).
        for _ in 0..16 {
            b.record("pi5@4096", 5.0);
        }
        // 5 tok/s is normal here, so it must NOT be flagged slow even though it
        // is under the absolute cold-start floor.
        assert_eq!(b.is_slow("pi5@4096", 5.0), Some(false));
        assert_eq!(
            b.classify(&stats_with_tps(5.0), "pi5@4096"),
            FeasibilityVerdict::LocalCanRunNow
        );
    }

    #[test]
    fn baseline_never_overrides_structural_verdict() {
        let b = FeasibilityBaseline::new();
        for _ in 0..16 {
            b.record("m@8192", 80.0);
        }
        // Oversized prompt is structural — chunking wins regardless of the
        // throughput baseline.
        let oversized = RuntimeStats {
            prompt_tokens: 9_000,
            n_ctx: 8_192,
            ..stats_with_tps(80.0)
        };
        assert_eq!(
            b.classify(&oversized, "m@8192"),
            FeasibilityVerdict::LocalNeedsChunking
        );
    }

    #[test]
    fn window_is_bounded() {
        let b = FeasibilityBaseline::new();
        for i in 0..(FeasibilityBaseline::WINDOW + 20) {
            b.record("m@8192", 10.0 + i as f32);
        }
        assert_eq!(b.sample_count("m@8192"), FeasibilityBaseline::WINDOW);
    }

    #[test]
    fn robust_z_is_zero_at_median() {
        let samples = vec![10.0, 20.0, 30.0, 40.0, 50.0];
        assert_eq!(robust_z(30.0, &samples), 0.0);
    }

    // Regression (gitar-bot): a near-zero MAD must not amplify a marginal dip
    // into a "slow" verdict. A perfectly stable 80 tok/s history has MAD 0, so
    // without the scale floor a 79.9 sample produced a huge negative z and
    // spurious LocalCanRunSlowly. A *material* drop must still register.
    #[test]
    fn stable_config_does_not_overflag_marginal_dip() {
        let b = FeasibilityBaseline::new();
        for _ in 0..16 {
            b.record("stable@8192", 80.0);
        }
        assert_eq!(
            b.is_slow("stable@8192", 79.9),
            Some(false),
            "a 0.1 tok/s dip on a stable config must not read as slow"
        );
        assert_eq!(
            b.is_slow("stable@8192", 30.0),
            Some(true),
            "a material drop must still read as slow"
        );
    }

    #[test]
    fn non_positive_samples_are_ignored() {
        let b = FeasibilityBaseline::new();
        b.record("m@8192", 0.0);
        b.record("m@8192", -5.0);
        b.record("m@8192", f32::NAN);
        assert_eq!(b.sample_count("m@8192"), 0);
    }

    // Regression: per-config learning must survive a restart. A snapshot
    // round-trip preserves the windows and the resulting slow/fast verdict.
    #[test]
    fn snapshot_round_trip_preserves_learning() {
        let b = FeasibilityBaseline::new();
        for _ in 0..16 {
            b.record("m3@8192", 80.0);
        }
        let restored = FeasibilityBaseline::restore(b.snapshot());
        assert_eq!(restored.sample_count("m3@8192"), 16);
        // The learned distribution carries over: 30 tok/s is still slow here.
        assert_eq!(restored.is_slow("m3@8192", 30.0), Some(true));
        assert_eq!(restored.is_slow("m3@8192", 79.0), Some(false));
    }

    #[test]
    fn snapshot_drops_non_positive_and_caps_window() {
        let mut snapshot = FeasibilityBaselineSnapshot::default();
        let mut samples = vec![0.0, -1.0, f32::NAN];
        samples.extend(std::iter::repeat_n(50.0, FeasibilityBaseline::WINDOW + 10));
        snapshot.windows.insert("m@8192".to_string(), samples);
        let restored = FeasibilityBaseline::restore(snapshot);
        assert_eq!(
            restored.sample_count("m@8192"),
            FeasibilityBaseline::WINDOW,
            "non-positive samples dropped and window capped on restore"
        );
    }

    #[test]
    fn save_and_load_via_disk_round_trip() {
        let dir =
            std::env::temp_dir().join(format!("arkavo_feasibility_test_{}", std::process::id()));
        let path = dir.join("baseline.json");
        let _ = std::fs::remove_file(&path);
        {
            let b = FeasibilityBaseline::load(path.clone());
            for _ in 0..20 {
                b.record("disk@8192", 60.0);
            }
            b.save().unwrap();
        }
        let reloaded = FeasibilityBaseline::load(path.clone());
        assert_eq!(reloaded.sample_count("disk@8192"), 20);
        let _ = std::fs::remove_file(&path);
    }
}
