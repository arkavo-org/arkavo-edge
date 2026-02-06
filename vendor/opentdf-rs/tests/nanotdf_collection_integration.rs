//! NanoTDF Collection Integration Tests
//!
//! Comprehensive integration tests for NanoTDF Collection implementation including:
//! - End-to-end encryption/decryption roundtrip
//! - Both wire formats (container framing and NanoTDF payload framing)
//! - Policy binding validation (GMAC)
//! - Various payload sizes and concurrency scenarios
//! - Header serialization/deserialization

use opentdf_crypto::tdf::{NanoTdfCollectionBuilder, NanoTdfCollectionDecryptor};
use opentdf_protocol::nanotdf::collection::{CollectionItem, MAX_IV, RESERVED_POLICY_IV};
use opentdf_protocol::nanotdf::header::EccMode;
use std::sync::Arc;
use std::thread;

/// Test data constants
const TEST_PLAINTEXT: &[u8] = b"Hello, NanoTDF Collection! This is a test.";
const SHORT_PLAINTEXT: &[u8] = b"Hi";
const LONG_PLAINTEXT: &[u8] = b"Lorem ipsum dolor sit amet, consectetur adipiscing elit. \
    Sed do eiusmod tempor incididunt ut labore et dolore magna aliqua. \
    Ut enim ad minim veniam, quis nostrud exercitation ullamco laboris.";

/// Platform endpoints for testing
const PLATFORM_ENDPOINT: &str = "http://localhost:8080";

// ============================================================================
// Helper Functions
// ============================================================================

/// Generate a test keypair for P-256
fn generate_test_keypair() -> (Vec<u8>, Vec<u8>) {
    use p256::SecretKey as P256SecretKey;
    use p256::elliptic_curve::sec1::ToEncodedPoint;
    use p256::pkcs8::EncodePrivateKey;
    use rand::rngs::OsRng;

    let secret_key = P256SecretKey::random(&mut OsRng);
    let public_key = secret_key.public_key();
    // Use PKCS#8 DER for private key
    let private_bytes = secret_key.to_pkcs8_der().unwrap().to_bytes().to_vec();
    // Use compressed SEC1 format (33 bytes)
    let public_bytes = public_key.to_encoded_point(true).as_bytes().to_vec();
    (private_bytes, public_bytes)
}

// ============================================================================
// Basic Roundtrip Tests
// ============================================================================

#[test]
fn test_collection_basic_roundtrip() {
    let (private_key, public_key) = generate_test_keypair();

    // Create collection
    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(b"test-policy-uuid".to_vec())
        .ecc_mode(EccMode::Secp256r1)
        .build(&public_key)
        .expect("Collection creation should succeed");

    // Encrypt item
    let item = collection
        .encrypt_item(TEST_PLAINTEXT)
        .expect("Encryption should succeed");

    // Verify IV starts at 1 (0 is reserved)
    assert_eq!(item.iv, 1, "First item should have IV=1");

    // Serialize header
    let header_bytes = collection
        .to_header_bytes()
        .expect("Header serialization should succeed");

    println!("Header size: {} bytes", header_bytes.len());
    println!("Item IV: {}", item.iv);
    println!(
        "Ciphertext+tag size: {} bytes",
        item.ciphertext_and_tag.len()
    );

    // Create decryptor from header and KAS private key
    let decryptor =
        NanoTdfCollectionDecryptor::from_header_with_kas_key(&header_bytes, &private_key)
            .expect("Decryptor creation should succeed");

    // Decrypt
    let decrypted = decryptor
        .decrypt_item(&item)
        .expect("Decryption should succeed");

    assert_eq!(decrypted, TEST_PLAINTEXT, "Plaintext should match");
}

