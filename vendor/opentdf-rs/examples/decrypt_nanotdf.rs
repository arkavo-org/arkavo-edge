//! NanoTDF Decryption Example
//!
//! Demonstrates decrypting a NanoTDF file using the opentdf-rs library.

use std::env;
use std::fs;

fn main() {
    println!("=== Decrypting NanoTDF with Rust opentdf-rs ===\n");

    // Get input filename from args
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <nanotdf-file>", args[0]);
        eprintln!("\nExample:");
        eprintln!("  cargo run --example decrypt_nanotdf /tmp/test-rust-nanotdf.bin");
        std::process::exit(1);
    }

    let input_file = &args[1];
    println!("Input file: {}", input_file);

    // Read NanoTDF file
    println!("\n1. Reading NanoTDF file...");
    let nanotdf_bytes = fs::read(input_file).expect("Failed to read file");
    println!("   File size: {} bytes", nanotdf_bytes.len());
    println!(
        "   Magic: {:?}",
        std::str::from_utf8(&nanotdf_bytes[0..3]).unwrap_or("???")
    );

    // Deserialize
    println!("\n2. Deserializing NanoTDF...");
    let nanotdf = opentdf_crypto::tdf::nanotdf::NanoTdf::from_bytes(&nanotdf_bytes)
        .expect("Deserialization failed");
    println!("   ✓ Deserialization succeeded");

    // Look for private key file
    let private_key_file = format!("{}.private.key", input_file);
    if !std::path::Path::new(&private_key_file).exists() {
        eprintln!("\n✗ Private key file not found: {}", private_key_file);
        eprintln!("  This example expects the private key to be saved alongside the NanoTDF file.");
        eprintln!("  Create the NanoTDF using: cargo run --example create_nanotdf");
        std::process::exit(1);
    }

    // Read private key
    println!("\n3. Reading private key...");
    let private_key = fs::read(&private_key_file).expect("Failed to read private key");
    println!("   Private key size: {} bytes", private_key.len());

    // Decrypt
    println!("\n4. Decrypting...");
    match nanotdf.decrypt(&private_key) {
        Ok(plaintext) => {
            println!("   ✓ Decryption succeeded");
            println!("   Plaintext size: {} bytes", plaintext.len());

            // Try to print as UTF-8 string
            match std::str::from_utf8(&plaintext) {
                Ok(text) => {
                    println!("\n=== Decrypted Content ===");
                    println!("{}", text);
                }
                Err(_) => {
                    println!("\n=== Decrypted Content (hex) ===");
                    println!("{:02x?}", plaintext);
                }
            }
        }
        Err(e) => {
            eprintln!("   ✗ Decryption failed: {}", e);
            std::process::exit(1);
        }
    }
}
