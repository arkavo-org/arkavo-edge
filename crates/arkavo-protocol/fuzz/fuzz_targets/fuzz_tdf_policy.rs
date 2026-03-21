#![no_main]

//! Fuzz target for TDF policy parsing
//!
//! Verifies:
//! - TDF-001: Policy schema validation
//! - TDF-002: Invalid policy rejection
//! - TDF-003: Policy round-trip serialization
//! - TDF-004: Manifest integrity checks

use arkavo_tdf::{Attribute, Policy, TdfManifest};
use libfuzzer_sys::fuzz_target;

/// Fuzz input for TDF policy testing
#[derive(Debug, Clone)]
struct TdfFuzzInput {
    /// Raw JSON bytes to parse
    raw_bytes: Vec<u8>,
    /// Test type selector (0=Policy, 1=Manifest, 2=PolicyBuilder)
    test_type: u8,
}

impl<'a> arbitrary::Arbitrary<'a> for TdfFuzzInput {
    fn arbitrary(u: &mut arbitrary::Unstructured<'a>) -> arbitrary::Result<Self> {
        let test_type = u.int_in_range(0..=2)?;
        let len = u.int_in_range(0..=8192)?;
        let mut raw_bytes = vec![0u8; len];
        u.fill_buffer(&mut raw_bytes)?;
        Ok(TdfFuzzInput {
            raw_bytes,
            test_type,
        })
    }
}

fuzz_target!(|input: TdfFuzzInput| {
    // Parse as UTF-8 string
    let json_str = match std::str::from_utf8(&input.raw_bytes) {
        Ok(s) => s,
        Err(_) => return, // Invalid UTF-8 is not a valid policy
    };

    match input.test_type {
        0 => {
            // Direct Policy deserialization
            match serde_json::from_str::<Policy>(json_str) {
                Ok(policy) => {
                    // Verify invariants
                    // Policy attributes should not cause panic when accessed
                    let _ = policy.attributes.len();
                    let _ = policy.dissemination.len();

                    // Round-trip test
                    match serde_json::to_string(&policy) {
                        Ok(encoded) => {
                            let decoded: Policy =
                                serde_json::from_str(&encoded).expect("round-trip should succeed");
                            assert_eq!(policy.id, decoded.id);
                            assert_eq!(policy.attributes.len(), decoded.attributes.len());
                            assert_eq!(policy.dissemination, decoded.dissemination);
                        }
                        Err(_) => {
                            // Encoding failure is ok, just check no panic
                        }
                    }

                    // Check attribute invariants
                    for attr in &policy.attributes {
                        // Attribute FQN should be a non-empty string
                        assert!(
                            !attr.attribute.is_empty(),
                            "Attribute FQN should not be empty"
                        );
                        // Accessing values should not panic
                        let _ = attr.values.len();
                    }
                }
                Err(_) => {
                    // Invalid policies should be rejected without panic
                }
            }
        }
        1 => {
            // TDF Manifest deserialization
            match serde_json::from_str::<TdfManifest>(json_str) {
                Ok(manifest) => {
                    // Version should be a valid semantic version string
                    assert!(!manifest.version.is_empty(), "Version cannot be empty");

                    // Encryption information access should not panic
                    let _ = manifest.encryption_information.key_access.len();
                    let _ = &manifest.encryption_information.method.algorithm;
                    let _ = &manifest.encryption_information.policy;

                    // Payload access should not panic
                    let _ = &manifest.payload.value;
                    let _ = &manifest.payload.mime_type;

                    // Round-trip test
                    match serde_json::to_string(&manifest) {
                        Ok(encoded) => {
                            let decoded: TdfManifest = serde_json::from_str(&encoded)
                                .expect("manifest round-trip should succeed");
                            assert_eq!(manifest.version, decoded.version);
                            assert_eq!(
                                manifest.encryption_information.key_access.len(),
                                decoded.encryption_information.key_access.len()
                            );
                        }
                        Err(_) => {}
                    }
                }
                Err(_) => {
                    // Invalid manifests should be rejected without panic
                }
            }
        }
        2 => {
            // Test PolicyBuilder with arbitrary JSON as attributes
            if let Ok(json_value) = serde_json::from_str::<serde_json::Value>(json_str) {
                // Try to interpret JSON as policy attributes
                if let Some(obj) = json_value.as_object() {
                    use arkavo_tdf::PolicyBuilder;

                    let mut builder = PolicyBuilder::new();

                    // Try to extract attributes from JSON
                    for (key, value) in obj {
                        let values: Vec<&str> = match value {
                            serde_json::Value::Array(arr) => arr
                                .iter()
                                .filter_map(|v| v.as_str())
                                .collect(),
                            serde_json::Value::String(s) => vec![s.as_str()],
                            _ => vec![],
                        };

                        if !values.is_empty() {
                            // Try with various attribute FQN formats
                            let fqn = if key.starts_with("https://") {
                                key.clone()
                            } else {
                                format!("https://arkavo.net/attr/{key}")
                            };
                            builder = builder.attribute(&fqn, &values);
                        }
                    }

                    // Attempt to build - may fail validation but should not panic
                    let _ = builder.build();
                }
            }
        }
        _ => unreachable!(),
    }

    // Additional: Test Attribute deserialization directly
    // Empty FQN is now rejected at deserialization time
    if input.test_type == 0 {
        if let Ok(attr) = serde_json::from_str::<Attribute>(json_str) {
            assert!(
                !attr.attribute.is_empty(),
                "Serde validation should reject empty FQN"
            );
            let _ = attr.values.len();
        }
    }
});
