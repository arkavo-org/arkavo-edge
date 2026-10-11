// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use std::sync::Arc;

use arkavo_tdf_iroh::{IrohNode, IrohNodeConfig};
use arkavo_test_macros::spec;

use super::*;
use crate::testing::generated;

async fn node(staged_limit: u64) -> IrohTransport {
    let node = Arc::new(IrohNode::new(IrohNodeConfig {
        staged_limit,
        ..IrohNodeConfig::default()
    }));
    node.start().await.unwrap();
    IrohTransport::new(node)
}

const QUICK: FetchPolicy = FetchPolicy {
    attempt_timeout: Duration::from_secs(5),
    backoff: Duration::from_millis(10),
};

/// ASSET-005, ASSET-007: staged assets come back byte for byte, checked
/// against the manifest, and the references stay inside the inline bound.
#[spec("ASSET-005", "ASSET-007")]
#[tokio::test]
async fn staged_assets_are_fetched_and_verified() {
    let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let assets = generated(src.path(), &["coin_000.png", "coin_001.png"]);
    let (provider, fetcher) = (node(1 << 20).await, node(1 << 20).await);

    let delivery = stage(&provider, &assets).await.unwrap();
    assert_eq!(delivery.references.len(), 2);
    assert!(delivery.references.iter().all(|r| r.media_type == PNG));
    assert!(serde_json::to_vec(&delivery).unwrap().len() <= INLINE_LIMIT);
    assert!(!serde_json::to_string(&delivery).unwrap().contains("base64"));

    let fetched = fetch(&fetcher, &delivery, dst.path(), QUICK).await;
    assert!(fetched.failed.is_empty(), "{:?}", fetched.failed);
    for (entry, path) in &fetched.saved {
        assert_eq!(
            std::fs::read(path).unwrap(),
            std::fs::read(src.path().join(&entry.name)).unwrap()
        );
    }
    provider.shutdown().await.unwrap();
    fetcher.shutdown().await.unwrap();
}

/// ASSET-007: a file that fails a check is reported and not written, and
/// the others are kept; an existing file is never overwritten (CLI-018).
#[spec("ASSET-007")]
#[tokio::test]
async fn failed_checks_keep_the_rest() {
    let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let assets = generated(src.path(), &["a.png", "b.png", "c.png"]);
    let (provider, fetcher) = (node(1 << 20).await, node(1 << 20).await);
    let mut delivery = stage(&provider, &assets).await.unwrap();
    delivery.manifest.assets[0].sha256 = "0".repeat(64);
    delivery.references[1].content_hash = "f".repeat(64);
    std::fs::write(dst.path().join("c.png"), b"mine").unwrap();

    let fetched = fetch(&fetcher, &delivery, dst.path(), QUICK).await;
    assert!(fetched.saved.is_empty());
    let reasons: Vec<String> = fetched.failed.iter().map(|(_, e)| e.to_string()).collect();
    assert!(reasons[0].contains("SHA-256"), "{reasons:?}");
    assert!(reasons[1].contains("different blob"), "{reasons:?}");
    assert!(
        reasons[2].contains("nothing was overwritten"),
        "{reasons:?}"
    );
    assert!(!dst.path().join("a.png").exists());
    assert!(!dst.path().join("b.png").exists());
    assert_eq!(std::fs::read(dst.path().join("c.png")).unwrap(), b"mine");
    provider.shutdown().await.unwrap();
    fetcher.shutdown().await.unwrap();
}

/// ASSET-007 edge: a producer that is gone is retried, then reported as
/// unreachable, naming the asset.
#[spec("ASSET-007")]
#[tokio::test]
async fn a_gone_producer_is_reported_unreachable() {
    let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let assets = generated(src.path(), &["coin_000.png"]);
    let (provider, fetcher) = (node(1 << 20).await, node(1 << 20).await);
    let delivery = stage(&provider, &assets).await.unwrap();
    provider.shutdown().await.unwrap();

    let fetched = fetch(&fetcher, &delivery, dst.path(), QUICK).await;
    let (name, error) = &fetched.failed[0];
    assert_eq!(name, "coin_000.png");
    assert_eq!(error.code(), "asset_unreachable");
    assert!(error.to_string().contains("3 attempts"), "{error}");
    fetcher.shutdown().await.unwrap();
}

/// ASSET-005 edge: staging past the data-plane bound fails with
/// data_plane_full (IROH-010).
#[spec("ASSET-005")]
#[tokio::test]
async fn staging_past_the_bound_is_data_plane_full() {
    let src = tempfile::tempdir().unwrap();
    let assets = generated(src.path(), &["coin_000.png"]);
    let provider = node(16).await;
    let error = stage(&provider, &assets).await.unwrap_err();
    assert_eq!(error.code(), "data_plane_full");
    provider.shutdown().await.unwrap();
}

/// PROTO-020: references that would not fit one inline message are refused
/// rather than sent.
#[spec("PROTO-020")]
#[tokio::test]
async fn references_past_the_inline_bound_are_refused() {
    let src = tempfile::tempdir().unwrap();
    let names: Vec<String> = (0..160).map(|i| format!("tile_{i:03}.png")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let assets = generated(src.path(), &names);
    let provider = node(1 << 24).await;
    let error = stage(&provider, &assets).await.unwrap_err();
    assert!(error.to_string().contains("inline bound"), "{error}");
    provider.shutdown().await.unwrap();
}

/// A file changed on disk after it was generated is not staged.
#[tokio::test]
async fn changed_files_are_not_staged() {
    let src = tempfile::tempdir().unwrap();
    let assets = generated(src.path(), &["coin_000.png"]);
    std::fs::write(src.path().join("coin_000.png"), b"not a png").unwrap();
    let provider = node(1 << 20).await;
    assert!(stage(&provider, &assets).await.is_err());
    assert_eq!(provider.node().staged_bytes(), 0);
    provider.shutdown().await.unwrap();
}
