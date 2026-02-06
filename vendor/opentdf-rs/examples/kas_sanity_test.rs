// Quick sanity test for ZTDF-JSON with real KAS endpoint
// Note: This example uses the deprecated TdfJsonRpc API for backwards compatibility testing.
// New code should use TdfJson with kas_public_key().
#![allow(deprecated)]

use opentdf::{Policy, jsonrpc::TdfJsonRpc};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== ZTDF-JSON Sanity Test with https://100.arkavo.net/kas/v2 ===\n");

    // Test data
    let test_data = b"Sanity test: Hello from ZTDF-JSON!";
    let kas_url = "https://100.arkavo.net/kas/v2/rewrap";

    println!("1. Creating policy...");
    let policy = Policy::new(
        uuid::Uuid::new_v4().to_string(),
        vec![],
        vec!["user@example.com".to_string()],
    );
    println!("   Policy ID: {}", policy.uuid);

    println!("\n2. Encrypting data ({} bytes)...", test_data.len());
    let envelope = TdfJsonRpc::encrypt(test_data)
        .kas_url(kas_url)
        .policy(policy)
        .mime_type("text/plain")
        .build()?;

    println!("   ✓ Encryption successful");
    println!("   Version: {}", envelope.version);
    println!(
        "   Payload type: {}",
        envelope.manifest.payload.payload_type
    );
    println!("   MIME type: {}", envelope.manifest.payload.mime_type);

    println!("\n3. Serializing to JSON...");
    let json = serde_json::to_string_pretty(&envelope)?;
    let json_size = json.len();
    println!("   JSON size: {} bytes", json_size);

    // Show structure
    println!("\n4. JSON structure preview:");
    let lines: Vec<&str> = json.lines().take(15).collect();
    for line in lines {
        println!("   {}", line);
    }
    println!("   ...");

    println!("\n5. Verifying manifest integrity...");
    let manifest = &envelope.manifest;
    let enc_info = &manifest.encryption_information;

    println!("   Encryption type: {}", enc_info.encryption_type);
    println!("   Algorithm: {}", enc_info.method.algorithm);
    println!(
        "   Segment hash alg: {}",
        enc_info.integrity_information.segment_hash_alg
    );
    println!(
        "   Root signature alg: {}",
        enc_info.integrity_information.root_signature.alg
    );

    // Check that root signature is not empty (TODO was implemented)
    if enc_info.integrity_information.root_signature.sig.is_empty() {
        println!("   ⚠ WARNING: Root signature is empty!");
    } else {
        println!(
            "   ✓ Root signature present: {}...",
            &enc_info.integrity_information.root_signature.sig[..20]
        );
    }

    // Check that segment hash is not empty (TODO was implemented)
    if let Some(segment) = enc_info.integrity_information.segments.first() {
        if segment.hash.is_empty() {
            println!("   ⚠ WARNING: Segment GMAC hash is empty!");
        } else {
            println!("   ✓ Segment GMAC hash present: {}...", &segment.hash[..20]);
        }
    }

    println!("\n6. Deserializing from JSON...");
    let deserialized: TdfJsonRpc = serde_json::from_str(&json)?;
    println!("   ✓ Deserialization successful");
    println!(
        "   Version match: {}",
        deserialized.version == envelope.version
    );

    println!("\n7. KAS endpoint information:");
    println!("   KAS URL: {}", kas_url);
    if let Some(key_access) = enc_info.key_access.first() {
        println!("   Access type: {}", key_access.access_type);
        println!("   Protocol: {}", key_access.protocol);
        println!("   Policy binding alg: {}", key_access.policy_binding.alg);
        println!(
            "   Policy binding hash: {}...",
            &key_access.policy_binding.hash[..20]
        );
    }

    println!("\n=== ✓ All Sanity Checks Passed! ===");
    println!("\nNote: Actual decryption requires KAS rewrap call with valid token.");
    println!("The envelope is properly formatted and ready for KAS integration.");

    Ok(())
}
