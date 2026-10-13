// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use super::*;
use crate::catalog::SetLabel;
use crate::testing::serve_hub_file;
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use wiremock::MockServer;

const REVISION: &str = "0000000000000000000000000000000000000000";

fn pinned(path: &'static str, body: &[u8]) -> &'static PinnedFile {
    Box::leak(Box::new(PinnedFile {
        repo: "test/repo",
        revision: REVISION,
        path,
        size: body.len() as u64,
        sha256: Box::leak(hex::encode(Sha256::digest(body)).into_boxed_str()),
    }))
}

fn set(
    diffusion: &'static PinnedFile,
    encoder: &'static PinnedFile,
    vae: &'static PinnedFile,
) -> &'static WeightSet {
    Box::leak(Box::new(WeightSet {
        label: SetLabel::S,
        diffusion,
        text_encoder: encoder,
        vae,
    }))
}

/// A store whose weights go in a Hugging Face cache under `dir`.
fn store_in(dir: &Path, hub: String) -> Store {
    Store::with_hub(dir.join("image-gen"), Cache::new(dir.join("hub")), hub)
}

/// IMG-004/IMG-005: a set is fetched through hf-hub into the Hugging Face
/// cache once per distinct file, at the pinned commit, with progress as
/// bytes fetched of bytes needed.
#[arkavo_test_macros::spec("IMG-004", "IMG-005")]
#[tokio::test]
async fn sets_fetch_each_distinct_file_once_into_the_hub_cache() {
    const D: &[u8] = b"diffusion-weights";
    const E: &[u8] = b"encoder-and-vae";
    let diffusion = pinned("d.gguf", D);
    let shared = pinned("vae/shared.safetensors", E);
    let weights = set(diffusion, shared, shared);
    let server = MockServer::start().await;
    serve_hub_file(&server, diffusion, D, 1, Duration::ZERO).await;
    serve_hub_file(&server, shared, E, 1, Duration::ZERO).await;

    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path(), server.uri());
    assert_eq!(store.missing_bytes(weights), (D.len() + E.len()) as u64);

    let seen = Mutex::new(Vec::new());
    let record = |fetched, needed| seen.lock().unwrap().push((fetched, needed));
    let paths = store.fetch_set(weights, &record).await.unwrap();
    assert_eq!(std::fs::read(&paths.diffusion).unwrap(), D);
    assert_eq!(paths.text_encoder, paths.vae);
    let snapshot = dir
        .path()
        .join("hub/models--test--repo/snapshots")
        .join(REVISION);
    assert_eq!(paths.vae, snapshot.join("vae/shared.safetensors"));
    let blob = std::fs::canonicalize(&paths.diffusion).unwrap();
    assert!(
        blob.ends_with(format!("blobs/{}", diffusion.sha256)),
        "{}",
        blob.display()
    );
    assert!(
        !dir.path().join("image-gen").exists(),
        "weights stay out of the generator's own directory"
    );
    assert_eq!(store.missing_bytes(weights), 0);
    let total = (D.len() + E.len()) as u64;
    assert_eq!(seen.lock().unwrap().last().copied(), Some((total, total)));

    // A second fetch finds everything present and reports nothing to do.
    store
        .fetch_set(weights, &|_, needed| assert_eq!(needed, 0))
        .await
        .unwrap();
}

/// IMG-004: no Hugging Face token is sent, even one saved in the cache.
#[arkavo_test_macros::spec("IMG-004")]
#[tokio::test]
async fn a_saved_token_is_not_sent() {
    const D: &[u8] = b"weights";
    let file = pinned("t.gguf", D);
    let server = MockServer::start().await;
    serve_hub_file(&server, file, D, 1, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("hub")).unwrap();
    std::fs::write(dir.path().join("hub/token"), "hf_secret").unwrap();

    let store = store_in(dir.path(), server.uri());
    store
        .fetch_set(set(file, file, file), &|_, _| {})
        .await
        .unwrap();
    let requests = server.received_requests().await.unwrap();
    assert!(!requests.is_empty());
    for request in requests {
        assert!(
            !request.headers.contains_key("authorization"),
            "{:?}",
            request.headers
        );
    }
}

