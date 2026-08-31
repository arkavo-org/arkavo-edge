//! KP-001, KP-002, KP-005, KP-008: tripwires against the `gguf-tdf/1` surface a
//! knowledge pack has to grow out of.
//!
//! A pack is several separately wrapped components bound by one signed
//! manifest. Today this crate protects a single GGUF: one payload, one
//! manifest, no signature, and no way to say which compartment the weights
//! belong to or how far their content may travel.

use arkavo_gguf_tdf::{
    DEFAULT_MAX_SEGMENT, EXTENSION, HEADER_ENTRY, MANIFEST_ENTRY, MANIFEST_ENTRY_FALLBACK, PROFILE,
    ProtectOptions, entry_name,
};
use arkavo_test_macros::spec;

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
/// Documents the layout a pack has to compose, not replace.
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

/// KP-001: an adapter is wrapped for a compartment, and the compartment has to
/// be recorded at wrap time rather than read back off a file name. Wrap options
/// carry attributes and dissemination, but nothing that names the component's
/// role in a pack.
/// Tripwire: flips when wrap options carry component role and compartment.
#[spec("KP-001")]
#[test]
#[should_panic(expected = "KP-001")]
fn wrap_options_carry_no_component_role() {
    let opts = ProtectOptions::default();
    let opts_str = format!("{opts:?}");

    assert!(
        opts_str.contains("compartment") || opts_str.contains("component"),
        "KP-001: a pack component must record its role and compartment at wrap \
         time, but wrap options are: {opts_str}"
    );
}

/// KP-002: the manifest binds the pack, and a detached signature over it is
/// what makes the binding worth anything. The profile defines a manifest member
/// and no signature member.
/// Tripwire: flips when a detached-signature member joins the profile.
#[spec("KP-002")]
#[test]
#[should_panic(expected = "KP-002")]
fn profile_defines_no_detached_signature_member() {
    let members = profile_members();

    assert!(
        members.iter().any(|m| m.ends_with(".sig")),
        "KP-002: a pack manifest must ship with a detached signature, but the \
         profile defines only: {members:?}"
    );
}

/// KP-008: output inherits the union of its input taint and the serving model's
/// classification ceiling. The ceiling has to travel with the weights, because a
/// ceiling supplied by local configuration is a ceiling the operator can lower.
/// Tripwire: flips when a classification ceiling joins component metadata.
#[spec("KP-008")]
#[test]
#[should_panic(expected = "KP-008")]
fn wrap_options_carry_no_classification_ceiling() {
    let opts = ProtectOptions::default();
    let opts_str = format!("{opts:?}");

    assert!(
        opts_str.contains("classification") || opts_str.contains("ceiling"),
        "KP-008: a protected model must carry its classification ceiling in \
         component metadata, but wrap options are: {opts_str}"
    );
}
