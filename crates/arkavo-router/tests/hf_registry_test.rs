//! Checks the local model registry against HuggingFace.
//!
//! A repo or filename that does not exist only shows up when a user's download
//! returns 404, so this asks HuggingFace directly for every local arm.
//!
//! Run with: cargo test -p arkavo-router --test hf_registry_test -- --ignored --nocapture

// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// disallows because of its hazards inside async code; a test entry point is
// not inside one.
#![allow(clippy::disallowed_methods)]

use arkavo_router::ModelChoice;
use std::time::Duration;

/// Large files answer with a redirect to the storage backend. The redirect is
/// not followed: it already proves the file exists, and following it would
/// make the check depend on the storage backend accepting HEAD requests.
fn resolves(status: reqwest::StatusCode) -> bool {
    status.is_success() || status.is_redirection()
}

#[tokio::test]
#[ignore = "requires network access to huggingface.co"]
async fn every_local_arm_resolves_on_huggingface() {
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(30))
        .build()
        .expect("HTTP client");

    let mut broken = Vec::new();
    for model in ModelChoice::ALL_LOCAL {
        let repo = model.repo_id().expect("local arm has a repo");
        let file = model.gguf_filename().expect("local arm has a filename");
        let url = format!("https://huggingface.co/{repo}/resolve/main/{file}");
        match client.head(&url).send().await {
            Ok(response) if resolves(response.status()) => {
                println!("ok {} {repo} {file}", response.status().as_u16());
            }
            Ok(response) => broken.push(format!("{model:?}: {} for {url}", response.status())),
            Err(e) => broken.push(format!("{model:?}: {e} for {url}")),
        }
    }

    assert!(
        broken.is_empty(),
        "local arms that do not resolve on HuggingFace:\n{}",
        broken.join("\n")
    );
}

#[test]
fn a_missing_file_does_not_count_as_resolved() {
    assert!(resolves(reqwest::StatusCode::OK));
    assert!(resolves(reqwest::StatusCode::FOUND));
    assert!(!resolves(reqwest::StatusCode::NOT_FOUND));
    assert!(!resolves(reqwest::StatusCode::UNAUTHORIZED));
}
