// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use super::*;
use sha2::{Digest, Sha256};
use std::io::Write;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MEMBERS: &[&str] = &["sd-cli", "libstable-diffusion.dylib", "ggml.txt"];

fn archive(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut buffer);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);
    for (name, body) in entries {
        zip.start_file(*name, options).unwrap();
        zip.write_all(body).unwrap();
    }
    zip.finish().unwrap();
    buffer.into_inner()
}

fn release() -> Vec<u8> {
    archive(&[
        ("ggml.txt", b"MIT"),
        ("libstable-diffusion.dylib", b"dylib-bytes"),
        ("sd-cli", b"#!/bin/sh\n"),
        ("sd-server", b"not extracted"),
    ])
}

fn pin(server: &MockServer, body: &[u8]) -> &'static RuntimePin {
    Box::leak(Box::new(RuntimePin {
        build: 956,
        tag: "master-956-test",
        url: Box::leak(format!("{}/release.zip", server.uri()).into_boxed_str()),
        size: body.len() as u64,
        sha256: Box::leak(hex::encode(Sha256::digest(body)).into_boxed_str()),
        members: MEMBERS,
        cli: "sd-cli",
    }))
}

async fn serving(body: Vec<u8>, times: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/release.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(body))
        .expect(times)
        .mount(&server)
        .await;
    server
}

/// IMG-003: only listed members are extracted, recorded and executable.
#[arkavo_test_macros::spec("IMG-003")]
#[tokio::test]
async fn installs_listed_members_only() {
    let body = release();
    let server = serving(body.clone(), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let runtime = install(&store, &Fetcher::without_backoff(), pin(&server, &body))
        .await
        .unwrap();
    assert_eq!(runtime.cli(), runtime.dir().join("sd-cli"));
    assert_eq!(std::fs::read(runtime.cli()).unwrap(), b"#!/bin/sh\n");
    assert!(!runtime.dir().join("sd-server").exists());
    assert!(!store.runtime_dir().join("master-956-test.zip").exists());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |name: &str| {
            std::fs::metadata(runtime.dir().join(name))
                .unwrap()
                .permissions()
                .mode()
                & 0o777
        };
        assert_eq!(mode("sd-cli"), 0o755);
        assert_eq!(mode("ggml.txt"), 0o644);
    }
    runtime.verify().await.unwrap();

    // Installed and intact: nothing is fetched again (the mock expects one GET).
    let again = install(&store, &Fetcher::without_backoff(), pin(&server, &body))
        .await
        .unwrap();
    assert_eq!(again.cli(), runtime.cli());
}

/// IMG-003: a changed file fails verification and is reinstalled.
#[arkavo_test_macros::spec("IMG-003")]
#[tokio::test]
async fn tampered_installs_are_replaced() {
    let body = release();
    let server = serving(body.clone(), 2).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let pin = pin(&server, &body);
    let runtime = install(&store, &Fetcher::without_backoff(), pin)
        .await
        .unwrap();
    std::fs::write(
        runtime.dir().join("libstable-diffusion.dylib"),
        b"dylib-bytez",
    )
    .unwrap();
    let err = runtime.verify().await.unwrap_err();
    assert!(matches!(err, ImageGenError::HashMismatch { .. }), "{err}");
    std::fs::write(runtime.cli(), b"#!/bin/sh\necho changed\n").unwrap();
    assert!(matches!(
        runtime.verify().await.unwrap_err(),
        ImageGenError::SizeMismatch { .. }
    ));
    let fresh = install(&store, &Fetcher::without_backoff(), pin)
        .await
        .unwrap();
    fresh.verify().await.unwrap();
    assert_eq!(std::fs::read(fresh.cli()).unwrap(), b"#!/bin/sh\n");
}

#[arkavo_test_macros::spec("IMG-003")]
#[tokio::test]
async fn entries_with_paths_fail_the_install() {
    let body = archive(&[
        ("sd-cli", b"x"),
        ("libstable-diffusion.dylib", b"y"),
        ("ggml.txt", b"z"),
        ("../escape", b"w"),
    ]);
    let server = serving(body.clone(), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let err = install(&store, &Fetcher::without_backoff(), pin(&server, &body))
        .await
        .unwrap_err();
    assert_eq!(err.code(), "runtime_install_failed");
    assert!(err.to_string().contains("names a path"), "{err}");
    assert!(!dir.path().join("escape").exists());
    assert!(!store.runtime_dir().join("master-956-test").exists());
}

#[arkavo_test_macros::spec("IMG-003")]
#[tokio::test]
async fn missing_members_fail_the_install() {
    let body = archive(&[("sd-cli", b"x"), ("ggml.txt", b"z")]);
    let server = serving(body.clone(), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let err = install(&store, &Fetcher::without_backoff(), pin(&server, &body))
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("libstable-diffusion.dylib"),
        "{err}"
    );
}

/// IMG-003 edge: an archive with the wrong hash is deleted and nothing is
/// extracted.
#[arkavo_test_macros::spec("IMG-003")]
#[tokio::test]
async fn archive_hash_is_checked_before_extracting() {
    let body = release();
    let server = serving(body.clone(), 1).await;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path().to_path_buf());
    let mut wrong = *pin(&server, &body);
    wrong.sha256 = "0000000000000000000000000000000000000000000000000000000000000000";
    let err = install(&store, &Fetcher::without_backoff(), &wrong)
        .await
        .unwrap_err();
    assert!(matches!(err, ImageGenError::HashMismatch { .. }), "{err}");
    assert!(!store.runtime_dir().join("master-956-test").exists());
    assert!(!store.runtime_dir().join("master-956-test.zip").exists());
}
