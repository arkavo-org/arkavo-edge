//! Create a TDF file for KAS testing
//!
//! This example creates a TDF file that can be decrypted using KAS.
//!
//! Usage:
//! ```bash
//! cargo run --example create_tdf_for_kas --features kas
//! ```

use opentdf::{TdfArchiveBuilder, TdfEncryption, TdfManifest};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let kas_url = env::var("KAS_URL").unwrap_or_else(|_| "http://localhost:8080/kas".to_string());
    let output_path = env::args()
        .nth(1)
        .unwrap_or_else(|| "/tmp/test-from-rust.tdf".to_string());

    println!("=== TDF Creation Test ===");
    println!("KAS URL:  {}", kas_url);
    println!("Output:   {}", output_path);
    println!();

    // Create test data
    let plaintext = b"Hello from Rust opentdf-rs!";
    println!("Plaintext: {}", String::from_utf8_lossy(plaintext));
    println!();

    // Create TDF encryption
    println!("1. Creating TDF encryption...");
    let tdf_encryption = TdfEncryption::new()?;
    println!("   ✓ Encryption created");

    // Encrypt the data
    println!("2. Encrypting data...");
    let encrypted_payload = tdf_encryption.encrypt(plaintext)?;
    println!("   ✓ Data encrypted");
    println!(
        "     - Ciphertext: {} bytes",
        encrypted_payload.ciphertext.len()
    );
    println!("     - IV: {}", encrypted_payload.iv);
    println!(
        "     - Wrapped key: {} bytes",
        encrypted_payload.encrypted_key.len()
    );

    // Create manifest
    println!("3. Creating manifest...");
    let mut manifest = TdfManifest::new("0.payload".to_string(), kas_url.clone());

    // Update manifest with encryption info
    manifest.encryption_information.method.algorithm = "AES-256-GCM".to_string();
    manifest.encryption_information.method.iv = encrypted_payload.iv.clone();
    manifest.encryption_information.key_access[0].wrapped_key =
        encrypted_payload.encrypted_key.clone();

    println!("   ✓ Manifest created");

    // Create TDF archive
    println!("4. Building TDF archive...");
    let mut builder = TdfArchiveBuilder::new(&output_path)?;
    builder.add_entry(&manifest, encrypted_payload.ciphertext.as_bytes(), 0)?;
    builder.finish()?;
    println!("   ✓ TDF archive created");
    println!();

    println!("✓ TDF file created successfully: {}", output_path);
    println!();
    println!("To decrypt with KAS:");
    println!("  export KAS_URL='{}'", kas_url);
    println!("  export KAS_OAUTH_TOKEN='your-token-here'");
    println!(
        "  cargo run --example kas_decrypt --features kas -- {}",
        output_path
    );

    Ok(())
}
