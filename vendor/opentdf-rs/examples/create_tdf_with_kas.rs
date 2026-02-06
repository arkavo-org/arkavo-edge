//! Create a TDF file using KAS public key wrapping
//!
//! This example demonstrates creating a TDF file that can be decrypted
//! using the KAS rewrap protocol using the NEW simplified API.
//!
//! Usage:
//! ```bash
//! export KAS_URL="http://localhost:8080/kas"
//! cargo run --example create_tdf_with_kas --features kas -- output.tdf
//! ```

use opentdf::{Policy, Tdf};
use std::env;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get command line arguments
    let args: Vec<String> = env::args().collect();
    let output_path = if args.len() >= 2 {
        args[1].clone()
    } else {
        "/tmp/test-from-rust.tdf".to_string()
    };

    // Get KAS URL from environment
    let kas_url = env::var("KAS_URL").unwrap_or_else(|_| "http://localhost:8080/kas".to_string());

    println!("=== Creating TDF with Simplified API ===");
    println!("KAS URL:  {}", kas_url);
    println!("Output:   {}", output_path);
    println!();

    // Prepare test data
    let plaintext: Vec<u8> = if std::path::Path::new("/tmp/large-test.bin").exists() {
        std::fs::read("/tmp/large-test.bin")?
    } else {
        b"Hello from Rust opentdf-rs! This TDF can be decrypted by any OpenTDF implementation."
            .to_vec()
    };

    if plaintext.len() > 100 {
        println!(
            "Plaintext: {} bytes ({:.2} MB)",
            plaintext.len(),
            plaintext.len() as f64 / 1024.0 / 1024.0
        );
    } else {
        println!(
            "Plaintext ({} bytes): {}",
            plaintext.len(),
            String::from_utf8_lossy(&plaintext)
        );
    }

    // Create policy
    let policy = Policy::new(
        "00000000-0000-0000-0000-000000000000".to_string(),
        vec![],
        vec![],
    );

    // NEW API: Just 4 lines!
    Tdf::encrypt(plaintext.clone())
        .kas_url(&kas_url)
        .policy(policy)
        .to_file(&output_path)?;

    let file_size = std::fs::metadata(&output_path)?.len();
    println!();
    println!("✓ SUCCESS! TDF file created: {}", output_path);
    println!("  File size: {} bytes", file_size);
    println!();
    println!("To decrypt with otdfctl:");
    println!(
        "  /Users/paul/Projects/opentdf/otdfctl/otdfctl decrypt {} \\",
        output_path
    );
    println!("    --host http://localhost:8080 \\");
    println!("    --tls-no-verify \\");
    println!("    --with-client-creds opentdf:secret");
    println!();
    println!("To decrypt with Rust:");
    println!("  export KAS_URL='{}'", kas_url);
    println!("  export KAS_OAUTH_TOKEN='your-token-here'");
    println!(
        "  cargo run --example kas_decrypt --features kas -- {}",
        output_path
    );

    Ok(())
}
