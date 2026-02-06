//! Simple file encryption example
//!
//! Encrypts a file to TDF format with minimal code.
//!
//! Usage:
//! ```bash
//! echo "Sensitive data" > /tmp/input.txt
//! cargo run --example simple_file_encryption
//! ```

use opentdf::{Policy, Tdf};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Simple File Encryption ===\n");

    // Create test input file
    let input_path = "/tmp/input.txt";
    let output_path = "/tmp/encrypted.tdf";

    std::fs::write(input_path, b"This is sensitive file content!")?;

    // Create policy
    let policy = Policy::new(
        uuid::Uuid::new_v4().to_string(),
        vec![],
        vec!["user@example.com".to_string()],
    );

    // Encrypt file - simple and clean!
    Tdf::encrypt_file(input_path, output_path)
        .kas_url("https://kas.example.com")
        .policy(policy)
        .mime_type("text/plain")
        .build()?;

    println!("✓ Encrypted file created: {}", output_path);

    let size = std::fs::metadata(output_path)?.len();
    println!("  TDF size: {} bytes", size);

    Ok(())
}
