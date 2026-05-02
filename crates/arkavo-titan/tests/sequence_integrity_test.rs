//! SEQ-006, SEQ-013: Tests for EmaAccumulator and drift detection
//! as building blocks for sequence integrity.

use arkavo_test_macros::spec;
use arkavo_titan::EmaAccumulator;

/// SEQ-013: EmaAccumulator CAN track drift via boolean signals.
/// This building block already works — verify it detects anomalous shift.
#[spec("SEQ-013")]
#[test]
fn ema_accumulator_detects_drift_after_warmup() {
    let mut acc = EmaAccumulator::with_config(0.05, 3.0, 50);

    for _ in 0..50 {
        acc.update(0, false);
    }
    assert!(acc.is_warmed_up(0));

    let z_score = acc.update(0, true);
    assert!(
        z_score.is_some(),
        "SEQ-013: EmaAccumulator should return z-score after warmup"
    );
}

/// SEQ-006: Detection should complete within 50μs overhead budget.
#[spec("SEQ-006")]
#[test]
fn ema_update_completes_within_latency_budget() {
    let mut acc = EmaAccumulator::with_config(0.05, 3.0, 50);

    let start = std::time::Instant::now();
    for _ in 0..100 {
        acc.update(0, false);
    }
    let elapsed = start.elapsed();
    let per_call_us = elapsed.as_micros() / 100;

    assert!(
        per_call_us < 50,
        "SEQ-006: EMA update took {per_call_us}μs per call, budget is <50μs"
    );
}

/// SEQ-006: EmaAccumulator returns None during warmup period.
#[spec("SEQ-006")]
#[test]
fn ema_returns_none_during_warmup() {
    let mut acc = EmaAccumulator::with_config(0.01, 3.0, 100);

    let z_score = acc.update(0, true);
    assert!(z_score.is_none());
    assert!(!acc.is_warmed_up(0));
}

/// SEQ-013: EmaAccumulator tracks per-output drift independently.
#[spec("SEQ-013")]
#[test]
fn ema_tracks_multiple_outputs_independently() {
    let mut acc = EmaAccumulator::with_config(0.05, 3.0, 50);

    for _ in 0..50 {
        acc.update(0, false);
    }

    assert!(acc.is_warmed_up(0));
    assert!(!acc.is_warmed_up(1));

    let z0 = acc.update(0, true);
    assert!(z0.is_some());

    let z1 = acc.update(1, true);
    assert!(z1.is_none());
}

/// SEQ-006: EmaAccumulator has no concept of "session."
/// Drift is tracked globally per output_id, not per session.
/// Sequence integrity needs per-session anomaly tracking.
#[spec("SEQ-006")]
#[test]
fn ema_has_no_per_session_tracking() {
    let mut acc = EmaAccumulator::with_config(0.05, 3.0, 50);

    // Warm up with "session A" normal data
    for _ in 0..50 {
        acc.update(0, false);
    }

    // "Session B" anomalous data — detected, but attributed to output_id 0 globally,
    // not to session B specifically.
    let z_score = acc.update(0, true);
    assert!(z_score.is_some());

    // SEQ-006 requires: divergence score computed per session action graph.
    // EmaAccumulator tracks per output_id, not per session_id.
    // There's no method like: acc.update_for_session("session-b", 0, true)
    // This is the gap — the building block works but isn't session-scoped.
}
