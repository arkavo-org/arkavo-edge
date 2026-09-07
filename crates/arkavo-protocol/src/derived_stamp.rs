//! Wrap-time exits for data-derived tags.
//!
//! The prescribed map is still the only place a label becomes a decrypt
//! attribute. Derived tags have three exits, and the `derived` table chooses:
//!
//! 1. **Assertion — always.** The full vector is signed with the tagger's key,
//!    not HMAC-bound to the DEK, so retrieval can verify provenance without
//!    holding payload key material.
//! 2. **Data attribute — only declared values above threshold.** Separate
//!    namespace (`https://derived.arkavo.com/attr/…`). ALL_OF, no KAS grants.
//!    A universally-entitled derived value is a no-op conjunct, not a silent
//!    allow. Promotion tightens the subject mapping; wrap still stamps it.
//! 3. **Nothing** — below threshold. The tag stays in the assertion so a
//!    false negative does not drop below the prescribed baseline.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use crate::taxonomy::{
    AttributeRequirement, TaxonomyError, TaxonomyMap, canonical_definition_fqn, canonical_value_fqn,
};

/// Namespace that makes model-emitted definitions visibly distinct.
pub const DERIVED_NAMESPACE: &str = "https://derived.arkavo.com";
/// Scores are stored as thousandths so a signed assertion never contains a float.
pub const SCORE_SCALE: u16 = 1000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedDefinition {
    pub fqn: String,
    pub values: BTreeSet<String>,
    pub tagger: String,
    pub tagger_version: String,
    pub threshold_millis: u16,
    pub stamp: bool,
    pub promoted: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DerivedTag {
    pub definition: String,
    pub value: String,
    /// 0..=1000. 850 is the 0.85 threshold in the OIDA topic table.
    pub score_millis: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DerivedAssertion {
    pub map_version: String,
    pub tagger: String,
    pub tagger_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cluster_id: Option<String>,
    pub tags: Vec<DerivedTag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedDerivedAssertion {
    pub assertion: DerivedAssertion,
    pub signature: String,
    pub verifying_key: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DerivedExit {
    Stamp,
    AssertOnly,
    Drop,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedWrap {
    pub stamped: BTreeSet<AttributeRequirement>,
    pub assertion: DerivedAssertion,
    pub dropped: Vec<DerivedTag>,
}

pub fn parse_derived_table(
    value: &serde_json::Value,
) -> Result<BTreeMap<String, DerivedDefinition>, TaxonomyError> {
    if value.is_null() {
        return Ok(BTreeMap::new());
    }
    let obj = value.as_object().ok_or_else(|| {
        TaxonomyError::Malformed("derived table must be an object keyed by definition FQN".into())
    })?;
    let mut out = BTreeMap::new();
    for (fqn, body) in obj {
        let def = parse_one(fqn, body)?;
        out.insert(def.fqn.clone(), def);
    }
    Ok(out)
}

fn parse_one(fqn: &str, body: &serde_json::Value) -> Result<DerivedDefinition, TaxonomyError> {
    let canonical = canonical_definition_fqn(fqn);
    if canonical.starts_with("https://attr.arkavo.com/") {
        return Err(TaxonomyError::DerivedInPrescribedNamespace(canonical));
    }
    if !canonical.contains("/attr/") {
        return Err(TaxonomyError::Malformed(format!(
            "derived definition '{fqn}' is not an /attr/ FQN"
        )));
    }
    let rule = body
        .get("rule")
        .and_then(|v| v.as_str())
        .unwrap_or_default();
    if !matches!(rule, "ALL_OF" | "allOf") {
        return Err(TaxonomyError::DerivedMustBeAllOf(canonical));
    }
    let values = body
        .get("values")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let source = body.get("source").ok_or_else(|| {
        TaxonomyError::Malformed(format!("derived '{canonical}' is missing source"))
    })?;
    let tagger = source
        .get("tagger")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let tagger_version = source
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    if tagger.is_empty() || tagger_version.is_empty() {
        return Err(TaxonomyError::Malformed(format!(
            "derived '{canonical}' source needs tagger and version"
        )));
    }
    let threshold = body
        .get("threshold")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);
    let threshold_millis = (threshold * f64::from(SCORE_SCALE)).round() as u16;
    let stamp = body.get("stamp").and_then(|v| v.as_bool()).unwrap_or(false);
    let promoted = body
        .get("promoted")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(DerivedDefinition {
        fqn: canonical,
        values,
        tagger,
        tagger_version,
        threshold_millis,
        stamp,
        promoted,
    })
}

impl DerivedTag {
    pub fn exit(&self, map: &TaxonomyMap) -> DerivedExit {
        let Some(def) = map.derived_definition(&self.definition) else {
            return DerivedExit::AssertOnly;
        };
        if !def.stamp {
            return DerivedExit::AssertOnly;
        }
        if self.score_millis < def.threshold_millis {
            return DerivedExit::Drop;
        }
        if !def.values.contains(&self.value) {
            // Open-ended tags cannot become attributes: validateAttributes
            // would reject an FQN that does not exist.
            return DerivedExit::AssertOnly;
        }
        DerivedExit::Stamp
    }
}

pub fn plan_derived_wrap(
    map: &TaxonomyMap,
    mut tags: Vec<DerivedTag>,
    cluster_id: Option<String>,
    tagger: &str,
    tagger_version: &str,
) -> DerivedWrap {
    tags.sort();
    let mut stamped = BTreeSet::new();
    let mut dropped = Vec::new();
    for tag in &tags {
        match tag.exit(map) {
            DerivedExit::Stamp => {
                stamped.insert(AttributeRequirement::new(&tag.definition, &tag.value));
            }
            DerivedExit::Drop => dropped.push(tag.clone()),
            DerivedExit::AssertOnly => {}
        }
    }
    DerivedWrap {
        stamped,
        assertion: DerivedAssertion {
            map_version: map.version().to_string(),
            tagger: tagger.to_string(),
            tagger_version: tagger_version.to_string(),
            cluster_id,
            tags,
        },
        dropped,
    }
}

/// Sign with the tagger key. Verification does not need the DEK.
pub fn sign_derived_assertion(
    assertion: &DerivedAssertion,
    key: &arkavo_crypto::AgentKeypair,
) -> Result<SignedDerivedAssertion, TaxonomyError> {
    let bytes = serde_json::to_vec(assertion)
        .map_err(|e| TaxonomyError::Malformed(format!("derived assertion: {e}")))?;
    Ok(SignedDerivedAssertion {
        assertion: assertion.clone(),
        signature: encode(&key.sign(&bytes)),
        verifying_key: encode(&key.public_key().to_bytes()),
    })
}

pub fn verify_derived_assertion(signed: &SignedDerivedAssertion) -> Result<(), TaxonomyError> {
    use arkavo_crypto::AgentPublicKey;

    let bytes = serde_json::to_vec(&signed.assertion)
        .map_err(|e| TaxonomyError::Malformed(format!("derived assertion: {e}")))?;
    let key_bytes = decode(&signed.verifying_key)?;
    let key = AgentPublicKey::from_bytes(&key_bytes)
        .map_err(|e| TaxonomyError::Malformed(format!("derived verifying key: {e}")))?;
    let signature = decode(&signed.signature)?;
    key.verify(&bytes, &signature)
        .map_err(|_| TaxonomyError::Malformed("derived assertion signature is invalid".into()))
}

pub fn stamped_uris(wrap: &DerivedWrap) -> Vec<String> {
    wrap.stamped
        .iter()
        .map(|req| canonical_value_fqn(&req.fqn, &req.value))
        .collect()
}

fn encode(bytes: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn decode(text: &str) -> Result<Vec<u8>, TaxonomyError> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .map_err(|e| TaxonomyError::Malformed(format!("derived assertion encoding: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;

    const OIDA: &str = include_str!("../../../schemas/taxonomy-map.oida.v1.json");
    const TOPIC: &str = "https://derived.arkavo.com/attr/topic";

    fn oida() -> TaxonomyMap {
        TaxonomyMap::from_json(OIDA).expect("oida map")
    }

    fn tag(value: &str, millis: u16) -> DerivedTag {
        DerivedTag {
            definition: TOPIC.into(),
            value: value.into(),
            score_millis: millis,
        }
    }

    #[test]
    fn declared_topic_above_threshold_is_stamped() {
        let map = oida();
        let wrap = plan_derived_wrap(
            &map,
            vec![tag("sales", 900)],
            None,
            "sentinel-topic",
            "0.1.0",
        );

        assert_eq!(
            stamped_uris(&wrap),
            vec!["https://derived.arkavo.com/attr/topic/value/sales".to_string()]
        );
        assert!(wrap.dropped.is_empty());
        assert_eq!(wrap.assertion.tags.len(), 1);
    }

    #[test]
    fn below_threshold_is_asserted_not_stamped() {
        let map = oida();
        let wrap = plan_derived_wrap(
            &map,
            vec![tag("sales", 800)],
            None,
            "sentinel-topic",
            "0.1.0",
        );

        assert!(wrap.stamped.is_empty());
        assert_eq!(wrap.dropped.len(), 1);
        assert_eq!(wrap.assertion.tags[0].value, "sales");
    }

    #[test]
    fn open_ended_cluster_stays_in_the_assertion() {
        let map = oida();
        let cluster = DerivedTag {
            definition: "https://derived.arkavo.com/attr/embed-cluster".into(),
            value: "3".into(),
            score_millis: 1000,
        };
        let wrap = plan_derived_wrap(
            &map,
            vec![cluster, tag("sales", 990)],
            Some("3".into()),
            "sentinel-topic",
            "0.1.0",
        );

        let uris = stamped_uris(&wrap);
        assert_eq!(uris.len(), 1);
        assert!(uris[0].ends_with("/attr/topic/value/sales"));
        assert!(!uris.iter().any(|u| u.contains("embed-cluster")));
        assert_eq!(wrap.assertion.cluster_id.as_deref(), Some("3"));
        assert_eq!(wrap.assertion.tags.len(), 2);
    }

    #[test]
    fn undeclared_topic_value_is_not_stamped() {
        let map = oida();
        let wrap = plan_derived_wrap(
            &map,
            vec![tag("clinical", 1000)],
            None,
            "sentinel-topic",
            "0.1.0",
        );

        assert!(wrap.stamped.is_empty());
        assert_eq!(wrap.assertion.tags[0].value, "clinical");
    }

    #[test]
    fn all_of_stamps_every_declared_tag_as_its_own_conjunct() {
        let map = oida();
        let wrap = plan_derived_wrap(
            &map,
            vec![tag("sales", 990), tag("pricing", 990)],
            None,
            "sentinel-topic",
            "0.1.0",
        );

        let uris = stamped_uris(&wrap);
        assert!(uris.iter().any(|u| u.ends_with("/topic/value/sales")));
        assert!(uris.iter().any(|u| u.ends_with("/topic/value/pricing")));
        assert_eq!(uris.len(), 2);
    }

    #[test]
    fn tagger_signature_verifies_without_a_dek() {
        let map = oida();
        let wrap = plan_derived_wrap(
            &map,
            vec![tag("sales", 900)],
            None,
            "sentinel-topic",
            "0.1.0",
        );
        let key = AgentKeypair::generate();
        let signed = sign_derived_assertion(&wrap.assertion, &key).expect("sign");

        assert!(verify_derived_assertion(&signed).is_ok());
        let mut tampered = signed.clone();
        tampered.assertion.tagger = "other".into();
        assert!(verify_derived_assertion(&tampered).is_err());
    }

    #[test]
    fn derived_table_rejects_a_prescribed_namespace() {
        let json = serde_json::json!({
            "https://attr.arkavo.com/attr/topic": {
                "rule": "ALL_OF",
                "values": ["sales"],
                "source": {"tagger": "x", "version": "1"},
                "threshold": 0.5,
                "stamp": true
            }
        });

        assert!(matches!(
            parse_derived_table(&json),
            Err(TaxonomyError::DerivedInPrescribedNamespace(_))
        ));
    }

    #[test]
    fn derived_table_rejects_any_of() {
        let json = serde_json::json!({
            "https://derived.arkavo.com/attr/topic": {
                "rule": "ANY_OF",
                "values": ["sales"],
                "source": {"tagger": "x", "version": "1"},
                "threshold": 0.5,
                "stamp": true
            }
        });

        assert!(matches!(
            parse_derived_table(&json),
            Err(TaxonomyError::DerivedMustBeAllOf(_))
        ));
    }
}
