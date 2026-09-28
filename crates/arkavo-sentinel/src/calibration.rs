//! Per-label thresholds (SENT-004).
//!
//! Thresholds live in the pack manifest, not here. A threshold compiled into
//! this crate would be a calibration that cannot be revised without a release,
//! against a detector that is revised on its own schedule — and the pairing
//! that matters is (detector version, taxonomy version, threshold), which only
//! the manifest holds all three of.
//!
//! A label the table does not mention is not thereby safe. The omission is
//! recorded and the label fires: the conservative reading of "we never
//! calibrated this" is that we cannot yet trust it to stay quiet.

use std::collections::BTreeMap;

use arkavo_protocol::classification_evidence::Confidence;
use serde::{Deserialize, Serialize};

/// Thresholds for one detector against one taxonomy version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationTable {
    pub detector_version: String,
    pub taxonomy_version: String,
    /// Label name to the confidence at which it fires.
    thresholds: BTreeMap<String, f32>,
}

/// Why a label fired at the threshold it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThresholdSource {
    /// Read from the table.
    Calibrated,
    /// Absent from the table. The label fires and the omission is reported.
    Uncalibrated,
}

impl CalibrationTable {
    pub fn new(detector_version: impl Into<String>, taxonomy_version: impl Into<String>) -> Self {
        Self {
            detector_version: detector_version.into(),
            taxonomy_version: taxonomy_version.into(),
            thresholds: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_threshold(mut self, label: impl Into<String>, threshold: Confidence) -> Self {
        self.thresholds.insert(label.into(), threshold.value());
        self
    }

    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// The threshold for a label, and whether it came from the table.
    ///
    /// An uncalibrated label gets a threshold of zero — it fires — rather than
    /// a compiled-in guess. Zero is not a calibration; it is the absence of one
    /// expressed so that the absence cannot hide anything.
    pub fn threshold(&self, label: &str) -> (Confidence, ThresholdSource) {
        match self.thresholds.get(label) {
            Some(value) => (Confidence::new(*value), ThresholdSource::Calibrated),
            None => (Confidence::new(0.0), ThresholdSource::Uncalibrated),
        }
    }

    /// Labels the detector emitted that this table does not calibrate, so the
    /// omission can be reported rather than inferred from quiet evidence.
    pub fn uncalibrated<'a>(&self, emitted: impl Iterator<Item = &'a str>) -> Vec<String> {
        emitted
            .filter(|label| !self.thresholds.contains_key(*label))
            .map(str::to_string)
            .collect()
    }

    /// SENT-015: refuse a taxonomy version this table was not calibrated
    /// against, rather than mapping labels it never saw onto known attributes.
    pub fn accepts_taxonomy(&self, taxonomy_version: &str) -> bool {
        self.taxonomy_version == taxonomy_version
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn table() -> CalibrationTable {
        CalibrationTable::new("sentinel-0.1", "1.0.0")
            .with_threshold("credentials", Confidence::new(0.8))
    }

    /// SENT-004: thresholds come from the table, not from the crate.
    #[spec("SENT-004")]
    #[test]
    fn a_calibrated_label_uses_the_recorded_threshold() {
        let (threshold, source) = table().threshold("credentials");

        assert_eq!(threshold, Confidence::new(0.8));
        assert_eq!(source, ThresholdSource::Calibrated);
    }

    /// SENT-004 edge case: a label the manifest omits fires, and the omission
    /// is reportable.
    #[spec("SENT-004")]
    #[test]
    fn an_uncalibrated_label_fires_and_is_reported() {
        let table = table();

        let (threshold, source) = table.threshold("healthcare");

        assert_eq!(threshold, Confidence::new(0.0));
        assert_eq!(source, ThresholdSource::Uncalibrated);
        assert_eq!(
            table.uncalibrated(["credentials", "healthcare"].into_iter()),
            vec!["healthcare".to_string()]
        );
    }

    /// SENT-015: a taxonomy the table was not calibrated against is refused.
    #[spec("SENT-015")]
    #[test]
    fn a_taxonomy_version_mismatch_is_refused() {
        let table = table();

        assert!(table.accepts_taxonomy("1.0.0"));
        assert!(!table.accepts_taxonomy("2.0.0"));
    }

    #[test]
    fn a_table_round_trips_through_json() {
        // The manifest carries it as JSON, so this is the wire form.
        let json = serde_json::to_string(&table()).expect("serialize");

        let restored = CalibrationTable::from_json(&json).expect("deserialize");

        assert_eq!(restored.threshold("credentials").0, Confidence::new(0.8));
        assert_eq!(restored.detector_version, "sentinel-0.1");
    }
}
