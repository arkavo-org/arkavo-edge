//! KP-001, KP-005, KP-008: what a pack component records about itself.
//!
//! A pack is several separately wrapped components bound by one signed
//! manifest. This crate wraps one component; the manifest that binds a set of
//! them lives in `arkavo-knowledge-pack`, which depends on this crate — so the
//! KP-002 signature tripwire moved there rather than distorting `gguf-tdf/1`
//! into carrying a member that belongs to the container above it.

mod common;

use arkavo_gguf_tdf::{
    COMPONENT_ENTRY, Classification, ComponentMetadata, ComponentRole, DEFAULT_MAX_SEGMENT,
    EXTENSION, GgufTdfArchive, GgufTdfError, HEADER_ENTRY, MANIFEST_ENTRY, MANIFEST_ENTRY_FALLBACK,
    PROFILE, PayloadKeyWrapper, ProtectOptions, WrappedKey, entry_name, protect,
};
use arkavo_test_macros::spec;
use base64::Engine as _;

/// Records the payload key instead of contacting a KAS. Unit tests must never
/// reach a production KAS.
struct MockWrapper;

impl PayloadKeyWrapper for MockWrapper {
    fn wrap(&self, payload_key: &[u8; 32]) -> Result<WrappedKey, GgufTdfError> {
        Ok(WrappedKey {
            kas_url: "https://kas.example.invalid".to_string(),
            kid: None,
            wrapped_key: base64::engine::general_purpose::STANDARD.encode(payload_key),
        })
    }
}

/// Every member name the profile defines today.
fn profile_members() -> Vec<String> {
    vec![
        HEADER_ENTRY.to_string(),
        MANIFEST_ENTRY.to_string(),
        MANIFEST_ENTRY_FALLBACK.to_string(),
        entry_name(1),
    ]
}

/// KP-005: one archive is one component — a single payload under a single key.
/// Documents the layout a pack composes rather than replaces.
#[spec("KP-005")]
#[test]
fn a_protected_artifact_is_a_single_component() {
    assert_eq!(PROFILE, "gguf-tdf/1");
    assert_eq!(EXTENSION, ".gguf.tdf");
    assert_eq!(DEFAULT_MAX_SEGMENT, 4_194_304);

    let members = profile_members();
    assert!(members.iter().any(|m| m == HEADER_ENTRY));
    assert!(members.iter().any(|m| m == MANIFEST_ENTRY));
}

/// KP-001: an adapter is wrapped *for* a compartment, and the compartment is
/// recorded at wrap time rather than read back off a file name later.
#[spec("KP-001")]
#[test]
fn wrap_options_carry_the_component_role_and_compartment() {
    let opts = ProtectOptions {
        component: Some(ComponentMetadata::new(ComponentRole::Adapter {
            compartment: "legal".to_string(),
        })),
        ..Default::default()
    };

    let component = opts.component.as_ref().expect("a component role");
    assert_eq!(component.role.as_str(), "adapter");
    assert_eq!(component.role.compartment(), Some("legal"));
}

/// KP-001: a plain protected model is not a pack component and says so, rather
/// than defaulting into a role nobody chose.
#[spec("KP-001")]
#[test]
fn a_plain_protected_model_records_no_role() {
    assert!(ProtectOptions::default().component.is_none());
}

/// KP-008: output inherits the union of its input taint and the serving model's
/// ceiling, so the ceiling has to travel with the weights — a ceiling supplied
/// by local configuration is a ceiling the operator can lower.
#[spec("KP-008")]
#[test]
fn wrap_options_carry_the_classification_ceiling() {
    let opts = ProtectOptions {
        component: Some(
            ComponentMetadata::new(ComponentRole::Model).with_ceiling(Classification::Confidential),
        ),
        ..Default::default()
    };

    assert_eq!(
        opts.component.and_then(|c| c.classification_ceiling),
        Some(Classification::Confidential)
    );
}

/// KP-008 edge case: metadata and configuration disagreeing resolves upward.
/// The merge is the same high-water rule the corpus classification uses, which
/// is why it lives on the type rather than at each call site.
#[spec("KP-008")]
#[test]
fn a_disagreement_about_the_ceiling_resolves_to_the_higher() {
    let from_metadata = Classification::Internal;
    let from_configuration = Classification::Restricted;

    assert_eq!(
        from_metadata.high_water(from_configuration),
        Classification::Restricted
    );
}

/// KP-001, KP-008: the role and the ceiling survive the wrap and are readable
/// before any key is requested. That ordering is the requirement, not a
/// convenience: an egress node decides whether it is entitled to ask for this
/// component's key by reading this, which necessarily precedes decrypting it.
#[spec("KP-008")]
#[test]
fn the_component_record_is_readable_before_the_key_is_requested() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = common::synthetic_gguf(&[("token_embd.weight", 0, [4096, 2, 1, 1])], None);
    let source = dir.path().join("model.gguf");
    std::fs::write(&source, &bytes).unwrap();
    let dest = dir.path().join("model.gguf.tdf");

    protect(
        &source,
        &dest,
        &MockWrapper,
        &ProtectOptions {
            component: Some(
                ComponentMetadata::new(ComponentRole::Adapter {
                    compartment: "legal".to_string(),
                })
                .with_ceiling(Classification::Restricted)
                .with_taxonomy_version("1.0.0"),
            ),
            ..Default::default()
        },
    )
    .expect("protect");

    // `open` performs no KAS round-trip; this is what a node reads first.
    let archive = GgufTdfArchive::open(&dest).expect("open");
    let component = archive.component().expect("a component record");

    assert_eq!(component.role.compartment(), Some("legal"));
    assert_eq!(
        component.classification_ceiling,
        Some(Classification::Restricted)
    );
    assert_eq!(component.taxonomy_version.as_deref(), Some("1.0.0"));
}

/// An archive wrapped without a component record still opens. Every artifact
/// protected before this member existed has none, and a reader that refused
/// them would have made this an incompatible format change.
#[spec("KP-005")]
#[test]
fn an_archive_without_a_component_record_still_opens() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = common::synthetic_gguf(&[("token_embd.weight", 0, [4096, 2, 1, 1])], None);
    let source = dir.path().join("model.gguf");
    std::fs::write(&source, &bytes).unwrap();
    let dest = dir.path().join("model.gguf.tdf");
    protect(&source, &dest, &MockWrapper, &ProtectOptions::default()).expect("protect");

    let archive = GgufTdfArchive::open(&dest).expect("open");

    assert!(archive.component().is_none());
    assert!(!profile_members().iter().any(|m| m == COMPONENT_ENTRY));
}
