//! Turning a verified pack into a running cascade (KP-003, SENT-004).
//!
//! This is where the pack stops being a file format. Everything the sentinel
//! needs that an operator must not be free to choose — the calibrated
//! thresholds, the taxonomy version they were fitted against, the reference
//! indices — comes out of a manifest whose signature was checked first.
//!
//! Order, again, is the point: `verify_pack` has already checked the signature
//! and every present digest by the time a `VerifiedPack` exists, so this
//! function cannot be reached with unverified content. The key request happens
//! here, after both.

use std::sync::Arc;

use arkavo_fingerprint::{
    IndexKey, NearDuplicateIndex, NearDuplicateTier, ReferenceIndex, ReferenceTier,
};
use arkavo_gguf_tdf::{Classification, ComponentRole, PayloadKeyUnwrapper};
use arkavo_protocol::RegexInferencer;
use arkavo_protocol::data_classification::SensitivityLevel;
use arkavo_sentinel::{CalibrationTable, Cascade, CascadeTier, PatternTier};
use serde::{Deserialize, Serialize};

use crate::blob::{SealedBlob, open_blob};
use crate::verify::VerifiedPack;

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("the pack manifest carries no calibrated thresholds")]
    NoThresholds,
    #[error("the manifest's thresholds are unusable: {0}")]
    BadThresholds(String),
    #[error(
        "the pack was calibrated against taxonomy {manifest} but the thresholds claim {thresholds}"
    )]
    TaxonomyMismatch {
        manifest: String,
        thresholds: String,
    },
    #[error("cannot read component {0}")]
    Read(String),
    #[error("the index component is unusable: {0}")]
    BadIndex(String),
    #[error(
        "component {component} is recorded at {ceiling} but was wrapped under a weaker policy; \
         refusing before a key request"
    )]
    PolicyTooWeak { component: String, ceiling: String },
    #[error(transparent)]
    Key(#[from] arkavo_gguf_tdf::GgufTdfError),
}

/// The index component's plaintext: both tiers, built from one corpus under one
/// tenant key, so they cannot drift apart.
#[derive(Debug, Serialize, Deserialize)]
pub struct PackIndexes {
    pub reference: ReferenceIndex,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub near: Option<NearDuplicateIndex>,
}

/// A pack that is ready to classify.
pub struct LoadedPack {
    pub cascade: Arc<Cascade>,
    pub calibration: CalibrationTable,
    /// The ceiling anything served from this pack carries.
    pub ceiling: Classification,
    /// What this node holds, for the audit record.
    pub inventory: String,
}

/// Build the cascade a verified pack describes.
///
/// The pattern tier is always present — it needs no provisioning and a node
/// with no index still has to catch a credential. Reference tiers are added
/// only when the pack actually carries an index this node can open; an index it
/// cannot open is an absence, not a gap on every span.
pub fn load_pack(
    pack: &VerifiedPack,
    index_key: Option<&Arc<IndexKey>>,
    unwrapper: &dyn PayloadKeyUnwrapper,
) -> Result<LoadedPack, LoadError> {
    let calibration = calibration_from(pack)?;

    let mut cascade = Cascade::new(&pack.manifest.taxonomy_version)
        .with_tier(Arc::new(PatternTier::new(Arc::new(RegexInferencer::new()))));

    if let Some(indexes) = open_indexes(pack, unwrapper)? {
        match index_key {
            Some(key) => {
                let reference = Arc::new(indexes.reference);
                cascade = cascade
                    .with_tier(Arc::new(ReferenceTier::loaded(reference, key.clone()))
                        as Arc<dyn CascadeTier>);
                if let Some(near) = indexes.near {
                    cascade = cascade.with_tier(Arc::new(NearDuplicateTier::loaded(
                        Arc::new(near),
                        key.clone(),
                    )) as Arc<dyn CascadeTier>);
                }
            }
            // The index opened but there is no tenant key to read it with. That
            // is a provisioning gap, and it is reported rather than papered
            // over with an unkeyed lookup, which cannot exist by design.
            None => {
                tracing::warn!(
                    "pack carries a reference index but no tenant key was provisioned; \
                     the cascade runs without it"
                );
            }
        }
    }

    Ok(LoadedPack {
        cascade: Arc::new(cascade),
        calibration,
        ceiling: pack.manifest.ceiling(),
        inventory: pack.inventory(),
    })
}

