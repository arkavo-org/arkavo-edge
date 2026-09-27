//! What a protected artifact is *for* (KP-001, KP-008).
//!
//! A pack is several separately wrapped components, and the runtime has to know
//! which one it is holding before it can decide anything about it. That fact is
//! recorded at wrap time and travels inside the archive, because the
//! alternative — inferring it from a file name — makes the component's identity
//! a property of whoever last renamed it.
//!
//! The metadata member is plaintext. It has to be: an egress node decides
//! whether it is even entitled to request a key by reading it, which is before
//! it could decrypt anything. Plaintext is also why it is not, on its own,
//! trustworthy — the pack manifest binds this member's digest and is signed,
//! and that signature is what makes the claim worth acting on.

use serde::{Deserialize, Serialize};

/// Plaintext zip member carrying [`ComponentMetadata`].
///
/// Numbered like the manifest so it sorts beside it, and named so a reader that
/// predates this member ignores it: the profile looks members up by name and
/// forbids only `0.payload`, so an extra member is not a format change.
pub const COMPONENT_ENTRY: &str = "0.component.json";

/// How far the content of a component may travel.
///
/// A local mirror of the four-level scale rather than a shared type: this crate
/// sits at the bottom of the dependency graph, underneath the crate that owns
/// the classification vocabulary, and the edge that would share the type runs
/// the wrong way. The mapping happens where both are visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Classification {
    Public,
    Internal,
    Confidential,
    Restricted,
}

impl Classification {
    /// The higher of two classifications. The only direction a merge may move
    /// (KP-006): a model carries the highest classification in its corpus.
    #[must_use]
    pub fn high_water(self, other: Classification) -> Classification {
        if self >= other { self } else { other }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Classification::Public => "public",
            Classification::Internal => "internal",
            Classification::Confidential => "confidential",
            Classification::Restricted => "restricted",
        }
    }
}

/// What this component is within a pack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "role", rename_all = "lowercase")]
pub enum ComponentRole {
    /// Knowledge adapter for one compartment.
    Adapter {
        /// The compartment this adapter serves. Exactly one adapter may serve a
        /// compartment (KP-001), which is checked at assembly.
        compartment: String,
    },
    /// The DLP classifier.
    Sentinel,
    /// A keyed reference index.
    Index,
    /// A base model that is not itself a pack component but carries a ceiling.
    Model,
}

impl ComponentRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            ComponentRole::Adapter { .. } => "adapter",
            ComponentRole::Sentinel => "sentinel",
            ComponentRole::Index => "index",
            ComponentRole::Model => "model",
        }
    }

    pub fn compartment(&self) -> Option<&str> {
        match self {
            ComponentRole::Adapter { compartment } => Some(compartment),
            _ => None,
        }
    }
}

/// Recorded at wrap time, read before any key request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComponentMetadata {
    #[serde(flatten)]
    pub role: ComponentRole,
    /// How far output derived from this component may travel.
    ///
    /// Optional because an artifact wrapped before this field existed has none,
    /// and absent must not read as `Public`: a component with no recorded
    /// ceiling is treated conservatively by the runtime, not permissively.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification_ceiling: Option<Classification>,
    /// Taxonomy-map version the ceiling and any attributes were derived
    /// against, so a runtime can refuse a pairing it was not calibrated for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub taxonomy_version: Option<String>,
}

impl ComponentMetadata {
    pub fn new(role: ComponentRole) -> Self {
        Self {
            role,
            classification_ceiling: None,
            taxonomy_version: None,
        }
    }

    #[must_use]
    pub fn with_ceiling(mut self, ceiling: Classification) -> Self {
        self.classification_ceiling = Some(ceiling);
        self
    }

    #[must_use]
    pub fn with_taxonomy_version(mut self, version: impl Into<String>) -> Self {
        self.taxonomy_version = Some(version.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    /// KP-006: a merge takes the maximum, in both directions.
    #[spec("KP-006")]
    #[test]
    fn a_high_water_merge_never_moves_down() {
        let low = Classification::Internal;
        let high = Classification::Restricted;

        assert_eq!(low.high_water(high), Classification::Restricted);
        assert_eq!(high.high_water(low), Classification::Restricted);
        assert_eq!(low.high_water(low), Classification::Internal);
    }

    /// KP-001: the compartment is data on the component, not a file name.
    #[spec("KP-001")]
    #[test]
    fn an_adapter_carries_its_compartment() {
        let meta = ComponentMetadata::new(ComponentRole::Adapter {
            compartment: "legal".into(),
        })
        .with_ceiling(Classification::Confidential);

        assert_eq!(meta.role.compartment(), Some("legal"));
        assert_eq!(meta.role.as_str(), "adapter");
        assert_eq!(
            meta.classification_ceiling,
            Some(Classification::Confidential)
        );
    }

    #[test]
    fn a_component_with_no_compartment_reports_none() {
        assert_eq!(ComponentRole::Sentinel.compartment(), None);
        assert_eq!(ComponentRole::Index.compartment(), None);
    }

    #[test]
    fn metadata_round_trips_through_json() {
        // This is the wire form: the plaintext member inside the archive.
        let meta = ComponentMetadata::new(ComponentRole::Adapter {
            compartment: "finance".into(),
        })
        .with_ceiling(Classification::Restricted)
        .with_taxonomy_version("1.0.0");

        let json = serde_json::to_string(&meta).expect("serialize");
        let back: ComponentMetadata = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back, meta);
    }

    /// An archive wrapped before this field existed has no ceiling, and absent
    /// must not deserialize into the most permissive value.
    #[spec("KP-008")]
    #[test]
    fn an_absent_ceiling_stays_absent_rather_than_becoming_public() {
        let meta: ComponentMetadata =
            serde_json::from_str(r#"{"role":"sentinel"}"#).expect("deserialize");

        assert_eq!(meta.classification_ceiling, None);
    }
}
