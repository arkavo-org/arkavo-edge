//! Lattice join of input policies onto a derived artifact (KP-006).
//!
//! A fine-tuned adapter is a lossy copy of its training corpus. Whatever
//! entitlement is required to read the corpus must be required to unwrap the
//! weights, or the weights become the side channel. The rule is not
//! "collection is an attribute"; it is the join at wrap:
//!
//! - hierarchical definitions take the **maximum** (existing high-water mark);
//! - conjunctive definitions take the **union**;
//! - provenance attributes are stamped only from asserted facets.
//!
//! A tenant that has not bound a provenance facet simply never asserts it, and
//! the attribute is omitted. That is opt-out, not a second code path.

use std::collections::{BTreeMap, BTreeSet};

use crate::data_classification::SensitivityLevel;
use crate::taxonomy::{AttributeRequirement, TaxonomyMap};

/// Value stamped when a provenance join misses. Entitled to nobody by default.
pub const UNKNOWN_VALUE: &str = "unknown";

/// The policy a document or a derived artifact carries at wrap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicySet {
    pub clearance: SensitivityLevel,
    /// Conjunctive attribute FQN → values. Hierarchical clearance lives in
    /// `clearance` so a join cannot union `internal` with `restricted` and
    /// demand both.
    pub attributes: BTreeMap<String, BTreeSet<String>>,
}

impl PolicySet {
    /// Policy for one corpus item from a sensitivity floor and asserted facets.
    ///
    /// `facets` are (fqn, value) pairs the connector vouches for. The sentinel
    /// does not belong in this list: provenance attributes ignore inferred
    /// labels by construction.
    pub fn from_asserted(
        map: &TaxonomyMap,
        clearance: SensitivityLevel,
        facets: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        let mut attributes: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for (fqn, value) in facets {
            seen.insert(fqn.clone());
            let stamped = stamp_value(map, &fqn, Some(value.as_str()));
            if let Some(v) = stamped {
                attributes.entry(fqn).or_default().insert(v);
            }
        }
        for fqn in map.provenance_fqns() {
            if !seen.contains(fqn)
                && let Some(v) = stamp_value(map, fqn, None)
            {
                attributes.entry(fqn.clone()).or_default().insert(v);
            }
        }
        PolicySet {
            clearance,
            attributes,
        }
    }

    /// Join of every input. Hierarchical: max. Conjunctive: union.
    pub fn join<'a>(map: &TaxonomyMap, inputs: impl IntoIterator<Item = &'a PolicySet>) -> Self {
        let mut out = PolicySet {
            clearance: SensitivityLevel::Public,
            attributes: BTreeMap::new(),
        };
        let mut any = false;
        for input in inputs {
            any = true;
            out.clearance = out.clearance.max(input.clearance);
            for (fqn, values) in &input.attributes {
                out.attributes
                    .entry(fqn.clone())
                    .or_default()
                    .extend(values.iter().cloned());
            }
        }
        if !any {
            // An empty corpus is not public: nobody said, and nobody saying is
            // not permission. Conservative floor matches SEQ-001 ambiguity.
            out.clearance = SensitivityLevel::Restricted;
        }
        let _ = map;
        out
    }

    /// Whether any provenance attribute is `unknown`. Those items never enter a
    /// partitioned adapter: wrapping them would demand a value no session holds.
    pub fn blocks_partitioned_adapter(&self, map: &TaxonomyMap) -> bool {
        map.provenance_fqns().iter().any(|fqn| {
            self.attributes
                .get(fqn)
                .is_some_and(|values| values.contains(UNKNOWN_VALUE))
        })
    }

    /// OpenTDF `dataAttributes` for wrap, including clearance.
    pub fn wrap_attributes(&self, map: &TaxonomyMap) -> BTreeSet<AttributeRequirement> {
        let mut out = BTreeSet::new();
        if let Some(req) = map.clearance_requirement(self.clearance) {
            out.insert(req);
        } else if self.clearance == SensitivityLevel::Public {
            out.insert(AttributeRequirement::new(
                "https://attr.arkavo.com/attr/clearance",
                "public",
            ));
        }
        for (fqn, values) in &self.attributes {
            if map.off_tdf_fqns().contains(fqn) {
                continue;
            }
            for value in values {
                out.insert(AttributeRequirement::new(fqn, value));
            }
        }
        out
    }

    /// URIs the blob wrapper stamps (`https://attr…/project/teva`).
    pub fn wrap_uris(&self, map: &TaxonomyMap) -> Vec<String> {
        self.wrap_attributes(map)
            .into_iter()
            .map(|req| req.as_attribute_uri())
            .collect()
    }
}

fn stamp_value(map: &TaxonomyMap, fqn: &str, asserted: Option<&str>) -> Option<String> {
    let trimmed = asserted.map(str::trim).filter(|v| !v.is_empty());
    if let Some(value) = trimmed {
        return Some(slug(value));
    }
    if map.provenance_fqns().contains(fqn) && map.missing_provenance_is_unknown() {
        return Some(UNKNOWN_VALUE.to_string());
    }
    None
}

