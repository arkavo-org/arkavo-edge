//! Integration tests for OpenTDF encryption + Iroh transport.
//!
//! Demonstrates the complete secure data sharing workflow:
//! 1. Encrypt data with real AES-256-GCM (via opentdf-rs)
//! 2. Transport encrypted manifest via Iroh P2P
//! 3. Fetch and verify the encrypted data

use arkavo_tdf::{BlobTransport, OpenTdfService, PolicyBuilder, TdfEncryptor};
use arkavo_tdf_iroh::IrohTransport;

/// Test full encryption + transport workflow.
#[tokio::test]
async fn opentdf_encrypt_iroh_transport() {
    // Create encryption service with KAS URL
    let tdf_service = OpenTdfService::with_kas_url("https://kas.arkavo.net");

    // Create Iroh transport
    let transport = IrohTransport::new().await.unwrap();

    // Create policy with attributes
    let policy = PolicyBuilder::new()
        .attribute("https://arkavo.net/attr/classification", &["confidential"])
        .attribute("https://arkavo.net/attr/department", &["engineering"])
        .build()
        .unwrap();

    // Original sensitive data
    let plaintext = b"Project Roadmap: Q4 2025 deliverables include...";

    // Encrypt the data with real AES-256-GCM
    let manifest = tdf_service.encrypt(plaintext, &policy).await.unwrap();

    // Verify encryption metadata
    assert_eq!(manifest.version, "3.0.0");
    assert_eq!(
        manifest.encryption_information.method.algorithm,
        "AES-256-GCM"
    );
    assert_eq!(manifest.payload.payload_type, "inline");

    // Serialize manifest to JSON
    let manifest_json = serde_json::to_vec(&manifest).unwrap();

    // Stage encrypted manifest via Iroh
    let ticket = transport.stage(&manifest_json).await.unwrap();
    assert!(!ticket.is_empty());

    // Fetch manifest from Iroh
    let fetched_json = transport.fetch(&ticket).await.unwrap();

    // Deserialize and verify
    let fetched_manifest: arkavo_tdf::TdfManifest =
        serde_json::from_slice(&fetched_json).unwrap();

    assert_eq!(fetched_manifest.version, manifest.version);
    assert_eq!(fetched_manifest.payload.value, manifest.payload.value);
    assert_eq!(
        fetched_manifest.encryption_information.policy,
        manifest.encryption_information.policy
    );

    transport.shutdown().await.unwrap();
}

/// Test streaming encryption with Iroh transport.
#[tokio::test]
async fn opentdf_stream_encrypt_iroh_transport() {
    use std::io::Cursor;

    let tdf_service = OpenTdfService::with_kas_url("https://kas.example.com");
    let transport = IrohTransport::new().await.unwrap();

    let policy = PolicyBuilder::new()
        .attribute("https://arkavo.net/attr/type", &["document"])
        .build()
        .unwrap();

    // Larger data for streaming
    let plaintext: Vec<u8> = (0..10000).map(|i| (i % 256) as u8).collect();
    let reader = Cursor::new(plaintext.clone());

    // Stream encrypt
    let manifest = tdf_service.encrypt_stream(reader, &policy).await.unwrap();

    // Stage via streaming
    let manifest_json = serde_json::to_vec(&manifest).unwrap();
    let reader = Cursor::new(manifest_json.clone());
    let ticket = transport.stage_stream(reader).await.unwrap();

    // Fetch via streaming
    let mut output = Vec::new();
    let bytes = transport.fetch_stream(&ticket, &mut output).await.unwrap();

    assert_eq!(bytes, manifest_json.len() as u64);
    assert_eq!(output, manifest_json);

    transport.shutdown().await.unwrap();
}

/// Test multiple encrypted blobs in sequence.
#[tokio::test]
async fn multiple_encrypted_blobs() {
    let tdf_service = OpenTdfService::with_kas_url("https://kas.example.com");
    let transport = IrohTransport::new().await.unwrap();

    let documents = vec![
        ("doc1", "First document content"),
        ("doc2", "Second document content"),
        ("doc3", "Third document content"),
    ];

    let mut tickets = Vec::new();

    // Encrypt and stage each document
    for (name, content) in &documents {
        let policy = PolicyBuilder::new()
            .attribute("https://arkavo.net/attr/docname", &[name])
            .build()
            .unwrap();

        let manifest = tdf_service
            .encrypt(content.as_bytes(), &policy)
            .await
            .unwrap();
        let manifest_json = serde_json::to_vec(&manifest).unwrap();
        let ticket = transport.stage(&manifest_json).await.unwrap();
        tickets.push(ticket);
    }

    // Verify all tickets are unique (different encrypted content)
    let unique: std::collections::HashSet<_> = tickets.iter().collect();
    assert_eq!(unique.len(), documents.len());

    // Fetch each and verify structure
    for ticket in &tickets {
        let fetched = transport.fetch(ticket).await.unwrap();
        let manifest: arkavo_tdf::TdfManifest = serde_json::from_slice(&fetched).unwrap();
        assert_eq!(manifest.version, "3.0.0");
        assert!(!manifest.payload.value.is_empty());
    }

    transport.shutdown().await.unwrap();
}

/// Test policy with dissemination controls.
#[tokio::test]
async fn policy_with_dissemination() {
    let tdf_service = OpenTdfService::with_kas_url("https://kas.example.com");
    let transport = IrohTransport::new().await.unwrap();

    let policy = PolicyBuilder::new()
        .attribute("https://arkavo.net/attr/clearance", &["secret"])
        .dissemination(&["user@company.com", "team@company.com"])
        .build()
        .unwrap();

    let manifest = tdf_service
        .encrypt(b"Classified information", &policy)
        .await
        .unwrap();

    // Stage and verify
    let manifest_json = serde_json::to_vec(&manifest).unwrap();
    let ticket = transport.stage(&manifest_json).await.unwrap();

    let fetched = transport.fetch(&ticket).await.unwrap();
    let fetched_manifest: arkavo_tdf::TdfManifest = serde_json::from_slice(&fetched).unwrap();

    // Verify policy is preserved
    assert_eq!(
        fetched_manifest.encryption_information.policy,
        manifest.encryption_information.policy
    );

    transport.shutdown().await.unwrap();
}
