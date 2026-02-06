//! Create a TDF file with proper KAS integration
//!
//! This example demonstrates creating a cross-platform compatible TDF file
//! using the OpenTDF platform's KAS service for key wrapping.
//!
//! Usage:
//! ```bash
//! export KAS_URL="http://localhost:8080/kas"
//! cargo run --example create_tdf_platform --features kas -- /tmp/test-platform.tdf
//! ```

use opentdf::{
    Policy, TdfArchiveBuilder, TdfEncryption, TdfManifest,
    manifest::{IntegrityInformationExt, KeyAccessExt, TdfManifestExt},
    wrap_key_with_rsa_oaep,
};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get command line arguments
    let args: Vec<String> = env::args().collect();
    let output_path = if args.len() >= 2 {
        args[1].clone()
    } else {
        "/tmp/test-platform.tdf".to_string()
    };

    // Get KAS URL from environment
    let kas_url = env::var("KAS_URL").unwrap_or_else(|_| "http://localhost:8080/kas".to_string());

    println!("=== Creating TDF with KAS Integration ===");
    println!("KAS URL:  {}", kas_url);
    println!("Output:   {}", output_path);
    println!();

    // Prepare test data
    let plaintext = b"Hello from Rust! This TDF is compatible with all OpenTDF implementations.";

    println!(
        "Plaintext ({} bytes): {}",
        plaintext.len(),
        String::from_utf8_lossy(plaintext)
    );
    println!();

    // Step 1: Fetch KAS public key
    println!("1. Fetching KAS public key...");
    let http_client = reqwest::Client::new();
    let kas_key_response = opentdf::kas_key::fetch_kas_public_key(&kas_url, &http_client).await?;
    println!("   ✓ Fetched key ID: {}", kas_key_response.kid);
    println!();

    // Step 2: Create encryption and encrypt data
    println!("2. Encrypting data...");
    let tdf_encryption = TdfEncryption::new()?;
    let segmented = tdf_encryption.encrypt_with_segments(plaintext, 2 * 1024 * 1024)?; // 2MB segments
    println!(
        "   ✓ Data encrypted with {} segments",
        segmented.segment_info.len()
    );
    println!();

    // Step 3: Wrap the payload key with KAS public key
    println!("3. Wrapping payload key with KAS public key...");
    let wrapped_key =
        wrap_key_with_rsa_oaep(tdf_encryption.payload_key(), &kas_key_response.public_key)?;
    println!("   ✓ Key wrapped ({} bytes base64)", wrapped_key.len());
    println!();

    // Step 4: Create policy
    println!("4. Creating policy...");
    let policy = Policy::new(
        "00000000-0000-0000-0000-000000000000".to_string(),
        vec![],
        vec![],
    );
    println!("   ✓ Policy created with UUID: {}", policy.uuid);
    println!();

    // Step 5: Build manifest
    println!("5. Building manifest...");
    let mut manifest = TdfManifest::new("0.payload".to_string(), kas_url.clone());
    manifest.encryption_information.method.algorithm = "AES-256-GCM".to_string();
    manifest.encryption_information.method.iv = String::new(); // Segments have their own IVs
    manifest.payload.mime_type = Some("text/plain; charset=utf-8".to_string());

    // Set policy
    manifest.set_policy(&policy)?;

    // Generate policy binding
    let policy_json = policy.to_json()?;
    manifest.encryption_information.key_access[0]
        .generate_policy_binding_raw(&policy_json, tdf_encryption.payload_key())?;

    // Set wrapped key and kid
    manifest.encryption_information.key_access[0].wrapped_key = wrapped_key;
    manifest.encryption_information.key_access[0].kid = Some(kas_key_response.kid.clone());

    // Add segment information
    use opentdf::manifest::Segment;
    for seg_info in &segmented.segment_info {
        manifest
            .encryption_information
            .integrity_information
            .segments
            .push(Segment {
                hash: seg_info.hash.clone(),
                segment_size: Some(seg_info.plaintext_size),
                encrypted_segment_size: Some(seg_info.encrypted_size),
            });
    }

    // Set segment defaults
    if let Some(first_seg) = segmented.segment_info.first() {
        manifest
            .encryption_information
            .integrity_information
            .segment_size_default = first_seg.plaintext_size;
        manifest
            .encryption_information
            .integrity_information
            .encrypted_segment_size_default = first_seg.encrypted_size;
    }

    // Generate root signature
    manifest
        .encryption_information
        .integrity_information
        .generate_root_signature(&segmented.gmac_tags, tdf_encryption.payload_key())?;

    println!("   ✓ Manifest created");
    println!();

    // Step 6: Create TDF archive
    println!("6. Creating TDF archive...");
    let mut builder = TdfArchiveBuilder::new(&output_path)?;
    builder.add_entry_with_segments(&manifest, &segmented.segments, 0)?;
    builder.finish()?;

    let file_size = std::fs::metadata(&output_path)?.len();
    println!("   ✓ TDF archive created ({} bytes)", file_size);
    println!();

    println!("=== ✓ SUCCESS! ==");
    println!();
    println!("To decrypt with Go otdfctl:");
    println!(
        "  /Users/paul/Projects/opentdf/otdfctl/otdfctl decrypt {} \\",
        output_path
    );
    println!("    --host http://localhost:8080 \\");
    println!("    --tls-no-verify \\");
    println!("    --with-client-creds '{{\"clientId\":\"opentdf\",\"clientSecret\":\"secret\"}}'");
    println!();

    Ok(())
}
