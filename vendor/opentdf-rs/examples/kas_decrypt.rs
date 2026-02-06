//! KAS decryption example
//!
//! This example demonstrates decrypting TDF files using KAS.
//!
//! Usage:
//! ```bash
//! export KAS_URL="http://10.0.0.138:8080/kas"
//! export KAS_OAUTH_TOKEN="your-token"
//! cargo run --example kas_decrypt --features kas -- /path/to/file.tdf
//! ```

use opentdf::{TdfArchive, kas::KasClient, manifest::TdfManifestExt};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get command line arguments
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <tdf-file>", args[0]);
        eprintln!("\nEnvironment variables:");
        eprintln!("  KAS_URL         - KAS endpoint URL (default: http://10.0.0.138:8080/kas)");
        eprintln!("  KAS_OAUTH_TOKEN - OAuth bearer token for authentication");
        std::process::exit(1);
    }

    let tdf_path = &args[1];

    // Get KAS configuration from environment
    let kas_url = env::var("KAS_URL").unwrap_or_else(|_| "http://10.0.0.138:8080/kas".to_string());
    let kas_token = env::var("KAS_OAUTH_TOKEN").unwrap_or_else(|_| {
        eprintln!("Warning: KAS_OAUTH_TOKEN not set, using empty token");
        String::new()
    });

    println!("=== KAS Decryption Test ===");
    println!("TDF file: {}", tdf_path);
    println!("KAS URL:  {}", kas_url);
    println!();

    // Create KAS client
    println!("1. Creating KAS client...");
    let kas_client = match KasClient::new(&kas_url, &kas_token) {
        Ok(client) => {
            println!("   ✓ KAS client created");
            client
        }
        Err(e) => {
            eprintln!("   ✗ Failed to create KAS client: {}", e);
            return Err(e.into());
        }
    };

    // Open TDF archive
    println!("2. Opening TDF archive...");
    let mut archive = match TdfArchive::open(tdf_path) {
        Ok(archive) => {
            println!("   ✓ TDF archive opened");
            archive
        }
        Err(e) => {
            eprintln!("   ✗ Failed to open TDF: {}", e);
            return Err(e.into());
        }
    };

    // Get first entry
    println!("3. Reading TDF entry...");
    let entry = match archive.by_index() {
        Ok(entry) => {
            println!("   ✓ Entry read successfully");
            println!("     - Payload size: {} bytes", entry.payload.len());
            println!(
                "     - Algorithm: {}",
                entry.manifest.encryption_information.method.algorithm
            );
            entry
        }
        Err(e) => {
            eprintln!("   ✗ Failed to read entry: {}", e);
            return Err(e.into());
        }
    };

    // Display manifest info
    println!("4. Manifest information:");
    println!("   - Payload URL: {}", entry.manifest.payload.url);
    println!("   - MIME type: {:?}", entry.manifest.payload.mime_type);
    if let Some(spec_version) = &entry.manifest.payload.tdf_spec_version {
        println!("   - TDF spec: {}", spec_version);
    }

    // Display key access info
    if let Some(ka) = entry.manifest.encryption_information.key_access.first() {
        println!("   - KAS URL: {}", ka.url);
        println!("   - Protocol: {}", ka.protocol);
        println!("   - Key type: {}", ka.access_type);
        println!(
            "   - Policy binding: {} ({})",
            ka.policy_binding.alg,
            &ka.policy_binding.hash[..16]
        );
    }

    // Display policy if available
    println!("5. Policy information:");
    match entry.manifest.get_policy() {
        Ok(policy) => {
            println!("   ✓ Policy found");
            println!("     - UUID: {}", policy.uuid);
            if let Some(from) = policy.valid_from {
                println!("     - Valid from: {}", from);
            }
            if let Some(to) = policy.valid_to {
                println!("     - Valid to: {}", to);
            }
            println!("     - Dissemination: {:?}", policy.body.dissem);
            println!(
                "     - Attributes: {} conditions",
                policy.body.attributes.len()
            );
        }
        Err(e) => {
            println!("   ⚠ No policy or failed to parse: {}", e);
        }
    }

    // Attempt KAS decryption
    println!("6. Decrypting with KAS...");
    match entry.decrypt_with_kas(&kas_client).await {
        Ok(plaintext) => {
            println!("   ✓ Successfully decrypted!");
            println!("     - Plaintext size: {} bytes", plaintext.len());
            println!();

            // Try to display as text if it's UTF-8
            match String::from_utf8(plaintext.clone()) {
                Ok(text) => {
                    if text.len() <= 500 {
                        println!("=== Decrypted Content ===");
                        println!("{}", text);
                    } else {
                        println!("=== Decrypted Content (first 500 chars) ===");
                        println!("{}", &text[..500]);
                        println!("\n... ({} more bytes)", text.len() - 500);
                    }
                }
                Err(_) => {
                    println!("=== Decrypted Content (binary) ===");
                    println!("First 100 bytes (hex):");
                    let display_len = plaintext.len().min(100);
                    for (i, chunk) in plaintext[..display_len].chunks(16).enumerate() {
                        print!("{:04x}: ", i * 16);
                        for byte in chunk {
                            print!("{:02x} ", byte);
                        }
                        println!();
                    }
                    if plaintext.len() > 100 {
                        println!("... ({} more bytes)", plaintext.len() - 100);
                    }
                }
            }

            println!();
            println!("✓ KAS decryption test PASSED");
            Ok(())
        }
        Err(e) => {
            eprintln!("   ✗ Decryption failed: {}", e);
            eprintln!();
            eprintln!("✗ KAS decryption test FAILED");
            Err(e.into())
        }
    }
}
