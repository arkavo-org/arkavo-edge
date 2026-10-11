// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use super::*;
use std::sync::Mutex;
use wiremock::matchers::{header, header_exists, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const BODY: &[u8] = b"0123456789abcdefghij";

fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

struct Seen(Mutex<Vec<u64>>);

impl Seen {
    fn new() -> Self {
        Self(Mutex::new(Vec::new()))
    }
    fn last(&self) -> Option<u64> {
        self.0.lock().unwrap().last().copied()
    }
}

async fn fetch(server: &MockServer, dest: &Path, sha: &str, size: u64, seen: &Seen) -> Result<()> {
    let url = format!("{}/f", server.uri());
    let pin = Pin {
        url: &url,
        size,
        sha256: sha,
    };
    let record = |n: u64| seen.0.lock().unwrap().push(n);
    Fetcher::without_backoff().fetch(pin, dest, &record).await
}

/// IMG-004: a fresh download is hashed, then renamed into place.
#[arkavo_test_macros::spec("IMG-004")]
#[tokio::test]
async fn downloads_verify_then_rename() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("w.gguf");
    let seen = Seen::new();
    fetch(&server, &dest, &digest(BODY), BODY.len() as u64, &seen)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), BODY);
    assert!(!dir.path().join("w.gguf.part").exists());
    assert_eq!(seen.last(), Some(BODY.len() as u64));
}

/// IMG-004: a partial file resumes with a Range request.
#[arkavo_test_macros::spec("IMG-004")]
#[tokio::test]
async fn partial_downloads_resume_with_range() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .and(header("range", "bytes=8-"))
        .respond_with(ResponseTemplate::new(206).set_body_bytes(&BODY[8..]))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("w.gguf");
    std::fs::write(dir.path().join("w.gguf.part"), &BODY[..8]).unwrap();
    let seen = Seen::new();
    fetch(&server, &dest, &digest(BODY), BODY.len() as u64, &seen)
        .await
        .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), BODY);
    assert_eq!(seen.0.lock().unwrap().first().copied(), Some(8));
}

/// A server that ignores the range sends the whole file, which replaces
/// the partial one rather than being appended to it.
#[arkavo_test_macros::spec("IMG-004")]
#[tokio::test]
async fn ignored_ranges_restart_the_file() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .and(header_exists("range"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("w.gguf");
    std::fs::write(dir.path().join("w.gguf.part"), b"stale").unwrap();
    fetch(
        &server,
        &dest,
        &digest(BODY),
        BODY.len() as u64,
        &Seen::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&dest).unwrap(), BODY);
}

/// IMG-004 edge: a wrong hash deletes the file and fails, with no retry.
#[arkavo_test_macros::spec("IMG-004")]
#[tokio::test]
async fn hash_mismatch_deletes_and_fails() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("w.gguf");
    let err = fetch(
        &server,
        &dest,
        &digest(b"other"),
        BODY.len() as u64,
        &Seen::new(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ImageGenError::HashMismatch { .. }), "{err}");
    assert!(!dest.exists());
    assert!(!dir.path().join("w.gguf.part").exists());
}

/// A body longer than the pin is refused and discarded at once.
#[arkavo_test_macros::spec("IMG-004")]
#[tokio::test]
async fn oversized_bodies_are_refused() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(BODY))
        .expect(1)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("w.gguf");
    let err = fetch(&server, &dest, &digest(BODY), 10, &Seen::new())
        .await
        .unwrap_err();
    assert!(matches!(err, ImageGenError::SizeMismatch { .. }), "{err}");
    assert!(!dir.path().join("w.gguf.part").exists());
}

/// Errors from the server are retried, then reported.
#[arkavo_test_macros::spec("IMG-004")]
#[tokio::test]
async fn server_errors_are_retried_then_reported() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/f"))
        .respond_with(ResponseTemplate::new(503))
        .expect(u64::from(ATTEMPTS))
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let err = fetch(
        &server,
        &dir.path().join("w"),
        &digest(BODY),
        20,
        &Seen::new(),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("503"), "{err}");
}

/// A file at its final name with the pinned size is not fetched again.
#[arkavo_test_macros::spec("IMG-004")]
#[tokio::test]
async fn present_files_are_not_fetched() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path().join("w.gguf");
    std::fs::write(&dest, BODY).unwrap();
    let seen = Seen::new();
    fetch(&server, &dest, &digest(BODY), BODY.len() as u64, &seen)
        .await
        .unwrap();
    assert_eq!(seen.last(), Some(BODY.len() as u64));
}

/// IMG-004 edge: refused before any byte when the disk cannot hold it.
#[arkavo_test_macros::spec("IMG-004")]
#[test]
fn space_is_checked_before_fetching() {
    let dir = tempfile::tempdir().unwrap();
    if cfg!(unix) {
        assert!(free_space(dir.path()).is_some_and(|b| b > 0));
        let err = check_space(dir.path(), u64::MAX / 2).unwrap_err();
        assert_eq!(err.code(), "disk_space");
    }
    assert!(check_space(dir.path(), 1).is_ok());
}

#[tokio::test]
async fn sha256_of_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("x");
    std::fs::write(&file, b"abc").unwrap();
    assert_eq!(
        sha256_file(&file).await.unwrap(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
