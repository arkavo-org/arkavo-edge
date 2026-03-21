use crate::EmaAccumulator;

#[derive(Debug, Clone)]
pub struct SequenceFeatureVector {
    pub action_count: usize,
    pub unique_tools: usize,
    pub tainted_ratio: f64,
    pub max_path_length: usize,
}

pub struct TitanSequenceBridge {
    accumulator: EmaAccumulator,
    drifting: bool,
}

impl TitanSequenceBridge {
    pub fn new() -> Self {
        Self {
            accumulator: EmaAccumulator::with_config(0.05, 3.0, 50),
            drifting: false,
        }
    }

    pub fn track_sequence_entropy(&mut self, features: &SequenceFeatureVector) -> Option<f64> {
        // Map feature vector to a boolean signal for EMA tracking:
        // high tainted_ratio or unusual action count = true (anomalous)
        let is_anomalous = features.tainted_ratio > 0.5
            || features.action_count > 20
            || features.max_path_length > 15;

        let z_score = self.accumulator.update(0, is_anomalous);
        if let Some(z) = z_score {
            if z.abs() > 3.0 {
                self.drifting = true;
            }
        }
        z_score
    }

    pub fn is_drifting(&self) -> bool {
        self.drifting
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn normal_features() -> SequenceFeatureVector {
        SequenceFeatureVector {
            action_count: 5,
            unique_tools: 3,
            tainted_ratio: 0.2,
            max_path_length: 4,
        }
    }

    #[spec("SEQ-013")]
    #[test]
    fn ema_returns_none_or_normal_during_warmup() {
        let mut bridge = TitanSequenceBridge::new();
        let z_score = bridge.track_sequence_entropy(&normal_features());
        assert!(z_score.is_none() || z_score.unwrap().abs() < 3.0);
    }

    #[spec("SEQ-013")]
    #[test]
    fn detects_drift_after_behavioral_shift() {
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

    #[spec("SEQ-013")]
    #[test]
    fn no_drift_during_warmup() {
        let bridge = TitanSequenceBridge::new();
        assert!(!bridge.is_drifting());
    }
}
