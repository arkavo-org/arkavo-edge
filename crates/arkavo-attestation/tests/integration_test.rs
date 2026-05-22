use arkavo_attestation::{SecurityState, detect_platform_code, evidence, platform};
use arkavo_device_identity::{AgentIdentity, get_or_create_device_id};

const TEST_APP_VERSION: &str = "0.38.2";

#[test]
fn test_end_to_end_device_attestation_flow() {
    println!("\n=== End-to-End Device Attestation Test ===\n");

    println!("Step 1: Get or create device ID");
    let device_id = get_or_create_device_id().expect("Failed to get device ID");
    println!("✓ Device ID: {}", device_id);

    println!("\nStep 2: Create agent identity");
    let agent_identity = AgentIdentity::with_device_id(device_id, TEST_APP_VERSION.to_string());
    println!("✓ Agent identity created");
    println!("  - Device ID: {}", agent_identity.device_id);
    println!("  - App version: {}", agent_identity.app_version);
    if let Some(agent_id) = agent_identity.agent_id {
        println!("  - Agent ID: {}", agent_id);
    }

    println!("\nStep 3: Detect platform");
    let platform_code = detect_platform_code();
    println!("✓ Platform: {}", platform_code);

    println!("\nStep 4: Create attestor (auto-selects best backend)");
    let attestor = platform::create_attestor(agent_identity.clone(), platform_code.clone());
    let capabilities = attestor.get_capabilities();
    println!("✓ Attestor created");
    println!("  - Type: {:?}", capabilities.attestation_type);
    println!(
        "  - Supports freshness: {}",
        capabilities.supports_freshness
    );
    println!(
        "  - Supports hardware binding: {}",
        capabilities.supports_hardware_binding
    );

    println!("\nStep 5: Collect platform evidence");
    let evidence = attestor
        .collect_evidence()
        .expect("Failed to collect evidence");
    println!("✓ Evidence collected");
    println!("  - Device ID: {}", evidence.device_id);
    println!("  - Platform: {}", evidence.platform_code);
    println!("  - State: {:?}", evidence.platform_state);
    println!("  - Type: {:?}", evidence.attestation_type);
    println!("  - Evidence size: {} bytes", evidence.evidence_blob.len());
    println!("  - Timestamp: {}", evidence.timestamp);

    println!("\nStep 6: Validate evidence");
    evidence::validate_evidence(&evidence).expect("Evidence validation failed");
    println!("✓ Evidence is valid");

    println!("\nStep 7: Serialize evidence to JSON (current format)");
    let json = serde_json::to_string_pretty(&evidence).expect("Failed to serialize");
    println!("✓ Evidence serialized:");
    println!("{}", json);

    println!("\n=== Attestation Flow Complete ===\n");

    assert_eq!(evidence.device_id, device_id);
    assert_eq!(evidence.platform_code, platform_code);
    assert_eq!(evidence.app_version, TEST_APP_VERSION);
    assert!(!evidence.evidence_blob.is_empty());
    assert!(matches!(
        evidence.platform_state,
        SecurityState::Trusted
            | SecurityState::Suspicious
            | SecurityState::Compromised
            | SecurityState::Unknown
    ));

    #[cfg(target_os = "macos")]
    {
        println!("macOS-specific assertions:");
        if capabilities.attestation_type == arkavo_attestation::AttestationType::SecureEnclave {
            println!("✓ Using Secure Enclave attestation");
            assert!(capabilities.supports_freshness);
            assert!(capabilities.supports_hardware_binding);
        } else {
            println!("✓ Fallback to software fingerprint (expected on some systems)");
        }
    }
}

#[test]
fn test_attestation_freshness() {
    println!("\n=== Testing Attestation Freshness ===\n");

    let device_id = get_or_create_device_id().expect("Failed to get device ID");
    let identity = AgentIdentity::with_device_id(device_id, TEST_APP_VERSION.to_string());
    let platform_code = detect_platform_code();
    let attestor = platform::create_attestor(identity, platform_code);

    println!("Collecting first attestation...");
    let evidence1 = attestor
        .collect_evidence()
        .expect("Failed to collect evidence");
    let timestamp1 = evidence1.timestamp;
    println!("✓ First timestamp: {}", timestamp1);

    std::thread::sleep(std::time::Duration::from_secs(1));

    println!("Collecting second attestation...");
    let evidence2 = attestor
        .collect_evidence()
        .expect("Failed to collect evidence");
    let timestamp2 = evidence2.timestamp;
    println!("✓ Second timestamp: {}", timestamp2);

    println!("Verifying freshness...");
    assert!(
        timestamp2 >= timestamp1,
        "Second attestation should be newer"
    );
    println!(
        "✓ Attestation freshness verified (Δ = {} seconds)",
        timestamp2 - timestamp1
    );

    println!("\n=== Freshness Test Complete ===\n");
}

#[test]
fn test_security_state_detection() {
    println!("\n=== Testing Security State Detection ===\n");

    let device_id = get_or_create_device_id().expect("Failed to get device ID");
    let identity = AgentIdentity::with_device_id(device_id, TEST_APP_VERSION.to_string());
    let platform_code = detect_platform_code();
    let attestor = platform::create_attestor(identity, platform_code);

    println!("Getting security state...");
    let state = attestor.get_security_state();
    println!("✓ Security state: {:?}", state);

    let evidence = attestor
        .collect_evidence()
        .expect("Failed to collect evidence");
    assert_eq!(evidence.platform_state, state);

    match state {
        SecurityState::Trusted => {
            println!("✓ System is in trusted state");
        }
        SecurityState::Suspicious => {
            println!("⚠ System is in suspicious state (debug mode or other anomalies detected)");
        }
        SecurityState::Compromised => {
            println!("❌ System is compromised (jailbreak or tampering detected)");
        }
        SecurityState::Unknown => {
            println!("? Security state unknown (software attestation)");
        }
    }

    println!("\n=== Security State Test Complete ===\n");
}

#[test]
fn test_multiple_attestations_same_device() {
    println!("\n=== Testing Multiple Attestations ===\n");

    let device_id = get_or_create_device_id().expect("Failed to get device ID");
    println!("Device ID: {}", device_id);

    for i in 1..=3 {
        println!("\nAttestation #{}", i);
        let identity = AgentIdentity::with_device_id(device_id, format!("0.38.{}", i));
        let platform_code = detect_platform_code();
        let attestor = platform::create_attestor(identity, platform_code);

        let evidence = attestor
            .collect_evidence()
            .expect("Failed to collect evidence");

        assert_eq!(
            evidence.device_id, device_id,
            "Device ID should remain stable"
        );
        assert_eq!(evidence.app_version, format!("0.38.{}", i));
        println!(
            "✓ Attestation #{} completed (app version: {})",
            i, evidence.app_version
        );
    }

    println!("\n=== Multiple Attestations Test Complete ===\n");
}
