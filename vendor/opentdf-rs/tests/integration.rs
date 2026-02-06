use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use opentdf::{
    AttributeIdentifier, AttributePolicy, AttributeValue, Operator, Policy, PolicyBody, TdfArchive,
    TdfArchiveBuilder, TdfEncryption, TdfManifest,
    manifest::{KeyAccessExt, TdfManifestExt},
};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;
use zip::ZipArchive;

#[test]
fn test_tdf_archive_structure_valid() -> Result<(), Box<dyn std::error::Error>> {
    let test_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("sensitive.txt.tdf");

    let mut archive: TdfArchive<File> = TdfArchive::open(test_path)?;

    // Dump archive information
    println!("\n=== TDF Archive Information ===");
    println!("Total entries: {}", archive.len());

    // Read first entry
    let entry = archive.by_index()?;
    println!("\nEntry 0:");
    println!("  Manifest URL: {}", entry.manifest.payload.url);
    println!("  Payload Size: {} bytes", entry.payload.len());
    println!(
        "  Encryption Type: {}",
        entry.manifest.encryption_information.encryption_type
    );
    println!(
        "  Algorithm: {}",
        entry.manifest.encryption_information.method.algorithm
    );
    println!(
        "  Is Streamable: {}",
        entry.manifest.encryption_information.method.is_streamable
    );
    println!(
        "  Number of Key Access Entries: {}",
        entry.manifest.encryption_information.key_access.len()
    );
    println!("  Policy: {}", entry.manifest.get_policy_raw()?);
    println!("===========================\n");

    // Validate structure
    archive.validate()?;

    Ok(())
}

#[test]
fn test_tdf_archive_structure() -> Result<(), Box<dyn std::error::Error>> {
    let test_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("data")
        .join("sensitive.txt.tdf");

    let file = std::fs::File::open(test_path)?;
    let mut archive = ZipArchive::new(file)?;

    // Dump all zip information
    println!("\n=== ZIP Archive Information ===");
    println!("Total files: {}", archive.len());

    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        println!("\nFile Entry {}:", i);
        println!("  Name: {}", file.name());
        println!("  Size: {} bytes", file.size());
        println!("  Compressed Size: {} bytes", file.compressed_size());
        println!("  Compression Method: {:?}", file.compression());
        println!("  Comment: {}", file.comment());
        println!("  CRC32: {:X}", file.crc32());
        println!("  Modified: {:?}", file.last_modified());
        println!("  Is Directory: {}", file.is_dir());
        println!("  Is File: {}", file.is_file());
    }
    println!("===========================\n");

    // First, verify all required files exist
    let required_files = [
        "0.manifest.json",
        "0.payload",
        // "0.c2pa",
    ];

    for required_file in required_files {
        assert!(
            archive.by_name(required_file).is_ok(),
            "Missing required file: {}",
            required_file
        );
    }

    // Read all contents at once to avoid multiple mutable borrows
    let mut manifest_contents = String::new();
    archive
        .by_name("0.manifest.json")?
        .read_to_string(&mut manifest_contents)?;
    println!("Manifest Contents:\n{}", manifest_contents);
    let mut payload = Vec::new();
    archive.by_name("0.payload")?.read_to_end(&mut payload)?;

    // Now validate the contents
    // let manifest: serde_json::Value = serde_json::from_str(&manifest_contents)?;
    // TODO file bug with opentdf
    // assert!(manifest.get("version").is_some(), "Manifest missing version");
    TdfManifest::from_json(&manifest_contents)?;
    assert!(!payload.is_empty(), "Payload file is empty");

    Ok(())
}

#[test]
#[ignore = "Skipping until we update the test for the new API"]
fn test_create_and_read_encrypted_archive() -> Result<(), Box<dyn std::error::Error>> {
    use tempfile::NamedTempFile;

    // Initialize encryption
    let tdf_encryption = TdfEncryption::new()?;
    let original_data = b"sensitive payload data".to_vec();

    // Encrypt the payload
    let encrypted_payload = tdf_encryption.encrypt(&original_data)?;

    // Create manifest with encryption information
    let mut manifest = TdfManifest::new(
        "0.payload".to_string(),
        "http://kas.example.com:4000".to_string(),
    );

    // Update manifest with encryption details
    manifest.encryption_information.method.algorithm = "AES-256-GCM".to_string();
    manifest.encryption_information.method.iv = encrypted_payload.iv.clone();
    manifest.encryption_information.key_access[0].wrapped_key =
        encrypted_payload.encrypted_key.clone();

    // Create policy with encryption metadata
    let policy = Policy {
        uuid: "test-policy".to_string(),
        valid_from: None,
        valid_to: None,
        body: PolicyBody {
            attributes: vec![
                // Create an attribute policy for encryption information
                AttributePolicy::condition(
                    AttributeIdentifier::new("encryption", "keyHash"),
                    Operator::Equals,
                    AttributeValue::String(encrypted_payload.policy_key_hash.clone()),
                ),
            ],
            dissem: vec!["user@example.com".to_string()],
        },
    };
    manifest.set_policy(&policy)?;

    // Generate policy binding
    manifest.encryption_information.key_access[0]
        .generate_policy_binding(&policy, tdf_encryption.policy_key())?;

    // Create archive
    let temp_file = NamedTempFile::new()?;
    let temp_path = temp_file.path().to_owned();

    let mut builder = TdfArchiveBuilder::new(&temp_path)?;
    let encrypted_data = BASE64.decode(&encrypted_payload.ciphertext)?;
    builder.add_entry(&manifest, &encrypted_data, 0)?;
    builder.finish()?;

    // Read it back and verify
    let mut archive = TdfArchive::open(&temp_path)?;
    let entry = archive.by_index()?;

    // Decrypt the payload using the new API
    let mut decryptor = TdfEncryption::with_policy_key(tdf_encryption.policy_key())?;

    // Decode the necessary values
    let ciphertext = entry.payload.clone();

    // Handle combined IV format - take only the first 12 bytes for the IV
    let combined_iv = BASE64.decode(&entry.manifest.encryption_information.method.iv)?;
    let iv = &combined_iv[0..12]; // Use only the first 12 bytes for AES-GCM

    let encrypted_key =
        BASE64.decode(&entry.manifest.encryption_information.key_access[0].wrapped_key)?;

    let decrypted_payload = decryptor.decrypt(&ciphertext, iv, &encrypted_key)?;

    assert_eq!(decrypted_payload, original_data);
    assert_eq!(entry.manifest.payload.url, "0.payload");
    assert_eq!(
        entry.manifest.encryption_information.method.algorithm,
        "AES-256-GCM"
    );

    Ok(())
}

