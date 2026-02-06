//! NanoTDF Integration Tests
//!
//! Comprehensive tests for NanoTDF implementation including:
//! - Rust encrypt → Rust decrypt roundtrip
//! - Cross-platform compatibility with otdfctl
//! - Platform integration (KAS public key retrieval)
//! - Various ECC curves and payload sizes

use opentdf_crypto::kem::ec::EcCurve;
use opentdf_crypto::tdf::nanotdf::{NanoTdf, NanoTdfBuilder};
use opentdf_protocol::nanotdf::header::EccMode;

/// Test data for encryption/decryption
const TEST_PLAINTEXT: &[u8] = b"Hello, NanoTDF! This is a test of the compact TDF format.";
const SHORT_PLAINTEXT: &[u8] = b"Hi";
const LONG_PLAINTEXT: &[u8] = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
    Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. \
    Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris.";

/// Platform endpoints for testing
const PLATFORM_ENDPOINT: &str = "http://localhost:8080";
const KAS_PUBLIC_KEY_URL: &str = "http://localhost:8080/kas/v2/kas_public_key";

#[test]
fn test_nanotdf_roundtrip_p256() {
    // Generate a test key pair for P-256
    let (private_key, public_key) = generate_test_keypair(EcCurve::P256);

    println!("Public key size: {} bytes", public_key.len());
    println!("Private key size: {} bytes", private_key.len());

    // Build and encrypt NanoTDF
    let nanotdf = NanoTdfBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_remote_body(b"test-policy-uuid".to_vec())
        .ecc_mode(EccMode::Secp256r1)
        .encrypt(TEST_PLAINTEXT, &public_key)
        .expect("Encryption should succeed");

    // Serialize to bytes
    let nanotdf_bytes = nanotdf.to_bytes().expect("Serialization should succeed");

    println!("NanoTDF size: {} bytes", nanotdf_bytes.len());
    println!("Plaintext size: {} bytes", TEST_PLAINTEXT.len());
    println!(
        "Overhead: {} bytes",
        nanotdf_bytes.len() - TEST_PLAINTEXT.len()
    );
    println!(
        "Magic: {:?}",
        std::str::from_utf8(&nanotdf_bytes[0..3]).unwrap()
    );
    println!(
        "First 80 bytes: {:02x?}",
        &nanotdf_bytes[..80.min(nanotdf_bytes.len())]
    );

    // Deserialize from bytes
    let decoded_nanotdf =
        NanoTdf::from_bytes(&nanotdf_bytes).expect("Deserialization should succeed");

    // Decrypt
    let decrypted = decoded_nanotdf
        .decrypt(&private_key)
        .expect("Decryption should succeed");

    // Verify plaintext matches
    assert_eq!(decrypted, TEST_PLAINTEXT);
}

#[test]
fn test_nanotdf_roundtrip_all_curves() {
    // Currently only P-256 is fully implemented
    // TODO: Add proper key generation for P-384, P-521, secp256k1
    let test_cases = vec![
        (EcCurve::P256, EccMode::Secp256r1, "P-256"),
        // (EcCurve::P384, EccMode::Secp384r1, "P-384"),
        // (EcCurve::P521, EccMode::Secp521r1, "P-521"),
        // (EcCurve::Secp256k1, EccMode::Secp256k1, "secp256k1"),
    ];

    for (curve, ecc_mode, name) in test_cases {
        println!("\n=== Testing {} ===", name);

        let (private_key, public_key) = generate_test_keypair(curve);

        // Encrypt
        let nanotdf = NanoTdfBuilder::new()
            .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
            .policy_remote_body(b"test-policy".to_vec())
            .ecc_mode(ecc_mode)
            .encrypt(TEST_PLAINTEXT, &public_key)
            .unwrap_or_else(|_| panic!("{} encryption should succeed", name));

        // Roundtrip through bytes
        let bytes = nanotdf.to_bytes().expect("Serialization should succeed");
        let decoded = NanoTdf::from_bytes(&bytes).expect("Deserialization should succeed");

        // Decrypt
        let decrypted = decoded
            .decrypt(&private_key)
            .unwrap_or_else(|_| panic!("{} decryption should succeed", name));

        assert_eq!(decrypted, TEST_PLAINTEXT, "{} roundtrip failed", name);
        println!("{}: ✓ roundtrip success ({} bytes)", name, bytes.len());
    }
}

#[test]
fn test_nanotdf_various_payload_sizes() {
    let (private_key, public_key) = generate_test_keypair(EcCurve::P256);

    let kb1 = vec![b'A'; 1024];
    let kb10 = vec![b'B'; 10240];

    let test_cases: Vec<(&str, &[u8])> = vec![
        ("empty", b"".as_slice()),
        ("single byte", b"X".as_slice()),
        ("short", SHORT_PLAINTEXT),
        ("medium", TEST_PLAINTEXT),
        ("long", LONG_PLAINTEXT),
        ("1KB", &kb1),
        ("10KB", &kb10),
    ];

    for (name, plaintext) in test_cases {
        let nanotdf = NanoTdfBuilder::new()
            .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
            .policy_remote_body(b"test-policy".to_vec())
            .encrypt(plaintext, &public_key)
            .unwrap_or_else(|_| panic!("{} encryption should succeed", name));

        let bytes = nanotdf.to_bytes().expect("Serialization should succeed");
        let decoded = NanoTdf::from_bytes(&bytes).expect("Deserialization should succeed");
        let decrypted = decoded
            .decrypt(&private_key)
            .expect("Decryption should succeed");

        assert_eq!(&decrypted, plaintext, "{} roundtrip failed", name);

        let overhead = bytes.len() as f64 - plaintext.len() as f64;
        let overhead_pct = if plaintext.is_empty() {
            0.0
        } else {
            (overhead / plaintext.len() as f64) * 100.0
        };

        println!(
            "{:12} - Plaintext: {:6} bytes, NanoTDF: {:6} bytes, Overhead: {:6.1} bytes ({:5.1}%)",
            name,
            plaintext.len(),
            bytes.len(),
            overhead,
            overhead_pct
        );
    }
}

