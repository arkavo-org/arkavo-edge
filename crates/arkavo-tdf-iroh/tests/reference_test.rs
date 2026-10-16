//! A reference fetches exactly the data it names, or nothing (IROH-012).

// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use std::sync::Arc;
use std::time::Duration;

use arkavo_tdf_iroh::{DataRef, FetchPolicy, IrohError, IrohNode, IrohNodeConfig, IrohTransport};
use arkavo_test_macros::spec;

async fn node() -> IrohTransport {
    let node = Arc::new(IrohNode::new(IrohNodeConfig::default()));
    node.start().await.unwrap();
    IrohTransport::new(node)
}

const QUICK: FetchPolicy = FetchPolicy {
    attempt_timeout: Duration::from_secs(5),
    backoff: Duration::from_millis(10),
};

/// IROH-012: a staged reference names its size, hash and media type, and
/// fetches back the same bytes.
#[spec("IROH-012")]
#[tokio::test]
async fn references_fetch_what_they_name() {
    let (provider, fetcher) = (node().await, node().await);
    let reference = provider
        .stage_ref(
            "report.json",
            "application/json",
            b"{\"ok\":true}",
            Duration::from_mins(1),
        )
        .await
        .unwrap();
    assert_eq!(reference.size, 11);
    assert_eq!(reference.media_type, "application/json");
    assert_eq!(reference.content_hash.len(), 64);

    let json = serde_json::to_value(&reference).unwrap();
    let back: DataRef = serde_json::from_value(json).unwrap();
    assert_eq!(
        fetcher.fetch_ref(&back, QUICK).await.unwrap(),
        b"{\"ok\":true}"
    );
    provider.shutdown().await.unwrap();
    fetcher.shutdown().await.unwrap();
}

/// IROH-012: a provider offering more than the reference names is refused
/// before the blob is downloaded, and a ticket for another blob is refused
/// without contacting anyone.
#[spec("IROH-012")]
#[tokio::test]
async fn mismatched_references_are_refused() {
    let (provider, fetcher) = (node().await, node().await);
    let big = provider
        .stage_ref(
            "big.bin",
            "application/octet-stream",
            &vec![1u8; 1 << 20],
            Duration::from_mins(1),
        )
        .await
        .unwrap();

    let understated = DataRef {
        size: 16,
        ..big.clone()
    };
    let err = fetcher.fetch_ref(&understated, QUICK).await.unwrap_err();
    assert!(matches!(err, IrohError::RefMismatch { .. }), "{err}");
    assert!(err.to_string().contains("1048576 bytes offered"), "{err}");

    let other = DataRef {
        content_hash: "0".repeat(64),
        ..big
    };
    let err = fetcher.fetch_ref(&other, QUICK).await.unwrap_err();
    assert!(err.to_string().contains("different blob"), "{err}");
    provider.shutdown().await.unwrap();
    fetcher.shutdown().await.unwrap();
}

/// IROH-012 edge: a reference to a blob this node already holds is read
/// here, since iroh never connects a node to itself; its size is still
/// checked against the reference.
#[spec("IROH-012")]
#[tokio::test]
async fn references_held_here_are_read_here() {
    let node = node().await;
    let reference = node
        .stage_ref(
            "state.txt",
            "text/plain; charset=utf-8",
            b"turn 12",
            Duration::from_mins(1),
        )
        .await
        .unwrap();
    assert_eq!(node.fetch_ref(&reference, QUICK).await.unwrap(), b"turn 12");

    let understated = DataRef {
        size: 3,
        ..reference
    };
    let err = node.fetch_ref(&understated, QUICK).await.unwrap_err();
    assert!(matches!(err, IrohError::RefMismatch { .. }), "{err}");
    assert!(err.to_string().contains("7 bytes held"), "{err}");
    node.shutdown().await.unwrap();
}

/// IROH-012: a provider that is gone is tried again, then reported as
/// unreachable, naming the reference.
#[spec("IROH-012")]
#[tokio::test]
async fn a_gone_provider_is_unreachable() {
    let (provider, fetcher) = (node().await, node().await);
    let reference = provider
        .stage_ref(
            "coin_000.png",
            "image/png",
            b"\x89PNG not really",
            Duration::from_mins(1),
        )
        .await
        .unwrap();
    provider.shutdown().await.unwrap();

    let err = fetcher.fetch_ref(&reference, QUICK).await.unwrap_err();
    assert!(matches!(err, IrohError::Unreachable { .. }), "{err}");
    assert!(err.to_string().contains("coin_000.png"), "{err}");
    assert!(err.to_string().contains("3 attempts"), "{err}");
    fetcher.shutdown().await.unwrap();
}

/// IROH-011: staging a reference past the bound fails without staging.
#[spec("IROH-011")]
#[tokio::test]
async fn references_respect_the_staging_bound() {
    let node = Arc::new(IrohNode::new(IrohNodeConfig {
        staged_limit: 8,
        ..IrohNodeConfig::default()
    }));
    node.start().await.unwrap();
    let provider = IrohTransport::new(node);
    let err = provider
        .stage_ref(
            "big.bin",
            "application/octet-stream",
            &[0u8; 64],
            Duration::from_mins(1),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, IrohError::StoreFull { .. }), "{err}");
    provider.shutdown().await.unwrap();
}