#[test]
#[ignore = "Skipping until we update the test for the new API"]
fn test_encrypted_archive_with_policy_verification() -> Result<(), Box<dyn std::error::Error>> {
    use tempfile::NamedTempFile;

    // Initialize encryption with a specific policy key
    let tdf_encryption = TdfEncryption::new()?;
    let original_data = b"data with policy verification".to_vec();

    // Create and encrypt payload
    let encrypted_payload = tdf_encryption.encrypt(&original_data)?;

    // Create manifest with policy
    let mut manifest = TdfManifest::new(
        "0.payload".to_string(),
        "http://kas.example.com:4000".to_string(),
    );

    // Set up policy with additional constraints
    let expiry_date = chrono::DateTime::parse_from_rfc3339("2025-12-31T23:59:59Z")
        .unwrap()
        .with_timezone(&chrono::Utc);

    let policy = Policy {
        uuid: "test-policy-verification".to_string(),
        valid_from: None,
        valid_to: Some(expiry_date),
        body: PolicyBody {
            attributes: vec![
                // Create CONFIDENTIAL classification attribute
                AttributePolicy::condition(
                    AttributeIdentifier::new("classification", "level"),
                    Operator::Equals,
                    AttributeValue::String("CONFIDENTIAL".to_string()),
                ),
                // Create encryption key hash attribute
                AttributePolicy::condition(
                    AttributeIdentifier::new("encryption", "keyHash"),
                    Operator::Equals,
                    AttributeValue::String(encrypted_payload.policy_key_hash.clone()),
                ),
            ],
            dissem: vec!["user@example.com".to_string()],
        },
    };
    manifest.set_policy(&policy)?;

    // Generate policy binding
    manifest.encryption_information.key_access[0]
        .generate_policy_binding(&policy, tdf_encryption.policy_key())?;

    // Update manifest encryption information
    manifest.encryption_information.method.algorithm = "AES-256-GCM".to_string();
    manifest.encryption_information.method.iv = encrypted_payload.iv.clone();
    manifest.encryption_information.key_access[0].wrapped_key =
        encrypted_payload.encrypted_key.clone();

    // Create archive
    let temp_file = NamedTempFile::new()?;
    let temp_path = temp_file.path().to_owned();

    let mut builder = TdfArchiveBuilder::new(&temp_path)?;
    let encrypted_data = BASE64.decode(&encrypted_payload.ciphertext)?;
    builder.add_entry(&manifest, &encrypted_data, 0)?;
    builder.finish()?;

    // Verify archive
    let mut archive = TdfArchive::open(&temp_path)?;
    let entry = archive.by_index()?;

    // Verify policy contains our key hash
    let stored_policy = entry.manifest.get_policy()?;

    // Find the attribute with the key hash
    let has_key_hash = stored_policy.body.attributes.iter().any(|attr| {
        if let AttributePolicy::Condition(condition) = attr {
            let is_key_hash = condition.attribute.namespace == "encryption"
                && condition.attribute.name == "keyHash";

            if let Some(AttributeValue::String(value)) = &condition.value {
                is_key_hash && value == &encrypted_payload.policy_key_hash
            } else {
                false
            }
        } else {
            false
        }
    });

    assert!(
        has_key_hash,
        "Policy is missing the correct key hash attribute"
    );

    // Verify encryption algorithm
    assert_eq!(
        entry.manifest.encryption_information.method.algorithm,
        "AES-256-GCM"
    );

    // Verify we can decrypt with the correct policy key using the new API
    let mut decryptor = TdfEncryption::with_policy_key(tdf_encryption.policy_key())?;

    // Decode the encrypted payload components
    let ciphertext = BASE64.decode(&encrypted_payload.ciphertext)?;

    // Handle combined IV format - take only the first 12 bytes for the IV
    let combined_iv = BASE64.decode(&encrypted_payload.iv)?;
    let iv = &combined_iv[0..12]; // Use only the first 12 bytes for AES-GCM

    let encrypted_key = BASE64.decode(&encrypted_payload.encrypted_key)?;

    let decrypted = decryptor.decrypt(&ciphertext, iv, &encrypted_key)?;
    assert_eq!(decrypted, original_data);

    Ok(())
}
