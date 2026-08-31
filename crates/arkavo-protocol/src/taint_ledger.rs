//! Ledger projection for taint sets (SEQ-004, SEQ-015).
//!
//! `arkavo-events` sits below this crate and cannot name a `TaintSet`, so the
//! ledger stores taint as plain strings and the conversion lives here, beside
//! the type it reads. Names are spelled out rather than derived from `Debug`:
//! renaming a Rust variant must not silently rewrite what past ledger entries
//! meant.

use crate::data_classification::{DataCategory, SensitivityLevel};
use crate::taint::TaintSet;

impl From<&TaintSet> for arkavo_events::TaintRecord {
    fn from(set: &TaintSet) -> Self {
        let mut provenance = Vec::new();
        let mut truncated_hops = 0u32;
        for label in set.labels() {
            truncated_hops = truncated_hops.saturating_add(label.truncated_hops);
            for hop in &label.hops {
                provenance.push(format!(
                    "{}|{}|{}",
                    label.source_id,
                    hop.transformation.as_str(),
                    hop.detail
                ));
            }
        }
        Self {
            sensitivity: sensitivity_name(set.sensitivity()).to_string(),
            categories: set
                .categories()
                .into_iter()
                .map(|c| category_name(c).to_string())
                .collect(),
            sources: set.source_ids().map(str::to_string).collect(),
            provenance,
            truncated_hops,
        }
    }
}

fn sensitivity_name(level: SensitivityLevel) -> &'static str {
    match level {
        SensitivityLevel::Public => "public",
        SensitivityLevel::Internal => "internal",
        SensitivityLevel::Confidential => "confidential",
        SensitivityLevel::Restricted => "restricted",
    }
}

fn category_name(category: DataCategory) -> &'static str {
    match category {
        DataCategory::Pii => "pii",
        DataCategory::Credentials => "credentials",
        DataCategory::Financial => "financial",
        DataCategory::Healthcare => "healthcare",
        DataCategory::Internal => "internal",
        DataCategory::Public => "public",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taint::{MAX_PROVENANCE_HOPS, TaintLabel, Transformation};

    fn label(source: &str, category: DataCategory, level: SensitivityLevel) -> TaintLabel {
        TaintLabel::new(source, [category], level)
    }

    #[test]
    fn ledger_projection_carries_sensitivity_categories_and_provenance() {
        let set = TaintSet::from_label(label(
            "file:/etc/creds",
            DataCategory::Credentials,
            SensitivityLevel::Restricted,
        ))
        .transformed(Transformation::Encode, "base64");

        let record: arkavo_events::TaintRecord = (&set).into();

        assert_eq!(record.sensitivity, "restricted");
        assert_eq!(record.categories, vec!["credentials".to_string()]);
        assert_eq!(record.sources, vec!["file:/etc/creds".to_string()]);
        assert_eq!(
            record.provenance,
            vec!["file:/etc/creds|encode|base64".to_string()]
        );
        assert_eq!(record.truncated_hops, 0);
    }

    #[test]
    fn ledger_projection_reports_truncated_provenance() {
        let mut set = TaintSet::from_label(label(
            "file:loop",
            DataCategory::Internal,
            SensitivityLevel::Internal,
        ));
        for i in 0..(MAX_PROVENANCE_HOPS + 3) {
            set = set.transformed(Transformation::Encode, &format!("codec-{i}"));
        }

        let record: arkavo_events::TaintRecord = (&set).into();

        assert_eq!(record.truncated_hops, 3);
    }
}
