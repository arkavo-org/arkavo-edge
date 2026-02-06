//! Cross-SDK testing tool for TDF-JSON and TDF-CBOR formats
//!
//! Usage:
//!   cargo run --example cross_sdk_test --features cbor -- create-json <input> <output> [kas_pubkey_pem]
//!   cargo run --example cross_sdk_test --features cbor -- create-cbor <input> <output> [kas_pubkey_pem]
//!   cargo run --example cross_sdk_test --features cbor -- read-json <input>
//!   cargo run --example cross_sdk_test --features cbor -- read-cbor <input>

use std::env;
use std::fs;

#[cfg(feature = "cbor")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use opentdf::{Policy, jsonrpc::TdfJson, tdf_cbor::TdfCbor};

    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        eprintln!("Usage:");
        eprintln!(
            "  {} create-json <input> <output> [kas_pubkey_pem]",
            args[0]
        );
        eprintln!(
            "  {} create-cbor <input> <output> [kas_pubkey_pem]",
            args[0]
        );
        eprintln!("  {} read-json <input>", args[0]);
        eprintln!("  {} read-cbor <input>", args[0]);
        eprintln!();
        eprintln!("Note: Provide EC public key PEM for real key wrapping (smaller files)");
        std::process::exit(1);
    }

    let command = &args[1];

    match command.as_str() {
        "create-json" => {
            if args.len() < 4 {
                eprintln!(
                    "Usage: {} create-json <input> <output> [kas_pubkey_pem]",
                    args[0]
                );
                std::process::exit(1);
            }
            let input_path = &args[2];
            let output_path = &args[3];
            let kas_pubkey_path = args.get(4);

            let plaintext = fs::read(input_path)?;
            println!("Read {} bytes from {}", plaintext.len(), input_path);

            let policy = Policy::new(
                uuid::Uuid::new_v4().to_string(),
                vec![],
                vec!["test@example.com".to_string()],
            );

            let mut builder = TdfJson::encrypt(&plaintext)
                .kas_url("https://kas.example.com")
                .policy(policy)
                .mime_type("text/plain");

            // Use EC key wrapping if public key provided
            if let Some(pem_path) = kas_pubkey_path {
                let pem = fs::read_to_string(pem_path)?;
                println!("Using EC key wrapping with {}", pem_path);
                builder = builder.kas_public_key(&pem);
            }

            let tdf_json = builder.build()?;

            let json_bytes = serde_json::to_vec_pretty(&tdf_json)?;
            fs::write(output_path, &json_bytes)?;

            println!(
                "Created TDF-JSON: {} bytes -> {}",
                json_bytes.len(),
                output_path
            );
            println!("TDF type: {}", tdf_json.tdf);
            println!("Version: {}", tdf_json.version);

            // Show wrapped key size
            if let Some(kao) = tdf_json.manifest.encryption_information.key_access.first() {
                println!("Wrapped key size: {} bytes", kao.wrapped_key.len());
                if kao.ephemeral_public_key.is_some() {
                    println!("Key wrapping: EC (ECDH + HKDF + AES-GCM)");
                } else {
                    println!("Key wrapping: Mock (AES-GCM with policy key)");
                }
            }
            Ok(())
        }

        "create-cbor" => {
            if args.len() < 4 {
                eprintln!(
                    "Usage: {} create-cbor <input> <output> [kas_pubkey_pem]",
                    args[0]
                );
                std::process::exit(1);
            }
            let input_path = &args[2];
            let output_path = &args[3];
            let kas_pubkey_path = args.get(4);

            let plaintext = fs::read(input_path)?;
            println!("Read {} bytes from {}", plaintext.len(), input_path);

            let policy = Policy::new(
                uuid::Uuid::new_v4().to_string(),
                vec![],
                vec!["test@example.com".to_string()],
            );

            let mut builder = TdfCbor::encrypt(&plaintext)
                .kas_url("https://kas.example.com")
                .policy(policy)
                .mime_type("text/plain");

            // Use EC key wrapping if public key provided
            if let Some(pem_path) = kas_pubkey_path {
                let pem = fs::read_to_string(pem_path)?;
                println!("Using EC key wrapping with {}", pem_path);
                builder = builder.kas_public_key(&pem);
            }

            let tdf_cbor = builder.build()?;

            let cbor_bytes = tdf_cbor.to_bytes()?;
            fs::write(output_path, &cbor_bytes)?;

            println!(
                "Created TDF-CBOR: {} bytes -> {}",
                cbor_bytes.len(),
                output_path
            );
            println!("TDF type: {}", tdf_cbor.tdf);
            println!(
                "Version: {}.{}.{}",
                tdf_cbor.version[0], tdf_cbor.version[1], tdf_cbor.version[2]
            );

            // Verify magic bytes
            if cbor_bytes.len() >= 3 && cbor_bytes[0..3] == [0xD9, 0xD9, 0xF7] {
                println!("Magic bytes: OK (CBOR self-describe tag)");
            }

            // Show wrapped key size
            if let Some(kao) = tdf_cbor.manifest.encryption_information.key_access.first() {
                println!("Wrapped key size: {} bytes", kao.wrapped_key.len());
                if kao.ephemeral_public_key.is_some() {
                    println!("Key wrapping: EC (ECDH + HKDF + AES-GCM)");
                } else {
                    println!("Key wrapping: Mock (AES-GCM with policy key)");
                }
            }
            Ok(())
        }

        "read-json" => {
            let input_path = &args[2];

            let json_bytes = fs::read(input_path)?;
            println!("Read {} bytes from {}", json_bytes.len(), input_path);

            let tdf_json: TdfJson = serde_json::from_slice(&json_bytes)?;

            println!("\n=== TDF-JSON Structure ===");
            println!("TDF type: {}", tdf_json.tdf);
            println!("Version: {}", tdf_json.version);
            if let Some(ref created) = tdf_json.created {
                println!("Created: {}", created);
            }
            println!("\nPayload:");
            println!("  Type: {}", tdf_json.payload.payload_type);
            println!("  Protocol: {}", tdf_json.payload.protocol);
            if let Some(ref mime) = tdf_json.payload.mime_type {
                println!("  MIME type: {}", mime);
            }
            println!("  Encrypted: {}", tdf_json.payload.is_encrypted);
            println!("  Value length: {} chars", tdf_json.payload.value.len());

            println!("\nEncryption:");
            println!(
                "  Type: {}",
                tdf_json.manifest.encryption_information.encryption_type
            );
            println!(
                "  Algorithm: {}",
                tdf_json.manifest.encryption_information.method.algorithm
            );
            println!(
                "  Key access objects: {}",
                tdf_json.manifest.encryption_information.key_access.len()
            );

            println!("\n✓ TDF-JSON parsed successfully");
            Ok(())
        }

        "read-cbor" => {
            let input_path = &args[2];

            let cbor_bytes = fs::read(input_path)?;
            println!("Read {} bytes from {}", cbor_bytes.len(), input_path);

            // Check magic bytes
            if cbor_bytes.len() >= 3 && cbor_bytes[0..3] == [0xD9, 0xD9, 0xF7] {
                println!("Magic bytes: OK (CBOR self-describe tag)");
            } else {
                println!("Warning: Missing CBOR magic bytes");
            }

            let tdf_cbor = TdfCbor::from_bytes(&cbor_bytes)?;

            println!("\n=== TDF-CBOR Structure ===");
            println!("TDF type: {}", tdf_cbor.tdf);
            println!(
                "Version: {}.{}.{}",
                tdf_cbor.version[0], tdf_cbor.version[1], tdf_cbor.version[2]
            );
            if let Some(created) = tdf_cbor.created {
                println!("Created: {}", created);
            }

            println!("\nPayload:");
            println!("  Type: {}", tdf_cbor.payload.payload_type);
            println!("  Protocol: {}", tdf_cbor.payload.protocol);
            if let Some(ref mime) = tdf_cbor.payload.mime_type {
                println!("  MIME type: {}", mime);
            }
            println!("  Encrypted: {}", tdf_cbor.payload.is_encrypted);
            println!("  Value length: {} bytes", tdf_cbor.payload.value.len());

            println!("\nEncryption:");
            println!(
                "  Type: {}",
                tdf_cbor.manifest.encryption_information.encryption_type
            );
            println!(
                "  Algorithm: {}",
                tdf_cbor.manifest.encryption_information.method.algorithm
            );
            println!(
                "  Key access objects: {}",
                tdf_cbor.manifest.encryption_information.key_access.len()
            );

            println!("\n✓ TDF-CBOR parsed successfully");
            Ok(())
        }

        _ => {
            eprintln!("Unknown command: {}", command);
            eprintln!("Valid commands: create-json, create-cbor, read-json, read-cbor");
            std::process::exit(1);
        }
    }
}

#[cfg(not(feature = "cbor"))]
fn main() {
    eprintln!("This example requires the 'cbor' feature.");
    eprintln!("Run with: cargo run --example cross_sdk_test --features cbor -- <command> <args>");
}