#[test]
fn test_collection_multiple_items() {
    let (private_key, public_key) = generate_test_keypair();

    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(b"multi-item-policy".to_vec())
        .build(&public_key)
        .expect("Collection creation should succeed");

    // Encrypt multiple items
    let plaintexts: Vec<&[u8]> = vec![
        b"First message",
        b"Second message",
        b"Third message",
        b"Fourth message",
        b"Fifth message",
    ];

    let items: Vec<CollectionItem> = plaintexts
        .iter()
        .map(|p| {
            collection
                .encrypt_item(p)
                .expect("Encryption should succeed")
        })
        .collect();

    // Verify IVs are sequential
    for (i, item) in items.iter().enumerate() {
        assert_eq!(item.iv, (i + 1) as u32, "IV should be sequential");
    }

    // Serialize header once
    let header_bytes = collection.to_header_bytes().unwrap();

    // Create decryptor
    let decryptor =
        NanoTdfCollectionDecryptor::from_header_with_kas_key(&header_bytes, &private_key).unwrap();

    // Decrypt all items
    for (i, item) in items.iter().enumerate() {
        let decrypted = decryptor
            .decrypt_item(item)
            .expect("Decryption should succeed");
        assert_eq!(decrypted, plaintexts[i], "Item {} should match", i);
    }
}

// ============================================================================
// Wire Format Tests
// ============================================================================

#[test]
fn test_collection_item_container_framing() {
    let (private_key, public_key) = generate_test_keypair();

    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(b"container-framing-test".to_vec())
        .build(&public_key)
        .unwrap();

    let item = collection.encrypt_item(TEST_PLAINTEXT).unwrap();

    // Test container framing (no length prefix)
    let container_bytes = item.to_bytes();
    println!(
        "Container framing size: {} bytes (3 IV + {} ciphertext+tag)",
        container_bytes.len(),
        item.ciphertext_and_tag.len()
    );

    // Parse back
    let parsed = CollectionItem::from_bytes(&container_bytes).expect("Parse should succeed");
    assert_eq!(parsed.iv, item.iv);
    assert_eq!(parsed.ciphertext_and_tag, item.ciphertext_and_tag);

    // Verify decryption works with parsed item
    let header_bytes = collection.to_header_bytes().unwrap();
    let decryptor =
        NanoTdfCollectionDecryptor::from_header_with_kas_key(&header_bytes, &private_key).unwrap();
    let decrypted = decryptor.decrypt_item(&parsed).unwrap();
    assert_eq!(decrypted, TEST_PLAINTEXT);
}

#[test]
fn test_collection_item_nanotdf_payload_framing() {
    let (private_key, public_key) = generate_test_keypair();

    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(b"payload-framing-test".to_vec())
        .build(&public_key)
        .unwrap();

    let item = collection.encrypt_item(TEST_PLAINTEXT).unwrap();

    // Test NanoTDF payload framing (with 3-byte length prefix)
    let payload_bytes = item.to_nanotdf_payload_bytes();
    println!(
        "NanoTDF payload framing size: {} bytes (3 length + 3 IV + {} ciphertext+tag)",
        payload_bytes.len(),
        item.ciphertext_and_tag.len()
    );

    // Verify length prefix
    let length = ((payload_bytes[0] as usize) << 16)
        | ((payload_bytes[1] as usize) << 8)
        | (payload_bytes[2] as usize);
    assert_eq!(
        length,
        3 + item.ciphertext_and_tag.len(),
        "Length prefix should be IV + ciphertext + tag"
    );

    // Parse back
    let parsed =
        CollectionItem::from_nanotdf_payload_bytes(&payload_bytes).expect("Parse should succeed");
    assert_eq!(parsed.iv, item.iv);
    assert_eq!(parsed.ciphertext_and_tag, item.ciphertext_and_tag);

    // Verify decryption works with parsed item
    let header_bytes = collection.to_header_bytes().unwrap();
    let decryptor =
        NanoTdfCollectionDecryptor::from_header_with_kas_key(&header_bytes, &private_key).unwrap();
    let decrypted = decryptor.decrypt_item(&parsed).unwrap();
    assert_eq!(decrypted, TEST_PLAINTEXT);
}

