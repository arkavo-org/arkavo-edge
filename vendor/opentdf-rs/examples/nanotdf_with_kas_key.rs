//! NanoTDF with Real KAS EC Public Key
//!
//! This example fetches the EC public key from the KAS and uses it for encryption.
//! The resulting NanoTDF can be decrypted by otdfctl using the KAS rewrap protocol.

use opentdf_crypto::tdf::nanotdf::NanoTdfBuilder;
use opentdf_protocol::nanotdf::header::EccMode;
use p256::PublicKey as P256PublicKey;
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::pkcs8::DecodePublicKey;
use std::fs;

fn main() {
    println!("=== NanoTDF with Real KAS EC Public Key ===\n");

    // KAS EC public key from platform
    let kas_pem = r#"-----BEGIN PUBLIC KEY-----
MFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAEsFmEavBcPuLcfnO2Y/+TrjITyuz7
cZiEMC62vIBEpi8wHRB+qlSGFFQhud5n0RlgTT6eqK6kdgtgOaEzEzJCEA==
-----END PUBLIC KEY-----"#;

    println!("1. Parsing KAS EC public key...");
    let public_key =
        P256PublicKey::from_public_key_pem(kas_pem).expect("Failed to parse KAS public key");

    // Convert to compressed format (33 bytes) per NanoTDF spec
    let public_bytes = public_key.to_encoded_point(true).as_bytes().to_vec();
    println!("   ✓ KAS public key parsed");
    println!("   Key size: {} bytes (compressed)", public_bytes.len());
    println!(
        "   First byte: 0x{:02x} (should be 0x02 or 0x03)",
        public_bytes[0]
    );

    // Create a proper JSON policy with UUID (matching otdfctl format)
    let policy = r#"{"uuid":"00000000-0000-0000-0000-000000000000","body":{"dataAttributes":null,"dissem":null}}"#;
    println!("\n2. Policy:");
    println!("   Type: Embedded Plaintext");
    println!("   Content: {}", policy);
    println!("   Size: {} bytes", policy.len());

    let plaintext = b"Hello from Rust using real KAS key!";
    println!(
        "\n3. Plaintext: {:?} ({} bytes)",
        std::str::from_utf8(plaintext).unwrap(),
        plaintext.len()
    );

    // Build NanoTDF with KAS public key and key ID
    println!("\n4. Creating NanoTDF with KAS public key and key ID (kid=e1)...");

    // Test both with and without kid
    let with_kid = true;
    let result = if with_kid {
        NanoTdfBuilder::new()
            .kas_url_with_kid("http://localhost:8080/kas", b"e1")
            .policy_plaintext(policy.as_bytes().to_vec())
            .ecc_mode(EccMode::Secp256r1)
            .encrypt(plaintext, &public_bytes)
    } else {
        NanoTdfBuilder::new()
            .kas_url("http://localhost:8080/kas")
            .policy_plaintext(policy.as_bytes().to_vec())
            .ecc_mode(EccMode::Secp256r1)
            .encrypt(plaintext, &public_bytes)
    };

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
                    let filename = "/tmp/test-with-kas-key.nanotdf";
                    fs::write(filename, &bytes).unwrap();
                    println!("     Saved to: {}", filename);

                    // Show hex dump
                    println!("\n5. Binary structure (first 96 bytes):");
                    for (i, chunk) in bytes.chunks(16).take(6).enumerate() {
                        print!("   {:04x}: ", i * 16);
                        for byte in chunk {
                            print!("{:02x} ", byte);
                        }
                        println!();
                    }

                    println!("\n=== Decrypt with otdfctl ===");
                    println!("This file was encrypted with the KAS public key.");
                    println!("The KAS can perform the rewrap operation to decrypt it.\n");
                    println!("Command:");
                    println!(
                        "  /Users/paul/Projects/opentdf/otdfctl/otdfctl decrypt {} \\",
                        filename
                    );
                    println!("    --host http://localhost:8080 --tls-no-verify \\");
                    println!(
                        "    --with-client-creds '{{\"clientId\":\"opentdf\",\"clientSecret\":\"secret\"}}'"
                    );

                    println!("\n✓ This should work because:");
                    println!("  1. File was encrypted with KAS's EC public key (kid=e1)");
                    println!("  2. KAS has the corresponding private key");
                    println!("  3. otdfctl will request rewrap from KAS");
                    println!("  4. KAS will perform ECDH and return the symmetric key");
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
