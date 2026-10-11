//! Blobs staged for another party live for their retention window, within
//! a bound on staged bytes (IROH-009, IROH-010).

// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use std::sync::Arc;
use std::time::Duration;

use arkavo_tdf_iroh::{IrohError, IrohNode, IrohNodeConfig, IrohTransport};
use arkavo_test_macros::spec;

async fn node(staged_limit: u64) -> IrohTransport {
    let config = IrohNodeConfig {
        gc_interval: Duration::from_millis(100),
        staged_limit,
        ..IrohNodeConfig::default()
    };
    let node = Arc::new(IrohNode::new(config));
    node.start().await.unwrap();
    IrohTransport::new(node)
}

/// IROH-009: another node fetches a retained blob inside its window; after
/// the window the blob is gone from the store that staged it.
#[spec("IROH-009")]
#[tokio::test]
async fn retained_blobs_expire_after_their_window() {
    let provider = node(1 << 20).await;
    let fetcher = node(1 << 20).await;
    let ticket = provider
        .stage_retained(b"sprite bytes", Duration::from_millis(300))
        .await
        .unwrap();
    assert_eq!(provider.node().staged_bytes(), 12);
    assert_eq!(fetcher.fetch_bytes(&ticket).await.unwrap(), b"sprite bytes");

    tokio::time::sleep(Duration::from_millis(900)).await;
    assert_eq!(provider.node().staged_bytes(), 0);
    let late = node(1 << 20).await;
    let outcome = tokio::time::timeout(Duration::from_secs(10), late.fetch_bytes(&ticket)).await;
    assert!(
        !matches!(outcome, Ok(Ok(_))),
        "an expired blob was still served"
    );
    for transport in [provider, fetcher, late] {
        transport.shutdown().await.unwrap();
    }
}

/// IROH-010: staging past the bound fails, and blobs still in their window
/// are not evicted to make room.
#[spec("IROH-010")]
#[tokio::test]
async fn staging_past_the_bound_fails_without_evicting() {
    let provider = node(16).await;
    let fetcher = node(1 << 20).await;
    let kept = provider
        .stage_retained(b"0123456789", Duration::from_mins(1))
        .await
        .unwrap();
    let err = provider
        .stage_retained(b"0123456789", Duration::from_mins(1))
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            IrohError::StoreFull {
                staged: 10,
                requested: 10,
                limit: 16
            }
        ),
        "{err}"
    );
    assert_eq!(fetcher.fetch_bytes(&kept).await.unwrap(), b"0123456789");
    provider.shutdown().await.unwrap();
    fetcher.shutdown().await.unwrap();
}

/// Fetched blobs carry no tag; the temp tag held during the fetch keeps a
/// collection that runs mid-read from taking them.
#[tokio::test]
async fn fetches_survive_collection() {
    let provider = node(8 << 20).await;
    let fetcher = node(1 << 20).await;
    let ticket = provider
        .stage_retained(&vec![7u8; 4 << 20], Duration::from_mins(1))
        .await
        .unwrap();
    for _ in 0..3 {
        assert_eq!(fetcher.fetch_bytes(&ticket).await.unwrap().len(), 4 << 20);
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    provider.shutdown().await.unwrap();
    fetcher.shutdown().await.unwrap();
}