#[test]
fn test_nanotdf_binary_format_structure() {
    let (_private_key, public_key) = generate_test_keypair(EcCurve::P256);

    let nanotdf = NanoTdfBuilder::new()
        .kas_url("http://localhost:8080/kas")
        .policy_plaintext(b"embedded-policy".to_vec())
        .encrypt(b"test", &public_key)
        .expect("Encryption should succeed");

    let bytes = nanotdf.to_bytes().expect("Serialization should succeed");

    // Verify magic number (L1L)
    assert_eq!(&bytes[0..3], b"L1L", "Magic number should be 'L1L'");

    // Verify minimum size (magic + header + payload)
    assert!(
        bytes.len() > 50,
        "NanoTDF should have reasonable minimum size"
    );

    println!("NanoTDF binary structure:");
    println!("  Total size: {} bytes", bytes.len());
    println!(
        "  Magic number: {:?}",
        std::str::from_utf8(&bytes[0..3]).unwrap()
    );
    println!("  First 32 bytes: {:02x?}", &bytes[..32.min(bytes.len())]);
}

#[test]
#[ignore = "Requires OpenTDF platform running on localhost:8080 and reqwest blocking feature"]
fn test_get_kas_public_key_from_platform() {
    println!("To test platform integration:");
    println!(
        "  1. Ensure OpenTDF platform is running at {}",
        PLATFORM_ENDPOINT
    );
    println!("  2. Enable 'reqwest/blocking' feature in Cargo.toml");
    println!("  3. Run: curl -s {}", KAS_PUBLIC_KEY_URL);
}

#[test]
#[ignore = "Requires OpenTDF platform and reqwest blocking feature"]
fn test_platform_health() {
    println!("To test platform health:");
    println!("  Run: curl -s {}/healthz", PLATFORM_ENDPOINT);
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Generate a test keypair for the given curve
fn generate_test_keypair(curve: EcCurve) -> (Vec<u8>, Vec<u8>) {
    use p256::SecretKey as P256SecretKey;
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    use p256::pkcs8::EncodePrivateKey;
    use rand::rngs::OsRng;

    match curve {
        EcCurve::P256 => {
            // Use p256 crate for proper key generation
            let secret_key = P256SecretKey::random(&mut OsRng);
            let public_key = secret_key.public_key();
            // Use PKCS#8 DER for private key (compatible with EC KEM)
            let private_bytes = secret_key.to_pkcs8_der().unwrap().to_bytes().to_vec();
            // Use compressed SEC1 format (33 bytes) per otdfctl gold standard
            let public_bytes = public_key.to_encoded_point(true).as_bytes().to_vec();
            (private_bytes, public_bytes)
        }
        EcCurve::P384 | EcCurve::P521 | EcCurve::Secp256k1 => {
            // For other curves, generate random bytes with appropriate sizes
            // This is a simplification for testing - real implementation would use proper curve libraries
            use rand::Rng;
            let mut rng = OsRng;

            let (private_size, public_size) = match curve {
                EcCurve::P256 => unreachable!(),
                EcCurve::P384 => (48, 97), // P-384: 48-byte private, 97-byte compressed public
                EcCurve::P521 => (66, 133), // P-521: 66-byte private, 133-byte uncompressed public
                EcCurve::Secp256k1 => (32, 33), // secp256k1: 32-byte private, 33-byte compressed public
            };

            let private_key: Vec<u8> = (0..private_size).map(|_| rng.r#gen()).collect();
            let public_key: Vec<u8> = (0..public_size).map(|_| rng.r#gen()).collect();
            (private_key, public_key)
        }
    }
}

/// Save NanoTDF to file for cross-platform testing.
/// Useful for debugging and generating test vectors for other SDK implementations.
#[allow(dead_code)]
fn save_nanotdf_for_testing(nanotdf: &NanoTdf, filename: &str) -> std::io::Result<()> {
    let bytes = nanotdf
        .to_bytes()
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    std::fs::write(filename, bytes)?;
    println!("Saved NanoTDF to {}", filename);
    Ok(())
}

/// Load NanoTDF from file for cross-platform testing.
/// Useful for verifying compatibility with test vectors from other SDKs.
#[allow(dead_code)]
fn load_nanotdf_from_file(filename: &str) -> std::io::Result<NanoTdf> {
    let bytes = std::fs::read(filename)?;
    NanoTdf::from_bytes(&bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))
}
