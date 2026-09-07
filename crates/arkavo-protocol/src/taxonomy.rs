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
    #[error("taxonomy map label '{0}' names unknown sensitivity '{1}'")]
    UnknownSensitivity(String, String),
    #[error("taxonomy map declares category '{0}' more than once")]
    DuplicateCategory(String),
    #[error("derived definition '{0}' uses the prescribed attribute namespace")]
    DerivedInPrescribedNamespace(String),
    #[error("derived definition '{0}' must be ALL_OF")]
    DerivedMustBeAllOf(String),
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
            fqn: canonical_definition_fqn(&fqn.into()),
            value: value.into(),
        }
    }

    /// Platform value FQN: `https://<ns>/attr/<def>/value/<val>`.
    ///
    /// Everything on a TDF and in entitlements is this form. Definition FQNs
    /// (`…/attr/<def>`) never go on the payload.
    pub fn as_attribute_uri(&self) -> String {
        canonical_value_fqn(&self.fqn, &self.value)
    }
}

/// Definition FQN: `https://<namespace>/attr/<name>` (or `/obl/<name>`).
///
/// Shorthand `https://attr.arkavo.com/clearance` is rewritten so a map or a
/// caller that dropped `/attr/` still emits what `opentdf-rs` parses.
pub fn canonical_definition_fqn(fqn: &str) -> String {
    let trimmed = fqn.trim().trim_end_matches('/');
    if trimmed.contains("/attr/") || trimmed.contains("/obl/") {
        return trimmed.to_string();
    }
    match trimmed.rsplit_once('/') {
        Some((base, name)) if !name.is_empty() => format!("{base}/attr/{name}"),
        _ => trimmed.to_string(),
    }
}

/// Value FQN: `https://<namespace>/attr/<name>/value/<value>`.
pub fn canonical_value_fqn(definition_fqn: &str, value: &str) -> String {
    let def = canonical_definition_fqn(definition_fqn);
    if def.contains("/value/") {
        return def;
    }
    format!("{def}/value/{value}")
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
    /// The hierarchical clearance definition, if the map declares one.
    clearance: Option<ClearanceDefinition>,
    /// Conjunctive (allOf / anyOf) definition FQNs. Joined by union at wrap.
    conjunctive_fqns: BTreeSet<String>,
    /// FQNs that may only be stamped from asserted provenance, never inferred.
    provenance_fqns: BTreeSet<String>,
    /// When true, a missing provenance facet stamps `unknown` rather than omitting.
    missing_provenance_unknown: bool,
    /// Data-derived definitions. Keyed by canonical definition FQN.
    derived: BTreeMap<String, crate::derived_stamp::DerivedDefinition>,
    /// Definitions that organize the corpus and must not appear on a TDF.
    off_tdf_fqns: BTreeSet<String>,
}

/// The hierarchical clearance attribute.
///
/// Kept separately from the labels because it answers a question the labels
/// cannot: what a payload requires when the detector found no category at all.
/// Sensitivity is always known — an ingestion floor guarantees it — so without
/// this a floor-only payload carries no requirement and satisfies any subject
/// vacuously.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearanceDefinition {
    pub fqn: String,
    /// Levels from least to most privileged, matching `SensitivityLevel` order.
    pub order: Vec<String>,
}

