use std::sync::Arc;

use arkavo_image_gen::testing::generated;
use arkavo_tdf_iroh::{IrohNode, IrohNodeConfig};
use arkavo_test_macros::spec;

use super::super::tests::{QUICK, transport};
use super::*;

/// ASSET-005, ASSET-007: assets staged with their manifest come back byte
/// for byte, each reference naming its size and media type, inside the
/// inline bound.
#[spec("ASSET-005", "ASSET-007")]
#[tokio::test]
async fn staged_assets_are_taken_back_verified() {
    let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let assets = generated(src.path(), &["coin_000.png", "coin_001.png"]);
    let (provider, requester) = (transport().await, transport().await);

    let delivery = stage(&provider, &assets).await.unwrap();
    for reference in &delivery.references {
        assert_eq!(reference.media_type, PNG);
        assert_eq!(reference.size, delivery.entry(reference).unwrap().bytes);
    }
    let part = delivery.part().unwrap();
    assert!(serde_json::to_vec(&part).unwrap().len() <= INLINE_LIMIT);
    assert_eq!(deliveries_in(&[part]), [delivery.clone()]);

    let fetched = take(&requester, &delivery, dst.path(), QUICK).await;
    assert!(fetched.failed.is_empty(), "{:?}", fetched.failed);
    for (entry, path) in &fetched.saved {
        assert_eq!(
            std::fs::read(path).unwrap(),
            std::fs::read(src.path().join(&entry.name)).unwrap()
        );
    }
    provider.shutdown().await.unwrap();
    requester.shutdown().await.unwrap();
}

/// ASSET-007: a file that fails a check is reported and not written, the
/// others are kept, and an existing file is never overwritten (CLI-018).
#[spec("ASSET-007", "CLI-018")]
#[tokio::test]
async fn failed_checks_keep_the_rest() {
    let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let assets = generated(src.path(), &["a.png", "b.png", "c.png", "d.png"]);
    let (provider, requester) = (transport().await, transport().await);
    let mut delivery = stage(&provider, &assets).await.unwrap();
    delivery.manifest.assets[0].sha256 = "0".repeat(64);
    delivery.references[1].content_hash = "f".repeat(64);
    delivery.references[2].name = "../escape.png".into();
    std::fs::write(dst.path().join("d.png"), b"mine").unwrap();

    let fetched = take(&requester, &delivery, dst.path(), QUICK).await;
    assert!(fetched.saved.is_empty());
    let reasons: Vec<String> = fetched.failed.iter().map(|(_, e)| e.to_string()).collect();
    assert!(reasons[0].contains("SHA-256"), "{reasons:?}");
    assert!(reasons[1].contains("different blob"), "{reasons:?}");
    assert!(reasons[2].contains("unsafe asset name"), "{reasons:?}");
    assert!(
        reasons[3].contains("nothing was overwritten"),
        "{reasons:?}"
    );
    assert_eq!(fetched.failed[1].1.code(), "asset_rejected");
    for name in ["a.png", "b.png", "escape.png"] {
        assert!(!dst.path().join(name).exists(), "{name}");
    }
    assert_eq!(std::fs::read(dst.path().join("d.png")).unwrap(), b"mine");
    provider.shutdown().await.unwrap();
    requester.shutdown().await.unwrap();
}

/// ASSET-007 edge: a producer that is gone is reported as unreachable,
/// naming the asset.
#[spec("ASSET-007")]
#[tokio::test]
async fn a_gone_producer_is_reported_unreachable() {
    let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let assets = generated(src.path(), &["coin_000.png"]);
    let (provider, requester) = (transport().await, transport().await);
    let delivery = stage(&provider, &assets).await.unwrap();
    provider.shutdown().await.unwrap();

    let fetched = take(&requester, &delivery, dst.path(), QUICK).await;
    let (name, error) = &fetched.failed[0];
    assert_eq!(name, "coin_000.png");
    assert_eq!(error.code(), "asset_unreachable");
    requester.shutdown().await.unwrap();
}

/// ASSET-005 edge: staging past the data-plane bound fails with
/// data_plane_full (IROH-011).
#[spec("ASSET-005")]
#[tokio::test]
async fn staging_past_the_bound_is_data_plane_full() {
    let src = tempfile::tempdir().unwrap();
    let assets = generated(src.path(), &["coin_000.png"]);
    let node = Arc::new(IrohNode::new(IrohNodeConfig {
        staged_limit: 16,
        ..IrohNodeConfig::default()
    }));
    node.start().await.unwrap();
    let provider = IrohTransport::new(node);
    let error = stage(&provider, &assets).await.unwrap_err();
    assert_eq!(error.code(), "data_plane_full");
    provider.shutdown().await.unwrap();
}

/// PROTO-020: references that would not fit one inline message are refused
/// rather than sent; a file changed on disk since it was generated is not
/// staged.
#[spec("PROTO-020", "ASSET-005")]
#[tokio::test]
async fn oversized_or_changed_deliveries_are_refused() {
    let src = tempfile::tempdir().unwrap();
    let names: Vec<String> = (0..160).map(|i| format!("tile_{i:03}.png")).collect();
    let names: Vec<&str> = names.iter().map(String::as_str).collect();
    let many = generated(src.path(), &names);
    let provider = transport().await;
    let error = stage(&provider, &many).await.unwrap_err();
    assert!(error.to_string().contains("inline bound"), "{error}");

    let other = tempfile::tempdir().unwrap();
    let changed = generated(other.path(), &["coin_000.png"]);
    std::fs::write(other.path().join("coin_000.png"), b"not a png").unwrap();
    let staged_before = provider.node().staged_bytes();
    assert!(stage(&provider, &changed).await.is_err());
    assert_eq!(provider.node().staged_bytes(), staged_before);
    provider.shutdown().await.unwrap();
}
