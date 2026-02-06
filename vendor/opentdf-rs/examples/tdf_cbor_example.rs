//! Example demonstrating TDF-CBOR format for binary protocols
//!
//! This example shows how to:
//! 1. Create a TDF-CBOR envelope with binary payload
//! 2. Serialize it to CBOR bytes for transmission
//! 3. Deserialize and inspect the structure
//!
//! Run with: cargo run --example tdf_cbor_example --features cbor

#[cfg(feature = "cbor")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use opentdf::{Policy, tdf_cbor::TdfCbor};

    println!("=== TDF-CBOR Example ===\n");

    // 1. Create a policy with attribute-based access control
    let policy = Policy::new(
        uuid::Uuid::new_v4().to_string(),
        vec![],
        vec!["user@example.com".to_string()],
    );

    println!("Created policy for user@example.com");

    // 2. Encrypt data into TDF-CBOR format
    let original_data = b"This is sensitive data for CBOR transmission";

    let tdf_cbor = TdfCbor::encrypt(original_data)
        .kas_url("https://kas.example.com")
        .policy(policy)
        .mime_type("text/plain")
        .build()?;

    println!("Encrypted data into TDF-CBOR envelope");

    // 3. Serialize to CBOR bytes
    let cbor_bytes = tdf_cbor.to_bytes()?;

    println!("\n=== CBOR Envelope ===");
    println!("CBOR size: {} bytes", cbor_bytes.len());
    println!(
        "First 16 bytes (hex): {:02x?}",
        &cbor_bytes[..16.min(cbor_bytes.len())]
    );

    // Check magic bytes
    if cbor_bytes.len() >= 3 && cbor_bytes[0..3] == [0xD9, 0xD9, 0xF7] {
        println!("Magic bytes verified: CBOR self-describe tag (55799)");
    }

    // Save to file for cross-SDK testing
    std::fs::write("/tmp/tdf-cross-test/rust_new_enums.cbor", &cbor_bytes)?;
    println!("Saved to: /tmp/tdf-cross-test/rust_new_enums.cbor");

    // 4. Deserialize from CBOR bytes
    let received = TdfCbor::from_bytes(&cbor_bytes)?;

    println!("\n=== Received Envelope ===");
    println!("TDF type: {}", received.tdf);
    println!(
        "Version: {}.{}.{}",
        received.version[0], received.version[1], received.version[2]
    );
    println!("Payload type: {}", received.payload.payload_type);
    println!("Payload protocol: {}", received.payload.protocol);
    if let Some(ref mime) = received.payload.mime_type {
        println!("MIME type: {}", mime);
    }
    println!("Encrypted: {}", received.payload.is_encrypted);
    println!("Payload size: {} bytes", received.payload.value.len());
    println!(
        "Algorithm: {}",
        received.manifest.encryption_information.method.algorithm
    );

    // 5. Compare sizes with JSON format
    // Create JSON for comparison
    use opentdf::jsonrpc::TdfJson;

    let policy2 = opentdf::Policy::new(
        uuid::Uuid::new_v4().to_string(),
        vec![],
        vec!["user@example.com".to_string()],
    );

    let tdf_json = TdfJson::encrypt(original_data)
        .kas_url("https://kas.example.com")
        .policy(policy2)
        .mime_type("text/plain")
        .build()?;

    let json_bytes = serde_json::to_vec(&tdf_json)?;

    println!("\n=== Size Comparison ===");
    println!("TDF-CBOR: {} bytes", cbor_bytes.len());
    println!("TDF-JSON: {} bytes", json_bytes.len());
    let savings = (1.0 - (cbor_bytes.len() as f64 / json_bytes.len() as f64)) * 100.0;
    println!("CBOR is {:.1}% smaller than JSON", savings);

    // 6. Cross-SDK interoperability test
    println!("\n=== Cross-SDK Test ===");
    let swift_cbor_path = "/tmp/tdf-cross-test/swift_new_enums.cbor";
    if std::path::Path::new(swift_cbor_path).exists() {
        let swift_bytes = std::fs::read(swift_cbor_path)?;
        println!(
            "Reading Swift-created CBOR ({} bytes)...",
            swift_bytes.len()
        );
        match TdfCbor::from_bytes(&swift_bytes) {
            Ok(swift_parsed) => {
                println!("✓ Successfully parsed Swift CBOR!");
                println!("  TDF type: {}", swift_parsed.tdf);
                println!(
                    "  Version: {}.{}.{}",
                    swift_parsed.version[0], swift_parsed.version[1], swift_parsed.version[2]
                );
                println!("  Payload type: {}", swift_parsed.payload.payload_type);
                println!("  Protocol: {}", swift_parsed.payload.protocol);
                println!("  Payload size: {} bytes", swift_parsed.payload.value.len());
            }
            Err(e) => {
                println!("✗ Failed to parse Swift CBOR: {}", e);
            }
        }
    } else {
        println!("Note: Swift CBOR not found at {}", swift_cbor_path);
        println!("      Run Swift CLI first to create it for cross-SDK testing.");
    }

    // 7. In production, you would:
    //    - Extract the wrapped key from key_access
    //    - Send it to KAS for unwrapping (with JWT authentication)
    //    - Receive the unwrapped payload key
    //    - Decrypt the payload
    //
    println!("\n=== Decryption (simulated) ===");
    println!("Note: In production, the payload key would be obtained from KAS");
    println!("      after policy validation and attribute checking.");

    Ok(())
}

#[cfg(not(feature = "cbor"))]
fn main() {
    eprintln!("This example requires the 'cbor' feature.");
    eprintln!("Run with: cargo run --example tdf_cbor_example --features cbor");
}
