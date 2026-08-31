//! Taxonomy map: taint categories to OpenTDF attributes (SEQ-003).
//!
//! The map is the only place that says what a label *means* in policy terms.
//! Keeping it as data rather than code is what lets a tenant change the
//! compartments without changing the gate, and what lets an auditor read the
//! rule that produced a decision.
//!
//! The v1 map ships embedded. A tenant map arrives through [`TaxonomyMap::from_json`]
//! and is validated on load, because a map that silently fails to parse would
//! degrade to no requirements at all — which is the wrong direction to fail in.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::data_classification::{DataCategory, SensitivityLevel};

/// The v1 map, embedded so a deployment cannot lose it or swap it unnoticed.
const V1_JSON: &str = include_str!("../../../schemas/taxonomy-map.v1.json");

static V1: LazyLock<TaxonomyMap> = LazyLock::new(|| {
    TaxonomyMap::from_json(V1_JSON).expect("embedded taxonomy-map.v1.json is malformed")
});

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TaxonomyError {
    #[error("taxonomy map is not valid JSON: {0}")]
    Malformed(String),
    #[error("taxonomy map declares no labels")]
    NoLabels,
    #[error("taxonomy map label '{0}' names unknown category '{1}'")]
    UnknownCategory(String, String),
}

/// One attribute a subject must hold, as an OpenTDF fully-qualified name and
/// value. Requirements combine conjunctively: holding one never implies another.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AttributeRequirement {
    pub fqn: String,
    pub value: String,
}

impl AttributeRequirement {
    pub fn new(fqn: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            fqn: fqn.into(),
            value: value.into(),
        }
    }
}

/// What to do for a subject that does not hold a label's requirements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unentitled {
    Block,
    Redact,
}

/// Policy for one taxonomy label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LabelPolicy {
    pub label: String,
    pub category: DataCategory,
    pub sensitivity: SensitivityLevel,
    /// Attributes a subject must hold to receive this label in the clear.
    pub requires: Vec<AttributeRequirement>,
    /// Attributes stamped onto the TDF policy when this label is wrapped.
    pub wrap_attributes: Vec<AttributeRequirement>,
    pub unentitled: Unentitled,
    /// Content that no entitlement and no wrapping rescues.
    pub never_release: bool,
}

/// Category to attribute mapping, loaded from a taxonomy map document.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaxonomyMap {
    version: String,
    namespace: String,
    labels: BTreeMap<DataCategory, LabelPolicy>,
}

impl TaxonomyMap {
    /// The embedded v1 map.
    pub fn v1() -> &'static TaxonomyMap {
        &V1
    }

    pub fn from_json(json: &str) -> Result<Self, TaxonomyError> {
        let doc: raw::Document =
            serde_json::from_str(json).map_err(|e| TaxonomyError::Malformed(e.to_string()))?;
        if doc.labels.is_empty() {
            return Err(TaxonomyError::NoLabels);
        }

        let mut labels = BTreeMap::new();
        for label in doc.labels {
            let category = parse_category(&label.category).ok_or_else(|| {
                TaxonomyError::UnknownCategory(label.name.clone(), label.category.clone())
            })?;
            let sensitivity = parse_sensitivity(&label.sensitivity).ok_or_else(|| {
                TaxonomyError::UnknownCategory(label.name.clone(), label.sensitivity.clone())
            })?;
            labels.insert(
                category,
                LabelPolicy {
                    label: label.name,
                    category,
                    sensitivity,
                    requires: label.requires,
                    wrap_attributes: label.wrap_attributes,
                    unentitled: label.unentitled,
                    never_release: label.never_release,
                },
            );
        }

        Ok(Self {
            version: doc.version,
            namespace: doc.namespace,
            labels,
        })
    }

    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn policy_for(&self, category: DataCategory) -> Option<&LabelPolicy> {
        self.labels.get(&category)
    }

    /// Every attribute a subject must hold for all of `categories`.
    ///
    /// The union, not the maximum: `clearance` is hierarchical inside its own
    /// definition, but a `department` grant is orthogonal to it, so dropping one
    /// requirement because another looks stronger would widen access.
    pub fn requirements_for(
        &self,
        categories: impl IntoIterator<Item = DataCategory>,
    ) -> BTreeSet<AttributeRequirement> {
        categories
            .into_iter()
            .filter_map(|c| self.labels.get(&c))
            .flat_map(|p| p.requires.iter().cloned())
            .collect()
    }

    /// Attributes to stamp on a TDF policy covering all of `categories`.
    pub fn wrap_attributes_for(
        &self,
        categories: impl IntoIterator<Item = DataCategory>,
    ) -> BTreeSet<AttributeRequirement> {
        categories
            .into_iter()
            .filter_map(|c| self.labels.get(&c))
            .flat_map(|p| p.wrap_attributes.iter().cloned())
            .collect()
    }

    /// Whether any category present must never leave, regardless of entitlement.
    pub fn never_release(&self, categories: impl IntoIterator<Item = DataCategory>) -> bool {
        categories
            .into_iter()
            .filter_map(|c| self.labels.get(&c))
            .any(|p| p.never_release)
    }

    /// Categories that must never leave, for an audit record that has to name them.
    pub fn never_release_categories(
        &self,
        categories: impl IntoIterator<Item = DataCategory>,
    ) -> Vec<DataCategory> {
        categories
            .into_iter()
            .filter(|c| self.labels.get(c).is_some_and(|p| p.never_release))
            .collect()
    }
}

