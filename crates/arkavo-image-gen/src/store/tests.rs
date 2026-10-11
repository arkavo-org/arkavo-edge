// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use super::*;
use crate::catalog::SetLabel;
use sha2::{Digest, Sha256};
use std::sync::Mutex;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn pinned(path: &'static str, body: &[u8]) -> &'static PinnedFile {
    Box::leak(Box::new(PinnedFile {
        repo: "test/repo",
        revision: "0000000000000000000000000000000000000000",
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

async fn serve(server: &MockServer, file: &PinnedFile, body: &'static [u8], times: u64) {
    Mock::given(method("GET"))
        .and(path(format!(
            "/{}/resolve/{}/{}",
            file.repo, file.revision, file.path
        )))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .expect(times)
        .mount(server)
        .await;
}

/// IMG-004/IMG-005: a set is fetched once per distinct file, with progress
/// as bytes fetched of bytes needed.
#[arkavo_test_macros::spec("IMG-004", "IMG-005")]
#[tokio::test]
async fn sets_fetch_each_distinct_file_once() {
    const D: &[u8] = b"diffusion-weights";
    const E: &[u8] = b"encoder-and-vae";
    let diffusion = pinned("d.gguf", D);
    let shared = pinned("shared.safetensors", E);
    let weights = set(diffusion, shared, shared);
    let server = MockServer::start().await;
    serve(&server, diffusion, D, 1).await;
    serve(&server, shared, E, 1).await;

    let dir = tempfile::tempdir().unwrap();
    let store = Store::with_hub(dir.path().to_path_buf(), server.uri());
    assert_eq!(store.missing_bytes(weights), (D.len() + E.len()) as u64);

    let seen = Mutex::new(Vec::new());
    let record = |fetched, needed| seen.lock().unwrap().push((fetched, needed));
    let paths = store
        .fetch_set(&Fetcher::without_backoff(), weights, &record)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&paths.diffusion).unwrap(), D);
    assert_eq!(paths.text_encoder, paths.vae);
    assert_eq!(store.missing_bytes(weights), 0);
    let total = (D.len() + E.len()) as u64;
    assert_eq!(seen.lock().unwrap().last().copied(), Some((total, total)));

    // A second fetch finds everything present and reports nothing to do.
    store
        .fetch_set(&Fetcher::without_backoff(), weights, &|_, needed| {
            assert_eq!(needed, 0);
        })
        .await
        .unwrap();
}

#[arkavo_test_macros::spec("IMG-005")]
#[test]
fn partial_downloads_count_what_they_lack() {
    let file = pinned("p.gguf", b"0123456789");
    let weights = set(file, file, file);
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    std::fs::create_dir_all(store.models_dir()).unwrap();
    let mut part = store.model_path(file).into_os_string();
    part.push(".part");
    std::fs::write(part, b"0123").unwrap();
    assert_eq!(store.missing_bytes(weights), 6);
}

/// IMG-020: files no pin names are deleted; pinned ones, partial downloads
/// of pinned files and the current runtime stay.
#[arkavo_test_macros::spec("IMG-020")]
#[test]
fn prune_keeps_only_what_pins_name() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let models = store.models_dir();
    let runtime = store.runtime_dir();
    for d in [&models, &runtime] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(models.join("keep.gguf"), b"k").unwrap();
    std::fs::write(models.join("keep2.gguf.part"), b"kk").unwrap();
    std::fs::write(models.join("old.gguf"), b"12345").unwrap();
    std::fs::write(models.join("old.gguf.part"), b"123").unwrap();
    std::fs::create_dir_all(runtime.join("master-956-1b0ba10")).unwrap();
    std::fs::create_dir_all(runtime.join("master-900-aaaaaaa")).unwrap();
    std::fs::write(
        runtime.join("master-900-aaaaaaa").join("sd-cli"),
        b"1234567",
    )
    .unwrap();
    std::fs::write(runtime.join("master-956-1b0ba10.zip.part"), b"z").unwrap();
    std::fs::write(dir.path().join(".generate.lock"), b"").unwrap();

    let keep = ["keep.gguf".to_string(), "keep2.gguf".to_string()]
        .into_iter()
        .collect();
    let mut pruned = store.prune(&keep, "master-956-1b0ba10").unwrap();
    pruned.names.sort();
    assert_eq!(
        pruned.names,
        ["master-900-aaaaaaa", "old.gguf", "old.gguf.part"]
    );
    assert_eq!(pruned.bytes, 5 + 3 + 7);
    assert!(
        dir.path().join(".generate.lock").exists(),
        "lock files stay"
    );
    assert!(models.join("keep.gguf").exists());
    assert!(models.join("keep2.gguf.part").exists());
    assert!(runtime.join("master-956-1b0ba10").exists());
    assert!(runtime.join("master-956-1b0ba10.zip.part").exists());

    // Nothing left to prune, and a missing store is not an error.
    assert_eq!(
        store.prune(&keep, "master-956-1b0ba10").unwrap(),
        Pruned::default()
    );
    let empty = Store::new(dir.path().join("absent"));
    assert_eq!(empty.prune(&keep, "x").unwrap(), Pruned::default());
}

#[arkavo_test_macros::spec("IMG-004")]
#[test]
fn default_root_is_under_the_data_dir() {
    if let Some(root) = Store::default_root() {
        assert!(root.ends_with("arkavo/image-gen"));
    }
}
