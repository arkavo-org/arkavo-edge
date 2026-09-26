//! Per-tier thresholds from a pack manifest (SENT-004).
//!
//! `manifest.thresholds` is one JSON value that must serve two independent
//! calibrations: `CalibrationTable` for the keyed tiers and
//! `SemanticCalibration` for the embedding tier. They cannot share one type:
//! `CalibrationTable` thresholds are `Confidence`s, clamped to `0.0..=1.0`,
//! while a semantic margin is a `cos - cos` difference that can be negative
//! and would be silently clamped into a different threshold. A manifest built
//! before the semantic tier existed still carries a bare `CalibrationTable`,
//! so that shape is read as the sentinel tier rather than refused. Both shapes
//! are pinned to the manifest's own taxonomy version here, once, so `load.rs`
//! never has to re-derive that check per tier.

use arkavo_fingerprint::{EmbedderRecord, SemanticCalibration};
use arkavo_sentinel::CalibrationTable;

use crate::load::LoadError;

/// The thresholds a pack manifest carries, one slot per tier that can be
/// calibrated. Either may be absent: a pack calibrated for only one tier is
/// not thereby malformed.
pub struct PackThresholds {
    pub sentinel: Option<CalibrationTable>,
    pub semantic: Option<SemanticCalibration>,
}

/// Read `manifest.thresholds` into per-tier calibrations, checked against
/// `taxonomy_version`.
///
/// `null` is refused outright: a pack with no thresholds at all cannot run the
/// sentinel tier, and there is no default that would not be a fabricated
/// threshold. An object carrying `"detector_version"` is a bare
/// `CalibrationTable` — the shape every pack used before the semantic tier
/// existed — and is read as the sentinel tier alone. Any other object's keys
/// must be a subset of `{"sentinel", "semantic"}`; anything else names a tier
/// this loader does not know, and is refused rather than silently ignored.
pub fn read_thresholds(
    value: &serde_json::Value,
    taxonomy_version: &str,
) -> Result<PackThresholds, LoadError> {
    if value.is_null() {
        return Err(LoadError::NoThresholds);
    }
    let Some(obj) = value.as_object() else {
        return Err(LoadError::BadThresholds(
            "thresholds must be a JSON object".to_string(),
        ));
    };
    if obj.is_empty() {
        // `{}` names no tier at all. Before the two-tier reader existed this
        // shape was refused too — `CalibrationTable` has no serde default, so
        // an empty object never deserialized into one — and an object with no
        // keys is exactly as uncalibrated as no thresholds at all.
        return Err(LoadError::NoThresholds);
    }
    if obj.contains_key("detector_version") {
        return Ok(PackThresholds {
            sentinel: Some(parse_sentinel(value, taxonomy_version)?),
            semantic: None,
        });
    }
    for key in obj.keys() {
        if key != "sentinel" && key != "semantic" {
            return Err(LoadError::BadThresholds(format!(
                "unknown threshold tier {key}"
            )));
        }
    }
    let sentinel = obj
        .get("sentinel")
        .map(|v| parse_sentinel(v, taxonomy_version))
        .transpose()?;
    let semantic = obj
        .get("semantic")
        .map(|v| parse_semantic(v, taxonomy_version))
        .transpose()?;
    Ok(PackThresholds { sentinel, semantic })
}

/// The semantic tier's embedder record, read straight off the manifest's raw
/// thresholds.
///
/// Read before any pack component is opened, so an entry point that only
/// needs to know which model to fetch (e.g. `LlamaEmbedder::load`) never has
/// to run the rest of `read_thresholds`' taxonomy checks to get it.
pub fn semantic_embedder_record(value: &serde_json::Value) -> Option<EmbedderRecord> {
    let semantic = value.get("semantic")?;
    serde_json::from_value(semantic.get("embedder")?.clone()).ok()
}

