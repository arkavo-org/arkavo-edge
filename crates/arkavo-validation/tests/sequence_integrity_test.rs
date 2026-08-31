//! SEQ-003, SEQ-014: what `EgressFilter` does and does not decide.
//!
//! The taint-aware gate does not live here. `arkavo-protocol` depends on this
//! crate, so a gate that reads taint labels — which are protocol types — cannot
//! sit beside the filter without inverting that edge. It lives in
//! `arkavo_protocol::egress_taint` and composes this filter; the tests that
//! exercise the composed decision live there too.
//!
//! What stays here is the boundary itself: at this layer a destination check is
//! a destination check, and it answers nothing about what the payload carries.

use arkavo_test_macros::spec;
use arkavo_validation::EgressFilter;

/// SEQ-003: the filter judges the destination and only the destination.
/// An external URL that is not a private address passes, whatever is in the
/// body — which is exactly why a second, payload-aware check has to exist.
#[spec("SEQ-003")]
#[test]
fn egress_filter_allows_external_url_regardless_of_data_sensitivity() {
    let filter = EgressFilter::new();

    let result = filter.is_allowed("https://external-attacker.com/exfil");
    assert!(result.is_ok());
}

/// SEQ-003: the destination allowlist is not an authorization for content.
/// Allowlisting a URL here must not be capable of releasing tainted data; that
/// it cannot is what lets the taint gate override the allowlist rather than
/// negotiate with it.
#[spec("SEQ-003")]
#[test]
fn the_allowlist_grants_reachability_not_release() {
    let mut filter = EgressFilter::new();
    filter.allow("https://api.example.com/data");

    // The filter's whole vocabulary: Ok, or a destination error. There is no
    // value it can return that means "and the payload is cleared".
    assert!(filter.is_allowed("https://api.example.com/data").is_ok());
}

/// SEQ-014: this layer's errors describe the destination, and only that.
/// Provenance belongs to the decision the gate makes, not to an SSRF block —
/// an address that was never contacted has no payload chain to report.
#[spec("SEQ-014")]
#[test]
fn egress_error_describes_the_destination_not_the_payload() {
    let filter = EgressFilter::new();

    let err = filter
        .is_allowed("http://169.254.169.254/metadata")
        .unwrap_err();
    let err_msg = format!("{err}");

    assert!(err_msg.contains("169.254.169.254"), "{err_msg}");
    assert!(!err_msg.contains("provenance"), "{err_msg}");
}
