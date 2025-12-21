//! Statistical accumulator for drift detection
//!
//! Uses Exponential Moving Average (EMA) for O(1) update and query.

/// Maximum number of outputs to track
pub const MAX_OUTPUTS: usize = 32;

/// Default smoothing factor (0.01 = slow adaptation)
pub const DEFAULT_ALPHA: f32 = 0.01;

/// Default z-score threshold for anomaly detection (3 sigma)
pub const DEFAULT_Z_THRESHOLD: f32 = 3.0;

/// Exponential Moving Average accumulator for output distribution tracking
///
/// Tracks the ratio of `true` outputs over time for each output ID.
/// Designed for fixed-size, zero-allocation operation.
#[derive(Debug, Clone)]
pub struct EmaAccumulator {
    /// EMA per output ID: tracks true/false ratio (0.0 = always false, 1.0 = always true)
    output_ema: [f32; MAX_OUTPUTS],
    /// Variance estimate per output (for z-score calculation)
    output_var: [f32; MAX_OUTPUTS],
    /// Sample count per output (for warm-up detection)
    sample_count: [u32; MAX_OUTPUTS],
    /// Threshold for z-score anomaly
    z_threshold: f32,
    /// Smoothing factor (0.01 = slow adaptation)
    alpha: f32,
    /// Minimum samples before drift detection activates
    min_samples: u32,
}

impl Default for EmaAccumulator {
    fn default() -> Self {
        Self::new()
    }
}

impl EmaAccumulator {
    /// Create a new accumulator with default settings
    #[must_use]
    pub fn new() -> Self {
        Self {
            output_ema: [0.5; MAX_OUTPUTS], // Start with 50% baseline
            output_var: [0.25; MAX_OUTPUTS], // Maximum variance for binary
            sample_count: [0; MAX_OUTPUTS],
            z_threshold: DEFAULT_Z_THRESHOLD,
            alpha: DEFAULT_ALPHA,
            min_samples: 100, // Warm-up period
        }
    }

    /// Create with custom settings
    #[must_use]
    pub fn with_config(alpha: f32, z_threshold: f32, min_samples: u32) -> Self {
        Self {
            output_ema: [0.5; MAX_OUTPUTS],
            output_var: [0.25; MAX_OUTPUTS],
            sample_count: [0; MAX_OUTPUTS],
            z_threshold,
            alpha,
            min_samples,
        }
    }

    /// Update the EMA for an output value
    ///
    /// Returns the z-score if a drift anomaly is detected.
    #[inline]
    pub fn update(&mut self, output_id: u16, value: bool) -> Option<f64> {
        let idx = output_id as usize;
        if idx >= MAX_OUTPUTS {
            return None;
        }

        let x = if value { 1.0_f32 } else { 0.0_f32 };
        let old_ema = self.output_ema[idx];

        // Update EMA: ema = alpha * x + (1 - alpha) * ema
        let new_ema = self.alpha.mul_add(x, (1.0 - self.alpha) * old_ema);
        self.output_ema[idx] = new_ema;

        // Update variance estimate using Welford's online algorithm (simplified)
        let diff = x - old_ema;
        let new_var = self.alpha.mul_add(diff * diff, (1.0 - self.alpha) * self.output_var[idx]);
        self.output_var[idx] = new_var;

        // Increment sample count (saturating)
        self.sample_count[idx] = self.sample_count[idx].saturating_add(1);

        // Check for drift if we have enough samples
        if self.sample_count[idx] >= self.min_samples {
            let std_dev = self.output_var[idx].sqrt().max(0.001); // Avoid division by zero
            let z_score = (x - old_ema).abs() / std_dev;

            if z_score > self.z_threshold {
                return Some(f64::from(z_score));
            }
        }

        None
    }

    /// Get the current EMA for an output
    #[inline]
    #[must_use]
    pub fn get_ema(&self, output_id: u16) -> Option<f64> {
        let idx = output_id as usize;
        if idx >= MAX_OUTPUTS {
            None
        } else {
            Some(f64::from(self.output_ema[idx]))
        }
    }

    /// Check if an output is warmed up (has enough samples)
    #[inline]
    #[must_use]
    pub fn is_warmed_up(&self, output_id: u16) -> bool {
        let idx = output_id as usize;
        idx < MAX_OUTPUTS && self.sample_count[idx] >= self.min_samples
    }

    /// Reset statistics for all outputs
    pub fn reset(&mut self) {
        self.output_ema = [0.5; MAX_OUTPUTS];
        self.output_var = [0.25; MAX_OUTPUTS];
        self.sample_count = [0; MAX_OUTPUTS];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ema_update() {
        let mut acc = EmaAccumulator::new();

        // Update with true values
        for _ in 0..50 {
            acc.update(0, true);
        }

        let ema = acc.get_ema(0).unwrap();
        assert!(ema > 0.5); // Should trend towards 1.0
    }

    #[test]
    fn test_ema_convergence() {
        let mut acc = EmaAccumulator::with_config(0.1, 3.0, 10); // Faster convergence for testing

        // Update with all true values
        for _ in 0..100 {
            acc.update(0, true);
        }

        let ema = acc.get_ema(0).unwrap();
        assert!(ema > 0.95); // Should be close to 1.0
    }

    #[test]
    fn test_drift_detection() {
        let mut acc = EmaAccumulator::with_config(0.1, 2.0, 10);

        // Warm up with true values
        for _ in 0..20 {
            acc.update(0, true);
        }

        // Now send false - should trigger drift
        let z_score = acc.update(0, false);
        assert!(z_score.is_some());
    }

    #[test]
    fn test_warmup_period() {
        let mut acc = EmaAccumulator::new();

        assert!(!acc.is_warmed_up(0));

        // Not enough samples yet
        for _ in 0..50 {
            let result = acc.update(0, false);
            assert!(result.is_none()); // No drift during warm-up
        }

        assert!(!acc.is_warmed_up(0));

        // Complete warm-up
        for _ in 0..50 {
            acc.update(0, false);
        }

        assert!(acc.is_warmed_up(0));
    }

    #[test]
    fn test_out_of_range() {
        let mut acc = EmaAccumulator::new();

        // Output ID beyond MAX_OUTPUTS should be ignored
        let result = acc.update(100, true);
        assert!(result.is_none());

        let ema = acc.get_ema(100);
        assert!(ema.is_none());
    }
}
