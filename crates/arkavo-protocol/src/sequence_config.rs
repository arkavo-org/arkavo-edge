use std::time::Duration;

/// SEQ-016: Configuration for sequence integrity per agent role
#[derive(Debug, Clone)]
pub struct SequenceIntegrityConfig {
    pub enabled: bool,
    pub taint_tracking: bool,
    pub gate_threshold_low: f64,
    pub gate_threshold_high: f64,
    pub ledger_retention: Duration,
    pub overhead_budget: Duration,
    pub baseline_min_sessions: usize,
}

impl Default for SequenceIntegrityConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            taint_tracking: false,
            gate_threshold_low: 0.0,
            gate_threshold_high: 0.0,
            ledger_retention: Duration::ZERO,
            overhead_budget: Duration::ZERO,
            baseline_min_sessions: 0,
        }
    }
}

impl SequenceIntegrityConfig {
    /// Create a strict config for use when no baseline is available
    pub fn strict() -> Self {
        Self::default()
    }

    /// Validate configuration values
    pub fn validate(&self) -> Result<(), String> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // =========================================================================
    // SEQ-016: Configure sequence integrity per agent role
    // =========================================================================

    #[test]
    fn default_config_has_sane_values() {
        let config = SequenceIntegrityConfig::default();
        assert!(config.enabled);
        assert!(config.taint_tracking);
        assert!(config.gate_threshold_low > 0.0);
        assert!(config.gate_threshold_high > config.gate_threshold_low);
        assert_eq!(config.ledger_retention, Duration::from_secs(86400));
        assert_eq!(config.baseline_min_sessions, 100);
    }

    #[test]
    fn strict_mode_has_lower_thresholds() {
        let strict = SequenceIntegrityConfig::strict();
        let default = SequenceIntegrityConfig::default();
        assert!(strict.gate_threshold_low < default.gate_threshold_low);
        assert!(strict.gate_threshold_high < default.gate_threshold_high);
    }

    #[test]
    fn disabled_config_still_validates() {
        let config = SequenceIntegrityConfig {
            enabled: false,
            ..SequenceIntegrityConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn invalid_thresholds_rejected() {
        let config = SequenceIntegrityConfig {
            gate_threshold_low: 0.8,
            gate_threshold_high: 0.3,
            ..SequenceIntegrityConfig::default()
        };
        assert!(config.validate().is_err());
    }
}