#[test]
fn test_collection_item_reader_writer() {
    use std::io::Cursor;

    let (private_key, public_key) = generate_test_keypair();

    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(b"reader-writer-test".to_vec())
        .build(&public_key)
        .unwrap();

    let item = collection.encrypt_item(TEST_PLAINTEXT).unwrap();

    // Test container framing with writer
    let mut container_buf = Vec::new();
    item.write_to(&mut container_buf).unwrap();

    let mut cursor = Cursor::new(&container_buf);
    let parsed_container = CollectionItem::read_from(&mut cursor, container_buf.len()).unwrap();
    assert_eq!(parsed_container, item);

    // Test NanoTDF payload framing with writer
    let mut payload_buf = Vec::new();
    item.write_nanotdf_payload_to(&mut payload_buf).unwrap();

    let mut cursor2 = Cursor::new(&payload_buf);
    let parsed_payload = CollectionItem::read_nanotdf_payload_from(&mut cursor2).unwrap();
    assert_eq!(parsed_payload, item);

    // Verify decryption
    let header_bytes = collection.to_header_bytes().unwrap();
    let decryptor =
        NanoTdfCollectionDecryptor::from_header_with_kas_key(&header_bytes, &private_key).unwrap();
    let decrypted = decryptor.decrypt_item(&parsed_payload).unwrap();
    assert_eq!(decrypted, TEST_PLAINTEXT);
}

// ============================================================================
// IV Management Tests
// ============================================================================

#[test]
fn test_iv_gcm_nonce_expansion() {
    // Verify IV-to-nonce expansion matches spec: [9 zeros][3-byte IV]
    let item = CollectionItem::new(0x010203, vec![]);
    let nonce = item.to_gcm_nonce();

    assert_eq!(&nonce[0..9], &[0u8; 9], "First 9 bytes should be zeros");
    assert_eq!(
        &nonce[9..12],
        &[0x01, 0x02, 0x03],
        "Last 3 bytes should be the IV"
    );
}

#[test]
fn test_reserved_iv_rejected() {
    // IV 0 is reserved for encrypted policy and should be rejected
    let bytes = [0x00, 0x00, 0x00, 0x01, 0x02, 0x03]; // IV = 0 (reserved)
    let result = CollectionItem::from_bytes(&bytes);

    assert!(result.is_err(), "Reserved IV should be rejected");
    assert!(
        result.unwrap_err().to_string().contains("reserved"),
        "Error should mention reserved"
    );
}

#[test]
fn test_max_iv_accepted() {
    // MAX_IV should be accepted
    let item = CollectionItem::new(MAX_IV, vec![0x42]);
    let bytes = item.to_bytes();
    let parsed = CollectionItem::from_bytes(&bytes).unwrap();

    assert_eq!(parsed.iv, MAX_IV);
}

#[test]
fn test_iv_counter_starts_at_one() {
    let (_, public_key) = generate_test_keypair();

    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(b"iv-start-test".to_vec())
        .build(&public_key)
        .unwrap();

    // First item should have IV=1
    let item = collection.encrypt_item(b"test").unwrap();
    assert_eq!(item.iv, 1, "First item IV should be 1, not 0 (reserved)");
}

#[test]
fn test_remaining_capacity() {
    let (_, public_key) = generate_test_keypair();

    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(b"capacity-test".to_vec())
        .build(&public_key)
        .unwrap();

    // Initial capacity
    assert_eq!(collection.remaining_capacity(), MAX_IV);

    // After encrypting items
    collection.encrypt_item(b"1").unwrap();
    assert_eq!(collection.remaining_capacity(), MAX_IV - 1);

    collection.encrypt_item(b"2").unwrap();
    assert_eq!(collection.remaining_capacity(), MAX_IV - 2);
}

