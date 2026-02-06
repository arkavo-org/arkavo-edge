//! Simple TDF encryption example using the high-level API
//!
//! This example demonstrates the new v0.5.0 fluent builder API with PolicyBuilder.
//!
//! Usage:
//! ```bash
//! cargo run --example simple_tdf
//! ```

use opentdf::prelude::*;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("=== Simple TDF Encryption (v0.5.0) ===\n");

    // Create a policy using the new PolicyBuilder
    let policy = PolicyBuilder::new()
        .id_auto() // Auto-generate UUID
        .attribute_fqn("https://example.com/attr/classification/value/secret")?
        .dissemination(["user@example.com"])
        .build()?;

    println!("Created policy: {}", policy.uuid);

    // Encrypt data with fluent API
    let plaintext = b"Hello from the simplified TDF API!";

    Tdf::encrypt(plaintext)
        .kas_url("https://kas.example.com")
        .policy(policy)
        .to_file("/tmp/simple.tdf")?;

    println!("✓ Encrypted {} bytes to /tmp/simple.tdf", plaintext.len());
    println!("\n🎉 That's it! Clean builder pattern with compile-time safety.");

    Ok(())
}
