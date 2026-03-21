use crate::EmaAccumulator;

/// Feature vector extracted from action sequence context
#[derive(Debug, Clone)]
pub struct SequenceFeatureVector {
    pub action_count: usize,
    pub unique_tools: usize,
    pub tainted_ratio: f64,
    pub max_path_length: usize,
}

/// Bridge between sequence integrity system and TitanMonitor
pub struct TitanSequenceBridge {
    _accumulator: EmaAccumulator,
}

impl TitanSequenceBridge {
    pub fn new() -> Self {
        Self {
            _accumulator: EmaAccumulator::default(),
        }
    }

    /// SEQ-013: Feed sequence features into EMA accumulator for drift detection
    pub fn track_sequence_entropy(
        &mut self,
        _features: &SequenceFeatureVector,
    ) -> Option<f64> {
        None
    }

    /// Check if current sequence shows statistical drift
    pub fn is_drifting(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normal_features() -> SequenceFeatureVector {
        SequenceFeatureVector {
            action_count: 5,
            unique_tools: 3,
            tainted_ratio: 0.2,
            max_path_length: 4,
        }
    }

    // =========================================================================
    // SEQ-013: Titan integration for statistical sequence drift
    // =========================================================================

    #[test]
    fn ema_tracks_sequence_entropy() {
        let mut bridge = TitanSequenceBridge::new();
        let features = normal_features();
        let z_score = bridge.track_sequence_entropy(&features);
        assert!(z_score.is_none() || z_score.unwrap().abs() < 3.0);
    }

    #[test]
    fn drift_detected_after_behavioral_shift() {
        let mut bridge = TitanSequenceBridge::new();
        for _ in 0..100 {
            bridge.track_sequence_entropy(&normal_features());
        }
        let anomalous = SequenceFeatureVector {
            action_count: 50,
            unique_tools: 20,
            tainted_ratio: 0.9,
            max_path_length: 30,
        };
        let z_score = bridge.track_sequence_entropy(&anomalous);
        assert!(z_score.is_some());
        assert!(z_score.unwrap().abs() > 3.0);
    }

    #[test]
    fn no_drift_during_warmup() {
        let bridge = TitanSequenceBridge::new();
        assert!(!bridge.is_drifting());
    }
}