#[test]
fn test_rotation_threshold() {
    let (_, public_key) = generate_test_keypair();

    // Threshold of 5 means: when current_iv() >= 5, threshold is reached
    // Counter starts at 1, so:
    // - After item 1: counter=2, 2>=5=false
    // - After item 2: counter=3, 3>=5=false
    // - After item 3: counter=4, 4>=5=false
    // - After item 4: counter=5, 5>=5=true
    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(b"rotation-test".to_vec())
        .rotation_threshold(5)
        .build(&public_key)
        .unwrap();

    // Initial: counter=1, 1>=5=false
    assert!(!collection.rotation_threshold_reached());

    // Encrypt items until just before threshold
    collection.encrypt_item(b"1").unwrap(); // counter becomes 2
    assert!(!collection.rotation_threshold_reached());

    collection.encrypt_item(b"2").unwrap(); // counter becomes 3
    assert!(!collection.rotation_threshold_reached());

    collection.encrypt_item(b"3").unwrap(); // counter becomes 4
    assert!(!collection.rotation_threshold_reached());

    // Fourth item should trigger threshold (counter becomes 5)
    collection.encrypt_item(b"4").unwrap();
    assert!(
        collection.rotation_threshold_reached(),
        "Threshold should be reached when counter >= threshold"
    );
}

// ============================================================================
// Thread Safety Tests
// ============================================================================

#[test]
fn test_concurrent_encryption() {
    let (_, public_key) = generate_test_keypair();

    let collection = Arc::new(
        NanoTdfCollectionBuilder::new()
            .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
            .policy_plaintext(b"concurrent-test".to_vec())
            .build(&public_key)
            .unwrap(),
    );

    let num_threads = 4;
    let items_per_thread = 50;
    let mut handles = Vec::new();

    for _ in 0..num_threads {
        let coll = Arc::clone(&collection);
        handles.push(thread::spawn(move || {
            let mut ivs = Vec::new();
            for i in 0..items_per_thread {
                let item = coll
                    .encrypt_item(format!("message-{}", i).as_bytes())
                    .unwrap();
                ivs.push(item.iv);
            }
            ivs
        }));
    }

    // Collect all IVs
    let mut all_ivs: Vec<u32> = Vec::new();
    for handle in handles {
        all_ivs.extend(handle.join().unwrap());
    }

    // Verify all IVs are unique
    let total = all_ivs.len();
    all_ivs.sort();
    all_ivs.dedup();

    assert_eq!(
        all_ivs.len(),
        total,
        "All IVs should be unique under concurrent access"
    );

    // Verify IVs are in valid range
    for iv in &all_ivs {
        assert!(
            *iv >= 1 && *iv <= MAX_IV,
            "IV {} should be in valid range",
            iv
        );
    }

    println!(
        "Concurrent test: {} threads x {} items = {} unique IVs",
        num_threads, items_per_thread, total
    );
}

// ============================================================================
// Payload Size Tests
// ============================================================================

#[test]
fn test_various_payload_sizes() {
    let (private_key, public_key) = generate_test_keypair();

    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(b"size-test".to_vec())
        .build(&public_key)
        .unwrap();

    let header_bytes = collection.to_header_bytes().unwrap();
    let decryptor =
        NanoTdfCollectionDecryptor::from_header_with_kas_key(&header_bytes, &private_key).unwrap();

    let kb1 = vec![b'A'; 1024];
    let kb10 = vec![b'B'; 10240];
    let kb100 = vec![b'C'; 102400];

    let test_cases: Vec<(&str, &[u8])> = vec![
        ("empty", b"".as_slice()),
        ("single byte", b"X".as_slice()),
        ("short", SHORT_PLAINTEXT),
        ("medium", TEST_PLAINTEXT),
        ("long", LONG_PLAINTEXT),
        ("1KB", &kb1),
        ("10KB", &kb10),
        ("100KB", &kb100),
    ];

    for (name, plaintext) in test_cases {
        let item = collection
            .encrypt_item(plaintext)
            .unwrap_or_else(|_| panic!("{} encryption should succeed", name));

        let decrypted = decryptor
            .decrypt_item(&item)
            .unwrap_or_else(|_| panic!("{} decryption should succeed", name));

        assert_eq!(&decrypted, plaintext, "{} roundtrip failed", name);

        let container_size = item.container_size();
        let payload_size = item.nanotdf_payload_size();
        let overhead = container_size as f64 - plaintext.len() as f64;

        println!(
            "{:12} - Plaintext: {:6} bytes, Container: {:6} bytes, Payload: {:6} bytes, Overhead: {:+6.0} bytes",
            name,
            plaintext.len(),
            container_size,
            payload_size,
            overhead
        );
    }
}

