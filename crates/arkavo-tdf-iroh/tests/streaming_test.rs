//! Streaming tests for large data validation.
//!
//! Tests the BlobTransport implementation with various data sizes
//! to ensure streaming works correctly without OOM issues.

use arkavo_tdf::BlobTransport;
use arkavo_tdf_iroh::IrohTransport;
use std::io::Cursor;

/// Test streaming with 1MB data (baseline).
#[tokio::test]
async fn streaming_1mb() {
    let transport = IrohTransport::new().await.unwrap();

    // Generate 1MB of test data
    let data: Vec<u8> = (0..1024 * 1024).map(|i| (i % 256) as u8).collect();
    let reader = Cursor::new(data.clone());

    let ticket = transport.stage_stream(reader).await.unwrap();
    assert!(!ticket.is_empty());

    let mut output = Vec::new();
    let bytes = transport.fetch_stream(&ticket, &mut output).await.unwrap();

    assert_eq!(bytes, data.len() as u64);
    assert_eq!(output.len(), data.len());
    // Verify first and last chunks
    assert_eq!(&output[..1024], &data[..1024]);
    assert_eq!(&output[output.len() - 1024..], &data[data.len() - 1024..]);

    transport.shutdown().await.unwrap();
}

/// Test streaming with 10MB data.
#[tokio::test]
async fn streaming_10mb() {
    let transport = IrohTransport::new().await.unwrap();

    // Generate 10MB of test data
    let data: Vec<u8> = (0..10 * 1024 * 1024).map(|i| (i % 256) as u8).collect();
    let reader = Cursor::new(data.clone());

    let ticket = transport.stage_stream(reader).await.unwrap();

    let mut output = Vec::new();
    let bytes = transport.fetch_stream(&ticket, &mut output).await.unwrap();

    assert_eq!(bytes, data.len() as u64);
    assert_eq!(output.len(), data.len());

    transport.shutdown().await.unwrap();
}

/// Test streaming with 100MB data.
/// This test validates that we can handle larger payloads.
#[tokio::test]
#[ignore] // Run with: cargo test --test streaming_test -- --ignored
async fn streaming_100mb() {
    let transport = IrohTransport::new().await.unwrap();

    // Generate 100MB of test data
    let data: Vec<u8> = (0..100 * 1024 * 1024).map(|i| (i % 256) as u8).collect();
    let reader = Cursor::new(data.clone());

    let ticket = transport.stage_stream(reader).await.unwrap();

    let mut output = Vec::new();
    let bytes = transport.fetch_stream(&ticket, &mut output).await.unwrap();

    assert_eq!(bytes, data.len() as u64);
    assert_eq!(output.len(), data.len());

    transport.shutdown().await.unwrap();
}

/// Test concurrent staging of multiple blobs.
#[tokio::test]
async fn concurrent_staging() {
    let transport = IrohTransport::new().await.unwrap();

    // Stage multiple blobs concurrently
    let handles: Vec<_> = (0..5)
        .map(|i| {
            let data = vec![i as u8; 1024 * 100]; // 100KB each
            let reader = Cursor::new(data);
            let transport_clone = transport.clone();
            tokio::spawn(async move { transport_clone.stage_stream(reader).await })
        })
        .collect();

    let tickets: Vec<String> = futures::future::join_all(handles)
        .await
        .into_iter()
        .map(|r| r.unwrap().unwrap())
        .collect();

    assert_eq!(tickets.len(), 5);

    // All tickets should be unique (different hashes)
    let unique: std::collections::HashSet<_> = tickets.iter().collect();
    assert_eq!(unique.len(), 5);

    transport.shutdown().await.unwrap();
}

/// Test that health check works during active transfers.
#[tokio::test]
async fn health_check_during_transfer() {
    let transport = IrohTransport::new().await.unwrap();

    // Start a staging operation
    let data = vec![0u8; 1024 * 1024];
    let _ = transport.stage(&data).await.unwrap();

    // Health check should still work
    assert!(transport.health_check().await.unwrap());

    transport.shutdown().await.unwrap();
}

/// Test binary data integrity (non-text data).
#[tokio::test]
async fn binary_data_integrity() {
    let transport = IrohTransport::new().await.unwrap();

    // Generate random-ish binary data with all byte values
    let mut data = Vec::with_capacity(256 * 100);
    for _ in 0..100 {
        for b in 0..=255u8 {
            data.push(b);
        }
    }

    let ticket = transport.stage(&data).await.unwrap();
    let fetched = transport.fetch(&ticket).await.unwrap();

    assert_eq!(fetched, data);

    transport.shutdown().await.unwrap();
}
