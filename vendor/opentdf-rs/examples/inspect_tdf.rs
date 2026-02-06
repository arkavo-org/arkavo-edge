//! TDF inspection example (no KAS required)
//!
//! This example inspects TDF files without attempting decryption.

use opentdf::{TdfArchive, manifest::TdfManifestExt};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <tdf-file>", args[0]);
        std::process::exit(1);
    }

    let tdf_path = &args[1];

    println!("=== TDF Inspection (No Decryption) ===");
    println!("File: {}", tdf_path);
    println!();

    // Open TDF archive
    let mut archive = TdfArchive::open(tdf_path)?;
    println!("✓ Archive opened successfully");
    println!("  Entries: {}", archive.len());
    println!();

    // Get first entry
    let entry = archive.by_index()?;
    println!("=== Entry 0 ===");
    println!();

    // Manifest info
    println!("Manifest:");
    println!("  Payload URL: {}", entry.manifest.payload.url);
    println!("  MIME type: {:?}", entry.manifest.payload.mime_type);
    if let Some(spec) = &entry.manifest.payload.tdf_spec_version {
        println!("  TDF spec version: {}", spec);
    }
    println!();

    // Encryption info
    println!("Encryption:");
    let ei = &entry.manifest.encryption_information;
    println!("  Algorithm: {}", ei.method.algorithm);
    println!("  IV: {}", ei.method.iv);
    println!("  Encrypted payload size: {} bytes", entry.payload.len());
    println!();

    // Key access info
    println!("Key Access:");
    for (i, ka) in ei.key_access.iter().enumerate() {
        println!("  [{}] KAS URL: {}", i, ka.url);
        println!("      Protocol: {}", ka.protocol);
        println!("      Type: {}", ka.access_type);
        println!("      Wrapped key: {} bytes", ka.wrapped_key.len());
        println!("      Policy binding:");
        println!("        Algorithm: {}", ka.policy_binding.alg);
        println!("        Hash: {}...", &ka.policy_binding.hash[..32]);
        if let Some(kid) = &ka.kid {
            println!("      Key ID: {}", kid);
        }
        println!();
    }

    // Policy info
    println!("Policy:");
    match entry.manifest.get_policy() {
        Ok(policy) => {
            println!("  ✓ Policy found");
            println!("    UUID: {}", policy.uuid);
            if let Some(from) = policy.valid_from {
                println!("    Valid from: {}", from);
            }
            if let Some(to) = policy.valid_to {
                println!("    Valid to: {}", to);
            }
            println!("    Dissemination: {:?}", policy.body.dissem);
            println!("    Attribute conditions: {}", policy.body.attributes.len());

            // Display attribute policies
            for (i, attr_policy) in policy.body.attributes.iter().enumerate() {
                println!("    [{}] {:#?}", i, attr_policy);
            }
        }
        Err(e) => {
            println!("  ⚠ No policy or parse error: {}", e);
            println!(
                "  Raw policy string: {}",
                &ei.policy[..ei.policy.len().min(100)]
            );
        }
    }

    println!();
    println!("✓ Inspection complete");

    Ok(())
}
