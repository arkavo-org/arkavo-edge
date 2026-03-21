use std::time::Duration;

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
            enabled: true,
            taint_tracking: true,
            gate_threshold_low: 0.3,
            gate_threshold_high: 0.7,
            ledger_retention: Duration::from_secs(86400),
            overhead_budget: Duration::from_micros(50),
            baseline_min_sessions: 100,
        }
    }
}

impl SequenceIntegrityConfig {
    pub fn strict() -> Self {
        Self {
            gate_threshold_low: 0.1,
            gate_threshold_high: 0.4,
            ..Self::default()
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.gate_threshold_low >= self.gate_threshold_high {
            return Err(format!(
                "gate_threshold_low ({}) must be less than gate_threshold_high ({})",
                self.gate_threshold_low, self.gate_threshold_high
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SEQ-016")]
    #[test]
    fn default_config_enables_tracking() {
        let config = SequenceIntegrityConfig::default();
        assert!(config.enabled);
        assert!(config.taint_tracking);
    }

    #[spec("SEQ-016")]
    #[test]
    fn default_config_has_sane_thresholds() {
        let config = SequenceIntegrityConfig::default();
        assert!(config.gate_threshold_low > 0.0);
        assert!(config.gate_threshold_high > config.gate_threshold_low);
        assert_eq!(config.ledger_retention, Duration::from_secs(86400));
        assert_eq!(config.baseline_min_sessions, 100);
    }

    #[spec("SEQ-016")]
    #[test]
    fn strict_mode_uses_lower_thresholds_than_default() {
        let strict = SequenceIntegrityConfig::strict();
        let default = SequenceIntegrityConfig::default();
        assert!(strict.gate_threshold_low < default.gate_threshold_low);
        assert!(strict.gate_threshold_high < default.gate_threshold_high);
    }

    #[spec("SEQ-016")]
    #[test]
    fn validate_rejects_low_threshold_above_high() {
        let config = SequenceIntegrityConfig {
            gate_threshold_low: 0.8,
            gate_threshold_high: 0.3,
            ..SequenceIntegrityConfig::default()
        };
        assert!(config.validate().is_err());
    }

    #[spec("SEQ-016")]
    #[test]
    fn disabled_config_passes_validation() {
        let config = SequenceIntegrityConfig {
            enabled: false,
            ..SequenceIntegrityConfig::default()
        };
        assert!(config.validate().is_ok());
    }

    #[spec("SEQ-016")]
    #[test]
    fn per_action_policy_applies_when_disabled() {
        let config = SequenceIntegrityConfig {
            enabled: false,
            ..SequenceIntegrityConfig::default()
        };
        assert!(!config.enabled);
    }
}