/// SENT-004: thresholds come from the verified manifest, and the pairing with
/// the taxonomy version is checked here rather than trusted.
fn sensitivity_of(ceiling: Classification) -> SensitivityLevel {
    match ceiling {
        Classification::Public => SensitivityLevel::Public,
        Classification::Internal => SensitivityLevel::Internal,
        Classification::Confidential => SensitivityLevel::Confidential,
        Classification::Restricted => SensitivityLevel::Restricted,
    }
}

/// KP-003: does this component's own policy demand at least the clearance its
/// recorded ceiling implies?
///
/// At least, not exactly. Clearance is hierarchical, so a component wrapped
/// under a *higher* clearance than it claims is over-protected — harmless, and
/// refusing it would reject a legitimate pack. The dangerous direction is the
/// other one: a component recorded as Confidential but wrapped under Internal
/// is one that anybody cleared for Internal can open, and no KAS will catch
/// that because the KAS is faithfully enforcing the weaker policy it was given.
fn policy_covers_ceiling(
    blob: &SealedBlob,
    ceiling: Classification,
) -> Result<bool, arkavo_gguf_tdf::GgufTdfError> {
    let needed = sensitivity_of(ceiling);
    if needed <= SensitivityLevel::Public {
        return Ok(true);
    }
    let map = arkavo_protocol::taxonomy::TaxonomyMap::v1();
    let Some(clearance) = map.clearance() else {
        return Ok(true);
    };
    let found = crate::blob::embedded_attributes(&blob.manifest)?;
    // The strongest clearance the policy actually demands.
    let strongest = [
        SensitivityLevel::Restricted,
        SensitivityLevel::Confidential,
        SensitivityLevel::Internal,
    ]
    .into_iter()
    .find(|level| {
        clearance.value_for(*level).is_some_and(|value| {
            found
                .iter()
                .any(|attribute| attribute == &format!("{}/{}", clearance.fqn, value))
        })
    });
    Ok(strongest.is_some_and(|level| level >= needed))
}

fn calibration_from(pack: &VerifiedPack) -> Result<CalibrationTable, LoadError> {
    if pack.manifest.thresholds.is_null() {
        return Err(LoadError::NoThresholds);
    }
    let table: CalibrationTable = serde_json::from_value(pack.manifest.thresholds.clone())
        .map_err(|e| LoadError::BadThresholds(e.to_string()))?;
    if !table.accepts_taxonomy(&pack.manifest.taxonomy_version) {
        return Err(LoadError::TaxonomyMismatch {
            manifest: pack.manifest.taxonomy_version.clone(),
            thresholds: table.taxonomy_version,
        });
    }
    Ok(table)
}

/// Open the index component, if this node holds one.
fn open_indexes(
    pack: &VerifiedPack,
    unwrapper: &dyn PayloadKeyUnwrapper,
) -> Result<Option<PackIndexes>, LoadError> {
    let Some(record) = pack.manifest.role(&ComponentRole::Index) else {
        return Ok(None);
    };
    if !pack.holds(&record.file) {
        // KP-005: an egress node legitimately holds some components and not
        // others, and a component it was never sent is not a failure.
        tracing::info!(component = %record.file, "pack index is not held on this node");
        return Ok(None);
    }
    let bytes =
        std::fs::read(pack.path(&record.file)).map_err(|_| LoadError::Read(record.file.clone()))?;
    let blob: SealedBlob = serde_json::from_slice(&bytes)
        .map_err(|e| LoadError::BadIndex(format!("index envelope: {e}")))?;
    // KP-003: the component's own policy is checked before any key request. An
    // index labelled Confidential that was wrapped under nothing is not an
    // index this node should be asking a KAS about — and the KAS would not
    // catch it, because it would be faithfully enforcing the weaker policy.
    if !policy_covers_ceiling(&blob, record.effective_ceiling())? {
        return Err(LoadError::PolicyTooWeak {
            component: record.file.clone(),
            ceiling: record.effective_ceiling().as_str().to_string(),
        });
    }
    let plaintext = open_blob(&blob, unwrapper)?;
    let indexes: PackIndexes = serde_json::from_slice(&plaintext)
        .map_err(|e| LoadError::BadIndex(format!("index contents: {e}")))?;
    Ok(Some(indexes))
}
