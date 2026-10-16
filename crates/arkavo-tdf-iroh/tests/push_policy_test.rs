//! A node serves blobs to peers but never lets them write into its store.
//!
//! iroh-blobs 0.103.0 never applied its push filter, so any peer that could
//! connect wrote blobs into a node built with the default event mask. This
//! pins the gate across iroh and iroh-blobs upgrades.

use arkavo_tdf_iroh::IrohNode;
use arkavo_test_macros::spec;
use iroh_blobs::api::blobs::BlobStatus;
use iroh_blobs::protocol::{ChunkRangesSeq, PushRequest};
use std::time::Duration;

#[spec("IROH-009")]
#[tokio::test]
async fn node_rejects_pushed_blobs() {
    let pusher = IrohNode::memory().await.unwrap();
    let target = IrohNode::memory().await.unwrap();

    let pusher_store = pusher.store().await.unwrap();
    let hash = pusher_store
        .add_bytes(vec![0x5a; 64 * 1024])
        .await
        .unwrap()
        .hash;

    let target_addr = target.endpoint().await.unwrap().addr();
    let conn = pusher
        .endpoint()
        .await
        .unwrap()
        .connect(target_addr, iroh_blobs::ALPN)
        .await
        .unwrap();

    // The pusher is not told whether the target refused, so the outcome is
    // judged from the target's store alone.
    pusher_store
        .remote()
        .execute_push(conn.clone(), PushRequest::new(hash, ChunkRangesSeq::root()))
        .await
        .ok();

    // `conn` stays open across the wait so a wrongly accepted push can land.
    // `observe` is not used to watch for it because that creates an empty
    // entry in the target's store.
    tokio::time::sleep(Duration::from_secs(1)).await;
    let target_store = target.store().await.unwrap();
    let status = target_store.blobs().status(hash).await.unwrap();
    assert!(
        matches!(status, BlobStatus::NotFound),
        "pushed blob reached the target's store: {status:?}"
    );

    drop(conn);
    pusher.stop().await.unwrap();
    target.stop().await.unwrap();
}
