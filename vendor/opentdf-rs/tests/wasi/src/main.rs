use opentdf::{
    AttributeIdentifier, AttributePolicy, AttributeValue, Policy, PolicyBody, TdfArchive,
    TdfArchiveMemoryBuilder, TdfEncryption, TdfManifest,
};
use std::io::Cursor;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== OpenTDF WASI Example ===\n");

    // Test data
    let test_data = b"Hello from WASI! This is confidential data.";
    println!("Original data: {}", String::from_utf8_lossy(test_data));
    println!("Data size: {} bytes\n", test_data.len());

    // Create a simple policy
    println!("Creating policy...");
    let attr_id = AttributeIdentifier::new(
        "https://example.com".to_string(),
        "classification".to_string(),
    );
    let attr_value = AttributeValue::String("confidential".to_string());
    let attr_policy =
        AttributePolicy::condition(attr_id.clone(), opentdf::Operator::Equals, attr_value);

    let policy = Policy {
        uuid: uuid::Uuid::new_v4().to_string(),
        valid_from: None,
        valid_to: None,
        body: PolicyBody {
            attributes: vec![attr_policy],
            dissem: Vec::new(),
        },
    };
    println!("Policy created with UUID: {}", policy.uuid);
    println!("Policy attribute: {}\n", attr_id);

    // Encrypt data
    println!("Encrypting data...");
    let tdf_encryption = TdfEncryption::new()?;
    let encrypted_payload = tdf_encryption.encrypt(test_data)?;
    println!("Encryption successful!");
    println!("  IV length: {} bytes", encrypted_payload.iv.len());
    println!(
        "  Encrypted key length: {} bytes",
        encrypted_payload.encrypted_key.len()
    );
    println!(
        "  Ciphertext length: {} bytes (base64)\n",
        encrypted_payload.ciphertext.len()
    );

    // Decode ciphertext from base64
    use base64::Engine;
    let ciphertext_bytes =
        base64::engine::general_purpose::STANDARD.decode(&encrypted_payload.ciphertext)?;
    println!("Decoded ciphertext: {} bytes\n", ciphertext_bytes.len());

    // Create manifest
    println!("Creating manifest...");
    let mut manifest = TdfManifest::new(
        "0.payload".to_string(),
        "https://kas.example.com".to_string(),
    );
    manifest.set_policy(&policy)?;
    manifest.encryption_information.method.iv = encrypted_payload.iv.clone();

    // Update key access with encrypted key
    if let Some(key_access) = manifest.encryption_information.key_access.first_mut() {
        key_access.wrapped_key = encrypted_payload.encrypted_key.clone();
        key_access.policy_binding = opentdf::manifest::PolicyBinding {
            alg: "HS256".to_string(),
            hash: encrypted_payload.policy_key_hash.clone(),
        };
    }
    println!("Manifest created\n");

    // Build TDF archive in memory (WASI-compatible!)
    println!("Building TDF archive in memory...");
    let mut builder = TdfArchiveMemoryBuilder::new();
    builder.add_entry(&manifest, &ciphertext_bytes, 0)?;
    let tdf_bytes = builder.finish()?;
    println!("TDF archive created!");
    println!("  Archive size: {} bytes\n", tdf_bytes.len());

    // Read TDF archive from memory
    println!("Reading TDF archive from memory...");
    let cursor = Cursor::new(&tdf_bytes);
    let mut archive = TdfArchive::new(cursor)?;
    println!("Archive opened successfully");
    println!("  Number of entries: {}\n", archive.len());

    // Extract entry
    println!("Extracting TDF entry...");
    let entry = archive.get_entry(0)?;
    println!("Entry extracted:");
    println!(
        "  Manifest encryption type: {}",
        entry.manifest.encryption_information.encryption_type
    );
    println!("  Encrypted payload size: {} bytes", entry.payload.len());
    println!(
        "  Number of key access objects: {}",
        entry.manifest.encryption_information.key_access.len()
    );

    println!("\n=== WASI Test Complete ===");
    println!("✓ TDF creation (in-memory) - SUCCESS");
    println!("✓ TDF reading (in-memory) - SUCCESS");
    println!("✓ No filesystem operations required!");

    Ok(())
}