/// IMG-004 regression: a file in the cache whose bytes do not match its pin
/// (written there by another client, or changed on disk) is never handed
/// to the runtime; link and blob are removed so the next request fetches it.
#[cfg(unix)]
#[arkavo_test_macros::spec("IMG-004")]
#[tokio::test]
async fn a_cached_file_that_fails_its_hash_is_removed() {
    let file = pinned("bad.gguf", b"the-pinned-bytes");
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path(), "http://127.0.0.1:9".into());
    let link = store.model_path(file);
    let blob = store.blob_path(file);
    std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::fs::create_dir_all(blob.parent().unwrap()).unwrap();
    std::fs::write(&blob, b"other-bytes-same").unwrap();
    std::os::unix::fs::symlink(&blob, &link).unwrap();
    assert_eq!(store.missing_bytes(set(file, file, file)), 0);

    let err = store
        .fetch_set(set(file, file, file), &|_, _| {})
        .await
        .unwrap_err();
    assert!(matches!(err, ImageGenError::HashMismatch { .. }), "{err}");
    assert!(std::fs::symlink_metadata(&link).is_err(), "link removed");
    assert!(!blob.exists(), "blob removed");
    assert_eq!(store.missing_bytes(set(file, file, file)), file.size);
}

/// IMG-004: when the Hub offers another file than the pin, nothing is
/// downloaded.
#[arkavo_test_macros::spec("IMG-004")]
#[tokio::test]
async fn another_file_on_offer_is_refused_before_download() {
    let file = pinned("swapped.gguf", b"pinned-weights");
    let server = MockServer::start().await;
    serve_hub_file(&server, file, b"different-weights!", 0, Duration::ZERO).await;
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path(), server.uri());
    let err = store
        .fetch_set(set(file, file, file), &|_, _| {})
        .await
        .unwrap_err();
    assert!(matches!(err, ImageGenError::SizeMismatch { .. }), "{err}");
    assert!(std::fs::symlink_metadata(store.model_path(file)).is_err());
}

/// IMG-005: hf-hub's partial download counts what it has written in order,
/// so progress and `model list` count only what it still lacks.
#[arkavo_test_macros::spec("IMG-005")]
#[test]
fn partial_downloads_count_what_they_lack() {
    let file = pinned("p.gguf", b"0123456789");
    let weights = set(file, file, file);
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path(), HUB.into());
    let mut part = store.blob_path(file);
    part.set_extension("sync.part");
    std::fs::create_dir_all(part.parent().unwrap()).unwrap();
    let mut bytes = b"0123\0\0\0\0\0\0".to_vec();
    bytes.extend_from_slice(&4u64.to_le_bytes());
    std::fs::write(&part, bytes).unwrap();
    assert_eq!(store.missing_bytes(weights), 6);

    // A part file of another length is not hf-hub's; it starts over.
    std::fs::write(&part, b"0123").unwrap();
    assert_eq!(store.missing_bytes(weights), 10);
}

/// IMG-020: runtimes no pin names are deleted; the current runtime, its
/// partial download, lock files and the Hugging Face cache stay.
#[arkavo_test_macros::spec("IMG-020")]
#[test]
fn prune_removes_old_runtimes_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = store_in(dir.path(), HUB.into());
    let runtime = store.runtime_dir();
    std::fs::create_dir_all(runtime.join("master-956-1b0ba10")).unwrap();
    std::fs::create_dir_all(runtime.join("master-900-aaaaaaa")).unwrap();
    std::fs::write(
        runtime.join("master-900-aaaaaaa").join("sd-cli"),
        b"1234567",
    )
    .unwrap();
    std::fs::write(runtime.join("master-956-1b0ba10.zip.part"), b"z").unwrap();
    std::fs::write(store.root().join(".generate.lock"), b"").unwrap();
    let foreign = dir.path().join("hub/models--other--model/blobs/abc");
    std::fs::create_dir_all(foreign.parent().unwrap()).unwrap();
    std::fs::write(&foreign, b"someone else's weights").unwrap();

    let pruned = store.prune("master-956-1b0ba10").unwrap();
    assert_eq!(pruned.names, ["master-900-aaaaaaa"]);
    assert_eq!(pruned.bytes, 7);
    assert!(
        store.root().join(".generate.lock").exists(),
        "lock files stay"
    );
    assert!(runtime.join("master-956-1b0ba10").exists());
    assert!(runtime.join("master-956-1b0ba10.zip.part").exists());
    assert!(foreign.exists(), "the Hugging Face cache is not touched");

    // Nothing left to prune, and a missing directory is not an error.
    assert_eq!(
        store.prune("master-956-1b0ba10").unwrap(),
        Pruned::default()
    );
    let empty = store_in(&dir.path().join("absent"), HUB.into());
    assert_eq!(empty.prune("x").unwrap(), Pruned::default());
}

#[arkavo_test_macros::spec("IMG-004")]
#[test]
fn default_root_is_under_the_data_dir() {
    if let Some(root) = Store::default_root() {
        assert!(root.ends_with("arkavo/image-gen"));
    }
}