fn slug(value: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_matches('-').to_string();
    if out.is_empty() {
        UNKNOWN_VALUE.to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::taxonomy::TaxonomyMap;
    use arkavo_test_macros::spec;

    const OIDA: &str = include_str!("../../../schemas/taxonomy-map.oida.v1.json");
    const PROJECT: &str = "https://attr.arkavo.com/attr/project";

    fn oida() -> TaxonomyMap {
        TaxonomyMap::from_json(OIDA).expect("oida map")
    }

    fn asserted(map: &TaxonomyMap, level: SensitivityLevel, project: Option<&str>) -> PolicySet {
        let facets = project
            .map(|p| vec![(PROJECT.to_string(), p.to_string())])
            .unwrap_or_default();
        PolicySet::from_asserted(map, level, facets)
    }

    /// KP-006: hierarchical attributes take the maximum of the corpus.
    #[spec("KP-006")]
    #[test]
    fn join_takes_the_clearance_high_water_mark() {
        let map = oida();
        let a = asserted(&map, SensitivityLevel::Internal, Some("teva"));
        let b = asserted(&map, SensitivityLevel::Confidential, Some("endo"));

        let joined = PolicySet::join(&map, [&a, &b]);

        assert_eq!(joined.clearance, SensitivityLevel::Confidential);
    }

    /// KP-006: conjunctive attributes take the union, so an archive-wide
    /// adapter demands every collection it saw rather than an umbrella value.
    #[spec("KP-006")]
    #[test]
    fn join_unions_conjunctive_project_values() {
        let map = oida();
        let a = asserted(&map, SensitivityLevel::Confidential, Some("teva"));
        let b = asserted(&map, SensitivityLevel::Confidential, Some("endo"));

        let joined = PolicySet::join(&map, [&a, &b]);
        let projects = joined.attributes.get(PROJECT).expect("project");

        assert_eq!(projects, &BTreeSet::from(["endo".into(), "teva".into()]));
        let uris = joined.wrap_uris(&map);
        assert!(
            uris.iter()
                .any(|u| u.ends_with("/attr/clearance/value/confidential"))
        );
        assert!(
            uris.iter().all(|u| !u.contains("/attr/project/")),
            "corpus organization must not be written to the TDF"
        );
    }

    #[test]
    fn project_stays_off_the_tdf() {
        let map = oida();
        assert!(map.off_tdf_fqns().contains(PROJECT));
        let item = asserted(&map, SensitivityLevel::Confidential, Some("mnk"));
        assert!(
            item.wrap_uris(&map)
                .iter()
                .all(|u| !u.contains("/attr/project/"))
        );
    }

    #[test]
    fn embedded_v1_omits_project_when_the_facet_is_not_asserted() {
        // Opt-out is "do not assert", not a second join implementation.
        let map = TaxonomyMap::v1();
        let item = PolicySet::from_asserted(map, SensitivityLevel::Confidential, []);

        assert!(item.attributes.get(PROJECT).is_none());
        assert!(!item.blocks_partitioned_adapter(map));
        let uris = item.wrap_uris(map);
        assert!(uris.iter().all(|u| !u.contains("/attr/project/")));
    }

    #[test]
    fn v1_still_stamps_project_when_a_connector_asserts_it() {
        let map = TaxonomyMap::v1();
        let item = PolicySet::from_asserted(
            map,
            SensitivityLevel::Confidential,
            [(PROJECT.to_string(), "matter-17".to_string())],
        );

        assert_eq!(
            item.attributes.get(PROJECT),
            Some(&BTreeSet::from(["matter-17".into()]))
        );
        assert!(
            item.wrap_uris(map)
                .iter()
                .all(|u| !u.contains("/attr/project/"))
        );
    }

    #[test]
    fn collection_names_slug_to_attribute_values() {
        let map = oida();
        let item = asserted(&map, SensitivityLevel::Confidential, Some("Endo Documents"));

        assert_eq!(
            item.attributes.get(PROJECT),
            Some(&BTreeSet::from(["endo-documents".into()]))
        );
    }

    #[test]
    fn an_empty_join_is_restricted_not_public() {
        let map = oida();
        let joined = PolicySet::join(&map, []);

        assert_eq!(joined.clearance, SensitivityLevel::Restricted);
    }

    #[test]
    fn sentinel_labels_cannot_smuggle_a_project_value() {
        // from_asserted is the only constructor that stamps provenance. A
        // caller that has only sentinel labels passes no project facet.
        let map = oida();
        let from_labels_only = PolicySet::from_asserted(&map, SensitivityLevel::Confidential, []);

        assert_eq!(
            from_labels_only
                .attributes
                .get(PROJECT)
                .and_then(|v| v.iter().next())
                .map(String::as_str),
            Some(UNKNOWN_VALUE)
        );
    }
}
