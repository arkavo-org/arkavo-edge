//! SEQ-003, SEQ-014: Tests that EgressFilter should consider data classification.

use arkavo_test_macros::spec;
use arkavo_validation::EgressFilter;

/// SEQ-003: EgressFilter allows any external URL that isn't a private IP.
/// Sequence integrity requires blocking when payload carries classified data.
#[spec("SEQ-003")]
#[test]
fn egress_filter_allows_external_url_regardless_of_data_sensitivity() {
    let filter = EgressFilter::new();

    let result = filter.is_allowed("https://external-attacker.com/exfil");
    assert!(result.is_ok());
    // SEQ-003: should be blockable when payload carries credential data.
    // Currently no API to pass data classification context.
}

/// SEQ-003: A request to a public API carrying credential data should be blocked.
#[spec("SEQ-003")]
#[test]
fn egress_filter_has_no_taint_aware_evaluation() {
    let filter = EgressFilter::new();

    let public_url_result = filter.is_allowed("https://api.example.com/data");
    assert!(
        public_url_result.is_err(),
        "SEQ-003: external URL should be blockable when carrying credential data, \
         but EgressFilter has no data classification awareness"
    );
}

/// SEQ-014: EgressFilter error type has no provenance information.
#[spec("SEQ-014")]
#[test]
fn egress_error_contains_no_provenance_chain() {
    let filter = EgressFilter::new();

    let err = filter
        .is_allowed("http://169.254.169.254/metadata")
        .unwrap_err();
    let err_msg = format!("{err}");

    assert!(
        err_msg.contains("provenance"),
        "SEQ-014: EgressError should include data provenance chain, \
         but current error is: {err_msg}"
    );
}
