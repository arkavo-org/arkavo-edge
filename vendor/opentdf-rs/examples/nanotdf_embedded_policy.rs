//! NanoTDF with Embedded Policy Example
//!
//! Tests NanoTDF creation with embedded plaintext policy instead of remote policy.
//! This is for testing cross-platform compatibility with otdfctl.

use opentdf_crypto::tdf::nanotdf::NanoTdfBuilder;
use opentdf_protocol::nanotdf::header::EccMode;
use p256::SecretKey as P256SecretKey;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::EncodePrivateKey;
use rand::rngs::OsRng;
use std::fs;

fn main() {
    println!("=== NanoTDF with Embedded Policy ===\n");

    // Generate test keypair
    let secret_key = P256SecretKey::random(&mut OsRng);
    let public_key = secret_key.public_key();
    let private_bytes = secret_key.to_pkcs8_der().unwrap().to_bytes().to_vec();
    // Use compressed format (33 bytes) per otdfctl gold standard
    let public_bytes = public_key.to_encoded_point(true).as_bytes().to_vec();

    println!("1. Key Generation:");
    println!("   Public key size: {} bytes", public_bytes.len());
    println!("   Private key size: {} bytes", private_bytes.len());

    // Create a simple JSON policy
    let policy = r#"{"body":{"dataAttributes":[],"dissem":[]}}"#;
    println!("\n2. Policy:");
    println!("   Type: Embedded Plaintext");
    println!("   Content: {}", policy);
    println!("   Size: {} bytes", policy.len());

    let plaintext = b"Hello from Rust with embedded policy!";
    println!(
        "\n3. Plaintext: {:?} ({} bytes)",
        std::str::from_utf8(plaintext).unwrap(),
        plaintext.len()
    );

    // Build NanoTDF with embedded plaintext policy
    println!("\n4. Creating NanoTDF...");
    let result = NanoTdfBuilder::new()
        .kas_url("http://localhost:8080/kas")
        .policy_plaintext(policy.as_bytes().to_vec())
        .ecc_mode(EccMode::Secp256r1)
        .encrypt(plaintext, &public_bytes);

    match result {
        Ok(nanotdf) => {
            println!("   ✓ Encryption succeeded");

            // Serialize
            match nanotdf.to_bytes() {
                Ok(bytes) => {
                    println!("   ✓ Serialization succeeded: {} bytes", bytes.len());
                    println!(
                        "     Magic: {:?}",
                        std::str::from_utf8(&bytes[0..3]).unwrap()
                    );
                    println!("     Overhead: {} bytes", bytes.len() - plaintext.len());

                    // Save to file
                    let filename = "/tmp/test-embedded-policy.nanotdf";
                    fs::write(filename, &bytes).unwrap();
                    println!("     Saved to: {}", filename);

                    // Save keys
                    fs::write(format!("{}.private.key", filename), &private_bytes).unwrap();
                    fs::write(format!("{}.public.key", filename), &public_bytes).unwrap();

                    // Show hex dump
                    println!("\n5. Binary structure (first 96 bytes):");
                    for (i, chunk) in bytes.chunks(16).take(6).enumerate() {
                        print!("   {:04x}: ", i * 16);
                        for byte in chunk {
                            print!("{:02x} ", byte);
                        }
                        println!();
                    }

                    // Try to deserialize and decrypt
                    println!("\n6. Verifying roundtrip...");
                    match opentdf_crypto::tdf::nanotdf::NanoTdf::from_bytes(&bytes) {
                        Ok(decoded) => {
                            println!("   ✓ Deserialization succeeded");

                            match decoded.decrypt(&private_bytes) {
                                Ok(decrypted) => {
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
                                }
                                Err(e) => {
                                    eprintln!("   ✗ Decryption failed: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("   ✗ Deserialization failed: {}", e);
                        }
                    }

                    println!("\n=== Test with otdfctl ===");
                    println!("To decrypt with otdfctl:");
                    println!(
                        "  /Users/paul/Projects/opentdf/otdfctl/otdfctl decrypt {} \\",
                        filename
                    );
                    println!("    --host http://localhost:8080 --tls-no-verify \\");
                    println!(
                        "    --with-client-creds '{{\"clientId\":\"opentdf\",\"clientSecret\":\"secret\"}}'"
                    );
                }
                Err(e) => {
                    eprintln!("   ✗ Serialization failed: {}", e);
                }
            }
        }
        Err(e) => {
            eprintln!("   ✗ Encryption failed: {}", e);
        }
    }
}