impl ClearanceDefinition {
    /// The clearance value a subject needs to receive data at `level`.
    ///
    /// Lookup is by value name, not by index: the platform creates hierarchy
    /// values highest-to-lowest, and an older map may have listed them the
    /// other way. Indexing would silently swap public and restricted.
    pub fn value_for(&self, level: SensitivityLevel) -> Option<&str> {
        let slug = match level {
            SensitivityLevel::Public => "public",
            SensitivityLevel::Internal => "internal",
            SensitivityLevel::Confidential => "confidential",
            SensitivityLevel::Restricted => "restricted",
        };
        self.order
            .iter()
            .find(|v| v.as_str() == slug)
            .map(String::as_str)
    }
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
                TaxonomyError::UnknownSensitivity(label.name.clone(), label.sensitivity.clone())
            })?;
            if labels.contains_key(&category) {
                // Last-wins would let a second entry drop `neverRelease` and
                // make credentials wrappable. A map that contradicts itself has
                // no safe reading.
                return Err(TaxonomyError::DuplicateCategory(label.category));
            }
            labels.insert(
                category,
                LabelPolicy {
                    label: label.name,
                    category,
                    sensitivity,
                    requires: label
                        .requires
                        .into_iter()
                        .map(|r| AttributeRequirement::new(r.fqn, r.value))
                        .collect(),
                    wrap_attributes: label
                        .wrap_attributes
                        .into_iter()
                        .map(|r| AttributeRequirement::new(r.fqn, r.value))
                        .collect(),
                    unentitled: label.unentitled,
                    never_release: label.never_release,
                },
            );
        }

        let mut clearance = None;
        let mut conjunctive_fqns = BTreeSet::new();
        let mut off_tdf_fqns = BTreeSet::new();
        for def in &doc.attribute_definitions {
            let fqn = canonical_definition_fqn(&def.fqn);
            if !def.on_tdf {
                off_tdf_fqns.insert(fqn.clone());
            }
            match def.rule.as_str() {
                "hierarchy" if !def.order.is_empty() && clearance.is_none() => {
                    clearance = Some(ClearanceDefinition {
                        fqn,
                        order: def.order.clone(),
                    });
                }
                "allOf" | "anyOf" => {
                    conjunctive_fqns.insert(fqn);
                }
                _ => {}
            }
        }
        let provenance_fqns: BTreeSet<String> = doc
            .derived_artifact_join
            .provenance_attributes
            .into_iter()
            .filter(|fqn| !fqn.is_empty())
            .map(|fqn| canonical_definition_fqn(&fqn))
            .collect();
        let missing_provenance_unknown = doc.derived_artifact_join.missing_provenance == "unknown"
            && !provenance_fqns.is_empty();
        let derived = crate::derived_stamp::parse_derived_table(&doc.derived)?;

        Ok(Self {
            version: doc.version,
            namespace: doc.namespace,
            labels,
            clearance,
            conjunctive_fqns,
            provenance_fqns,
            missing_provenance_unknown,
            derived,
            off_tdf_fqns,
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

    pub fn clearance(&self) -> Option<&ClearanceDefinition> {
        self.clearance.as_ref()
    }

    /// Conjunctive attribute definitions. A derived artifact unions these.
    pub fn conjunctive_fqns(&self) -> &BTreeSet<String> {
        &self.conjunctive_fqns
    }

    /// Provenance-only FQNs. The sentinel never writes these.
    pub fn provenance_fqns(&self) -> &BTreeSet<String> {
        &self.provenance_fqns
    }

    /// Whether a missed provenance join stamps `unknown` (fail closed).
    pub fn missing_provenance_is_unknown(&self) -> bool {
        self.missing_provenance_unknown
    }

    pub fn derived_definition(
        &self,
        fqn: &str,
    ) -> Option<&crate::derived_stamp::DerivedDefinition> {
        self.derived.get(&canonical_definition_fqn(fqn))
    }

    pub fn derived_definitions(
        &self,
    ) -> &BTreeMap<String, crate::derived_stamp::DerivedDefinition> {
        &self.derived
    }

    /// Corpus-organization definitions. Used to slice adapters, never wrap.
    pub fn off_tdf_fqns(&self) -> &BTreeSet<String> {
        &self.off_tdf_fqns
    }

    /// The clearance a subject needs for data at `sensitivity`, independent of
    /// any category. Public data needs nothing.
    pub fn clearance_requirement(
        &self,
        sensitivity: SensitivityLevel,
    ) -> Option<AttributeRequirement> {
        if sensitivity <= SensitivityLevel::Public {
            return None;
        }
        let clearance = self.clearance.as_ref()?;
        clearance
            .value_for(sensitivity)
            .map(|value| AttributeRequirement::new(&clearance.fqn, value))
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
        #[serde(default, rename = "attributeDefinitions")]
        pub(super) attribute_definitions: Vec<AttributeDefinition>,
        pub(super) labels: Vec<Label>,
        #[serde(default, rename = "derivedArtifactJoin")]
        pub(super) derived_artifact_join: DerivedArtifactJoin,
        #[serde(default)]
        pub(super) derived: serde_json::Value,
    }

    #[derive(Default, Deserialize)]
    pub(super) struct DerivedArtifactJoin {
        #[serde(default, rename = "provenanceAttributes")]
        pub(super) provenance_attributes: Vec<String>,
        #[serde(default, rename = "missingProvenance")]
        pub(super) missing_provenance: String,
    }

    #[derive(Deserialize)]
    pub(super) struct AttributeDefinition {
        pub(super) fqn: String,
        pub(super) rule: String,
        #[serde(default)]
        pub(super) order: Vec<String>,
        #[serde(default = "default_on_tdf", rename = "onTdf")]
        pub(super) on_tdf: bool,
    }

    fn default_on_tdf() -> bool {
        true
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
    fn wrap_emits_platform_value_fqns() {
        // Shorthand inside our PDP is rewritten; opentdf-rs parses the value form.
        let req = AttributeRequirement::new("https://attr.arkavo.com/clearance", "secret");
        assert_eq!(req.fqn, "https://attr.arkavo.com/attr/clearance");
        assert_eq!(
            req.as_attribute_uri(),
            "https://attr.arkavo.com/attr/clearance/value/secret"
        );
        assert_eq!(
            canonical_value_fqn("https://attr.arkavo.com/attr/project", "teva-allergan"),
            "https://attr.arkavo.com/attr/project/value/teva-allergan"
        );
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
            "https://attr.arkavo.com/attr/clearance",
            "confidential"
        )));
        assert!(reqs.contains(&AttributeRequirement::new(
            "https://attr.arkavo.com/attr/department",
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
            "https://attr.arkavo.com/attr/clearance",
            "restricted"
        )));
        assert!(reqs.contains(&AttributeRequirement::new(
            "https://attr.arkavo.com/attr/department",
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
    fn a_duplicate_category_is_rejected() {
        // Last-wins would let the second entry drop neverRelease.
        let json = r#"{"version":"9","namespace":"https://x/","labels":[
            {"label":"a","category":"Credentials","sensitivity":"Restricted","unentitled":"block","neverRelease":true},
            {"label":"b","category":"Credentials","sensitivity":"Public","unentitled":"redact"}]}"#;

        assert!(matches!(
            TaxonomyMap::from_json(json).unwrap_err(),
            TaxonomyError::DuplicateCategory(_)
        ));
    }

    #[test]
    fn an_unknown_sensitivity_reports_itself_as_one() {
        let json = r#"{"version":"9","namespace":"https://x/","labels":[
            {"label":"a","category":"Pii","sensitivity":"Nonsense","unentitled":"block"}]}"#;

        assert!(matches!(
            TaxonomyMap::from_json(json).unwrap_err(),
            TaxonomyError::UnknownSensitivity(_, _)
        ));
    }

    #[test]
    fn sensitivity_alone_yields_a_clearance_requirement() {
        // A payload the detector found no category in still has a floor, and a
        // floor with no requirement satisfies every subject vacuously.
        let map = TaxonomyMap::v1();

        let internal = map
            .clearance_requirement(SensitivityLevel::Internal)
            .expect("internal needs clearance");

        assert_eq!(internal.fqn, "https://attr.arkavo.com/attr/clearance");
        assert_eq!(internal.value, "internal");
        assert_eq!(
            internal.as_attribute_uri(),
            "https://attr.arkavo.com/attr/clearance/value/internal"
        );
        assert_eq!(
            map.clearance_requirement(SensitivityLevel::Restricted)
                .map(|r| r.value),
            Some("restricted".to_string())
        );
        assert!(
            map.clearance_requirement(SensitivityLevel::Public)
                .is_none()
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

    #[test]
    fn embedded_v1_does_not_bind_project_as_provenance() {
        let map = TaxonomyMap::v1();

        assert!(map.provenance_fqns().is_empty());
        assert!(!map.missing_provenance_is_unknown());
        assert!(
            map.conjunctive_fqns()
                .contains("https://attr.arkavo.com/attr/project")
        );
    }

    #[test]
    fn oida_map_binds_project_as_fail_closed_provenance() {
        let map =
            TaxonomyMap::from_json(include_str!("../../../schemas/taxonomy-map.oida.v1.json"))
                .expect("oida map");

        assert_eq!(map.version(), "1.1.0");
        assert!(
            map.provenance_fqns()
                .contains("https://attr.arkavo.com/attr/project")
        );
        assert!(map.missing_provenance_is_unknown());
        assert_eq!(map.labels.len(), 6);
        let topic = map
            .derived_definition("https://derived.arkavo.com/attr/topic")
            .expect("topic table");
        assert!(topic.stamp);
        assert!(topic.values.contains("sales"));
        assert_eq!(topic.threshold_millis, 850);
    }
}