// ============================================================================
// Decryptor Tests
// ============================================================================

#[test]
fn test_decryptor_with_dek() {
    let (private_key, public_key) = generate_test_keypair();

    // Create collection and encrypt
    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(b"dek-test".to_vec())
        .build(&public_key)
        .unwrap();

    let item = collection.encrypt_item(TEST_PLAINTEXT).unwrap();
    let header_bytes = collection.to_header_bytes().unwrap();

    // Create KAS-side decryptor to get DEK (simulating KAS rewrap)
    let kas_decryptor =
        NanoTdfCollectionDecryptor::from_header_with_kas_key(&header_bytes, &private_key).unwrap();

    // Extract DEK (in real scenario this would come from KAS rewrap response)
    // For testing, we decrypt with KAS decryptor first to verify
    let decrypted_kas = kas_decryptor.decrypt_item(&item).unwrap();
    assert_eq!(decrypted_kas, TEST_PLAINTEXT);

    // Now test client-side decryptor with DEK
    // The DEK isn't directly accessible from the struct, but we can verify
    // the API works by checking that invalid DEK lengths are rejected
    let bad_dek = vec![0u8; 16]; // Wrong length
    let result = NanoTdfCollectionDecryptor::from_header_with_dek(&header_bytes, &bad_dek);
    assert!(result.is_err(), "Should reject invalid DEK length");
}

#[test]
fn test_header_roundtrip() {
    let (private_key, public_key) = generate_test_keypair();

    // Create collection
    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(b"header-roundtrip-test".to_vec())
        .build(&public_key)
        .unwrap();

    // Encrypt item
    let item = collection.encrypt_item(TEST_PLAINTEXT).unwrap();

    // Serialize header
    let header_bytes = collection.to_header_bytes().unwrap();
    println!("Header size: {} bytes", header_bytes.len());

    // Verify magic number
    assert_eq!(
        &header_bytes[0..3],
        b"L1L",
        "Header should start with L1L magic"
    );

    // Create decryptor from serialized header
    let decryptor =
        NanoTdfCollectionDecryptor::from_header_with_kas_key(&header_bytes, &private_key).unwrap();

    // Decrypt
    let decrypted = decryptor.decrypt_item(&item).unwrap();
    assert_eq!(decrypted, TEST_PLAINTEXT, "Roundtrip should preserve data");
}

// ============================================================================
// Policy Binding Tests
// ============================================================================

#[test]
fn test_policy_binding_gmac() {
    use opentdf_crypto::sha2::{Digest, Sha256};

    let (_, public_key) = generate_test_keypair();

    let policy_body = b"test-policy-for-binding";

    // Create collection with plaintext policy
    let collection = NanoTdfCollectionBuilder::new()
        .kas_url(format!("{}/kas", PLATFORM_ENDPOINT))
        .policy_plaintext(policy_body.to_vec())
        .build(&public_key)
        .unwrap();

    // Get header
    let header = collection.header();

    // Verify policy binding is GMAC (SHA-256 last 8 bytes)
    let expected_hash = Sha256::digest(policy_body);
    let expected_binding = &expected_hash[24..]; // Last 8 bytes

    assert_eq!(
        header.policy.binding.len(),
        8,
        "GMAC binding should be 8 bytes"
    );
    assert_eq!(
        header.policy.binding, expected_binding,
        "Policy binding should match SHA-256 last 8 bytes"
    );
}

// ============================================================================
// Constants Verification Tests
// ============================================================================

#[test]
fn test_constants() {
    // Verify constants match spec
    assert_eq!(MAX_IV, 0x00FF_FFFF, "MAX_IV should be 2^24-1");
    assert_eq!(MAX_IV, 16_777_215, "MAX_IV should be 16,777,215 in decimal");
    assert_eq!(RESERVED_POLICY_IV, 0, "Reserved IV for policy should be 0");
}
