//! NanoTDF Creation Example
//!
//! Demonstrates creating a NanoTDF file using the opentdf-rs library.
//! NanoTDF is a compact binary format designed for constrained environments.

use opentdf_crypto::tdf::nanotdf::NanoTdfBuilder;
use opentdf_protocol::nanotdf::header::EccMode;
use p256::SecretKey as P256SecretKey;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::EncodePrivateKey;
use rand::rngs::OsRng;
use std::env;
use std::fs;

fn main() {
    println!("=== Creating NanoTDF with Rust opentdf-rs ===\n");

    // Get output filename from args or use default
    let args: Vec<String> = env::args().collect();
    let output_file = if args.len() > 1 {
        args[1].clone()
    } else {
        "/tmp/test-rust-nanotdf.bin".to_string()
    };

    println!("Output file: {}", output_file);

    // Generate test keypair for P-256
    println!("\n1. Generating P-256 keypair...");
    let secret_key = P256SecretKey::random(&mut OsRng);
    let public_key = secret_key.public_key();
    let private_bytes = secret_key.to_pkcs8_der().unwrap().to_bytes().to_vec();
    // Use compressed format (33 bytes) per otdfctl gold standard
    let public_bytes = public_key.to_encoded_point(true).as_bytes().to_vec();

    println!("   Public key size: {} bytes", public_bytes.len());
    println!("   Private key size: {} bytes", private_bytes.len());

    // Save keys for later decryption
    let private_key_file = format!("{}.private.key", output_file);
    let public_key_file = format!("{}.public.key", output_file);
    fs::write(&private_key_file, &private_bytes).expect("Failed to write private key");
    fs::write(&public_key_file, &public_bytes).expect("Failed to write public key");
    println!("   Saved keys:");
    println!("     Private: {}", private_key_file);
    println!("     Public:  {}", public_key_file);

    // Create NanoTDF
    let plaintext = b"Hello from Rust opentdf-rs! NanoTDF is compact and efficient.";
    println!("\n2. Creating NanoTDF...");
    println!("   Plaintext: {} bytes", plaintext.len());
    println!("   Content: {:?}", std::str::from_utf8(plaintext).unwrap());

    let nanotdf = NanoTdfBuilder::new()
        .kas_url("http://localhost:8080/kas")
        .policy_remote_body(b"test-policy-uuid".to_vec())
        .ecc_mode(EccMode::Secp256r1)
        .encrypt(plaintext, &public_bytes)
        .expect("Encryption failed");

    println!("   ✓ Encryption succeeded");

    // Serialize to bytes
    let nanotdf_bytes = nanotdf.to_bytes().expect("Serialization failed");
    println!("   ✓ Serialization succeeded");
    println!("   NanoTDF size: {} bytes", nanotdf_bytes.len());
    println!(
        "   Overhead: {} bytes ({:.1}%)",
        nanotdf_bytes.len() - plaintext.len(),
        ((nanotdf_bytes.len() - plaintext.len()) as f64 / plaintext.len() as f64) * 100.0
    );

    // Write to file
    fs::write(&output_file, &nanotdf_bytes).expect("Failed to write NanoTDF file");
    println!("\n✓ SUCCESS! NanoTDF file created: {}", output_file);
    println!("  File size: {} bytes", nanotdf_bytes.len());
    println!(
        "  Magic: {:?}",
        std::str::from_utf8(&nanotdf_bytes[0..3]).unwrap()
    );

    // Verify roundtrip
    println!("\n3. Verifying roundtrip (deserialize + decrypt)...");
    let decoded = opentdf_crypto::tdf::nanotdf::NanoTdf::from_bytes(&nanotdf_bytes)
        .expect("Deserialization failed");
    println!("   ✓ Deserialization succeeded");

    let decrypted = decoded.decrypt(&private_bytes).expect("Decryption failed");
    println!("   ✓ Decryption succeeded");

    if decrypted == plaintext {
        println!("   ✓ Plaintext matches!");
        println!(
            "   Decrypted: {:?}",
            std::str::from_utf8(&decrypted).unwrap()
        );
    } else {
        eprintln!("   ✗ Plaintext mismatch!");
    }

    println!("\n=== Instructions ===");
    println!("To decrypt with Rust:");
    println!("  cargo run --example decrypt_nanotdf {}", output_file);
    println!("\nTo decrypt with otdfctl (when platform is running):");
    println!(
        "  /Users/paul/Projects/opentdf/otdfctl/otdfctl decrypt {} \\",
        output_file
    );
    println!("    --host http://localhost:8080 --tls-no-verify \\");
    println!("    --with-client-creds '{{\"clientId\":\"opentdf\",\"clientSecret\":\"secret\"}}'");
}