/// Category names are matched against the spelled-out taxonomy vocabulary
/// rather than derived from the Rust identifier, so renaming a variant cannot
/// silently change which policy a stored map selects.
fn parse_category(name: &str) -> Option<DataCategory> {
    match name {
        "Pii" => Some(DataCategory::Pii),
        "Credentials" => Some(DataCategory::Credentials),
        "Financial" => Some(DataCategory::Financial),
        "Healthcare" => Some(DataCategory::Healthcare),
        "Internal" => Some(DataCategory::Internal),
        "Public" => Some(DataCategory::Public),
        _ => None,
    }
}

fn parse_sensitivity(name: &str) -> Option<SensitivityLevel> {
    match name {
        "Public" => Some(SensitivityLevel::Public),
        "Internal" => Some(SensitivityLevel::Internal),
        "Confidential" => Some(SensitivityLevel::Confidential),
        "Restricted" => Some(SensitivityLevel::Restricted),
        _ => None,
    }
}

/// Wire shapes for the taxonomy map document. Separate from the runtime types
/// so the JSON schema can evolve without the gate's vocabulary moving with it.
mod raw {
    use super::{AttributeRequirement, Unentitled};
    use serde::Deserialize;

    #[derive(Deserialize)]
    pub(super) struct Document {
        pub(super) version: String,
        pub(super) namespace: String,
        pub(super) labels: Vec<Label>,
    }

    #[derive(Deserialize)]
    pub(super) struct Label {
        #[serde(rename = "label")]
        pub(super) name: String,
        pub(super) category: String,
        pub(super) sensitivity: String,
        #[serde(default)]
        pub(super) requires: Vec<AttributeRequirement>,
        #[serde(default, rename = "wrapAttributes")]
        pub(super) wrap_attributes: Vec<AttributeRequirement>,
        pub(super) unentitled: Unentitled,
        #[serde(default, rename = "neverRelease")]
        pub(super) never_release: bool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_v1_map_loads() {
        let map = TaxonomyMap::v1();

        assert_eq!(map.version(), "1.0.0");
        assert_eq!(map.namespace(), "https://attr.arkavo.com/");
    }

    #[test]
    fn credentials_never_release() {
        let map = TaxonomyMap::v1();

        assert!(map.never_release([DataCategory::Credentials]));
        assert!(!map.never_release([DataCategory::Pii, DataCategory::Internal]));
    }

    #[test]
    fn pii_floor_matches_the_detector_default() {
        // The map must not reclassify what DatumType already asserts, or wiring
        // it in would move existing detections without anyone deciding to.
        let map = TaxonomyMap::v1();
        let pii = map.policy_for(DataCategory::Pii).expect("pii label");

        assert_eq!(
            pii.sensitivity,
            crate::data_classification::DatumType::Email.default_sensitivity()
        );
    }

    #[test]
    fn financial_requirements_are_conjunctive() {
        let map = TaxonomyMap::v1();

        let reqs = map.requirements_for([DataCategory::Financial]);

        assert!(reqs.contains(&AttributeRequirement::new(
            "https://attr.arkavo.com/clearance",
            "confidential"
        )));
        assert!(reqs.contains(&AttributeRequirement::new(
            "https://attr.arkavo.com/department",
            "finance"
        )));
    }

    #[test]
    fn requirements_union_rather_than_dominate() {
        let map = TaxonomyMap::v1();

        // Healthcare needs restricted clearance; financial needs confidential
        // plus a department. Neither subsumes the other.
        let reqs = map.requirements_for([DataCategory::Healthcare, DataCategory::Financial]);

        assert!(reqs.contains(&AttributeRequirement::new(
            "https://attr.arkavo.com/clearance",
            "restricted"
        )));
        assert!(reqs.contains(&AttributeRequirement::new(
            "https://attr.arkavo.com/department",
            "finance"
        )));
    }

    #[test]
    fn public_content_carries_no_requirement() {
        let map = TaxonomyMap::v1();

        assert!(map.requirements_for([DataCategory::Public]).is_empty());
    }

    #[test]
    fn a_malformed_map_is_an_error_not_an_empty_policy() {
        let err = TaxonomyMap::from_json("{ not json").unwrap_err();

        assert!(matches!(err, TaxonomyError::Malformed(_)));
    }

    #[test]
    fn a_map_with_no_labels_is_rejected() {
        let json = r#"{"version":"9","namespace":"https://x/","labels":[]}"#;

        assert_eq!(
            TaxonomyMap::from_json(json).unwrap_err(),
            TaxonomyError::NoLabels
        );
    }

    #[test]
    fn an_unknown_category_is_rejected() {
        let json = r#"{"version":"9","namespace":"https://x/","labels":[
            {"label":"x","category":"Nonsense","sensitivity":"Public","unentitled":"block"}]}"#;

        assert!(matches!(
            TaxonomyMap::from_json(json).unwrap_err(),
            TaxonomyError::UnknownCategory(_, _)
        ));
    }
}