fn parse_sentinel(
    value: &serde_json::Value,
    taxonomy_version: &str,
) -> Result<CalibrationTable, LoadError> {
    let table: CalibrationTable = serde_json::from_value(value.clone())
        .map_err(|e| LoadError::BadThresholds(e.to_string()))?;
    if !table.accepts_taxonomy(taxonomy_version) {
        return Err(LoadError::TaxonomyMismatch {
            manifest: taxonomy_version.to_string(),
            thresholds: table.taxonomy_version,
        });
    }
    Ok(table)
}

fn parse_semantic(
    value: &serde_json::Value,
    taxonomy_version: &str,
) -> Result<SemanticCalibration, LoadError> {
    let calibration: SemanticCalibration = serde_json::from_value(value.clone())
        .map_err(|e| LoadError::BadThresholds(e.to_string()))?;
    if calibration.taxonomy_version != taxonomy_version {
        return Err(LoadError::TaxonomyMismatch {
            manifest: taxonomy_version.to_string(),
            thresholds: calibration.taxonomy_version,
        });
    }
    Ok(calibration)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> serde_json::Value {
        serde_json::json!({"detector_version":"sentinel-0.1","taxonomy_version":"1.0.0","thresholds":{}})
    }

    #[test]
    fn a_bare_table_reads_as_the_sentinel_tier() {
        let t = read_thresholds(&table(), "1.0.0").unwrap();
        assert!(t.sentinel.is_some() && t.semantic.is_none());
    }

    #[test]
    fn the_object_form_reads_both_tiers() {
        let semantic = serde_json::json!({
            "detector_version":"semantic:abc","taxonomy_version":"1.0.0",
            "margins":{"internal:confidential":0.1},
            "embedder":{"source":"o/m/f.gguf","sha256":"abc","pooling":"last"}});
        let v = serde_json::json!({"sentinel": table(), "semantic": semantic});
        let t = read_thresholds(&v, "1.0.0").unwrap();
        assert!(t.sentinel.is_some() && t.semantic.is_some());
        assert_eq!(semantic_embedder_record(&v).unwrap().sha256, "abc");
    }

    #[test]
    fn a_semantic_only_pack_has_no_sentinel_table() {
        let v = serde_json::json!({"semantic": {
            "detector_version":"semantic:abc","taxonomy_version":"1.0.0","margins":{},
            "embedder":{"source":"o/m/f.gguf","sha256":"abc","pooling":"last"}}});
        assert!(read_thresholds(&v, "1.0.0").unwrap().sentinel.is_none());
    }

    #[test]
    fn null_thresholds_are_still_refused() {
        assert!(matches!(
            read_thresholds(&serde_json::Value::Null, "1.0.0"),
            Err(LoadError::NoThresholds)
        ));
    }

    /// An empty object names no tier at all. Before the semantic tier existed
    /// this shape was refused too, because `CalibrationTable` has no serde
    /// default and `{}` does not deserialize into one; the two-tier reader
    /// must not accidentally re-open that as "calibrated for nothing, and
    /// that's fine."
    #[test]
    fn empty_thresholds_are_refused() {
        assert!(matches!(
            read_thresholds(&serde_json::json!({}), "1.0.0"),
            Err(LoadError::NoThresholds)
        ));
    }

    #[test]
    fn an_unknown_tier_key_is_refused() {
        let v = serde_json::json!({"mystery": {}});
        assert!(matches!(
            read_thresholds(&v, "1.0.0"),
            Err(LoadError::BadThresholds(_))
        ));
    }

    #[test]
    fn a_semantic_table_for_another_taxonomy_is_refused() {
        let v = serde_json::json!({"semantic": {
            "detector_version":"semantic:abc","taxonomy_version":"2.0.0","margins":{},
            "embedder":{"source":"o/m/f.gguf","sha256":"abc","pooling":"last"}}});
        assert!(matches!(
            read_thresholds(&v, "1.0.0"),
            Err(LoadError::TaxonomyMismatch { .. })
        ));
    }
}
