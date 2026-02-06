//! NanoTDF with Compressed Public Key
//!
//! Tests NanoTDF with compressed (33-byte) vs uncompressed (65-byte) ephemeral public keys.

use opentdf_crypto::tdf::nanotdf::NanoTdfBuilder;
use opentdf_protocol::nanotdf::header::EccMode;
use p256::pkcs8::EncodePrivateKey;
use p256::{EncodedPoint, SecretKey as P256SecretKey};
use rand::rngs::OsRng;
use std::fs;

fn main() {
    println!("=== NanoTDF with Compressed Public Key ===\n");

    // Generate test keypair
    let secret_key = P256SecretKey::random(&mut OsRng);
    let public_key = secret_key.public_key();
    let private_bytes = secret_key.to_pkcs8_der().unwrap().to_bytes().to_vec();

    // Get both compressed and uncompressed formats
    let uncompressed_bytes = public_key.to_sec1_bytes().to_vec();
    let compressed_point = EncodedPoint::from(public_key).compress();
    let compressed_bytes = compressed_point.as_bytes().to_vec();

    println!("1. Key Generation:");
    println!("   Private key size: {} bytes", private_bytes.len());
    println!(
        "   Uncompressed public key: {} bytes",
        uncompressed_bytes.len()
    );
    println!("   Compressed public key: {} bytes", compressed_bytes.len());
    println!(
        "   Uncompressed (first 16 bytes): {:02x?}",
        &uncompressed_bytes[..16]
    );
    println!(
        "   Compressed (first 16 bytes): {:02x?}",
        &compressed_bytes[..16.min(compressed_bytes.len())]
    );

    let policy = r#"{"body":{"dataAttributes":[],"dissem":[]}}"#;
    let plaintext = b"Test with compressed key";

    println!("\n2. Testing with COMPRESSED public key (33 bytes)...");
    test_nanotdf(
        &compressed_bytes,
        &private_bytes,
        policy,
        plaintext,
        "/tmp/test-compressed-key.nanotdf",
    );

    println!("\n3. Testing with UNCOMPRESSED public key (65 bytes)...");
    test_nanotdf(
        &uncompressed_bytes,
        &private_bytes,
        policy,
        plaintext,
        "/tmp/test-uncompressed-key.nanotdf",
    );
}

fn test_nanotdf(
    public_key: &[u8],
    private_key: &[u8],
    policy: &str,
    plaintext: &[u8],
    filename: &str,
) {
    println!("   Public key format: {} bytes", public_key.len());

    let result = NanoTdfBuilder::new()
        .kas_url("http://localhost:8080/kas")
        .policy_plaintext(policy.as_bytes().to_vec())
        .ecc_mode(EccMode::Secp256r1)
        .encrypt(plaintext, public_key);

    match result {
        Ok(nanotdf) => {
            println!("   ✓ Encryption succeeded");

            match nanotdf.to_bytes() {
                Ok(bytes) => {
                    println!("   ✓ Serialization succeeded: {} bytes", bytes.len());

                    // Save to file
                    fs::write(filename, &bytes).unwrap();
                    fs::write(format!("{}.private.key", filename), private_key).unwrap();
                    println!("   ✓ Saved to: {}", filename);

                    // Test roundtrip
                    match opentdf_crypto::tdf::nanotdf::NanoTdf::from_bytes(&bytes) {
                        Ok(decoded) => {
                            println!("   ✓ Deserialization succeeded");

                            match decoded.decrypt(private_key) {
                                Ok(decrypted) => {
                                    if decrypted == plaintext {
                                        println!("   ✓ Decryption succeeded - Plaintext matches!");
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

    println!("\n   To test with otdfctl:");
    println!(
        "     /Users/paul/Projects/opentdf/otdfctl/otdfctl decrypt {} \\",
        filename
    );
    println!("       --host http://localhost:8080 --tls-no-verify \\");
    println!(
        "       --with-client-creds '{{\"clientId\":\"opentdf\",\"clientSecret\":\"secret\"}}'"
    );
}
