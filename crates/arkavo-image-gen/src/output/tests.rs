// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use super::*;
use crate::request::ImageRequest;
use crate::testing::png;

fn job(count: u32, transparent: bool) -> Job {
    ImageRequest {
        prompt: "coin".into(),
        width: Some(256),
        height: Some(256),
        count: Some(count),
        seed: Some(10),
        name: Some("coin".into()),
        transparent: Some(transparent),
        ..ImageRequest::default()
    }
    .validate()
    .unwrap()
}

fn set(job: &Job) -> &'static WeightSet {
    &job.entry.sets[1]
}

#[test]
fn headers_report_size_and_alpha() {
    assert_eq!(
        png_header(&png(64, 32, true)),
        Some(PngHeader {
            width: 64,
            height: 32,
            has_alpha: true
        })
    );
    assert!(!png_header(&png(8, 8, false)).unwrap().has_alpha);
    assert_eq!(png_header(b"GIF89a"), None);
    assert_eq!(png_header(&png(8, 8, false)[..20]), None);
}

/// IMG-009: a tRNS chunk counts as alpha.
#[test]
fn trns_chunks_count_as_alpha() {
    let mut bytes = png(8, 8, false);
    let ihdr_end = 8 + 12 + 13;
    let mut chunk = Vec::new();
    chunk.extend(6u32.to_be_bytes());
    chunk.extend(b"tRNS");
    chunk.extend([0, 0, 0, 0, 0, 0]);
    chunk.extend([0, 0, 0, 0]);
    bytes.splice(ihdr_end..ihdr_end, chunk);
    assert!(png_header(&bytes).unwrap().has_alpha);
}

/// IMG-010, IMG-013: every image is checked and described, with the entry,
/// license and runtime named.
#[tokio::test]
async fn collect_describes_verified_images() {
    let dir = tempfile::tempdir().unwrap();
    let job = job(2, true);
    std::fs::write(dir.path().join("coin_000.png"), png(256, 256, true)).unwrap();
    std::fs::write(dir.path().join("coin_001.png"), png(256, 256, false)).unwrap();
    std::fs::write(dir.path().join(".prompt"), b"coin").unwrap();
    let manifest = collect(dir.path(), &job, set(&job), &[10, 11])
        .await
        .unwrap();
    assert_eq!(manifest.assets.len(), 2);
    assert_eq!(manifest.assets[0].name, "coin_000.png");
    assert_eq!(manifest.assets[1].seed, 11);
    assert!(manifest.assets[0].has_alpha && !manifest.assets[1].has_alpha);
    assert!(manifest.transparency_missing);
    assert_eq!(manifest.model.id, "qwen-image-2.1-turbo");
    assert_eq!(manifest.model.alias.as_deref(), Some("qwen-image"));
    assert_eq!(manifest.model.diffusion, "Qwen-Image-2.1-Turbo-Q8_0.gguf");
    assert_eq!(manifest.runtime, "stable-diffusion.cpp master-956-1b0ba10");
    assert_eq!(manifest.license, "Qwen Research License Agreement");
    assert_eq!(manifest.guidance, None);
    assert!(
        manifest
            .license_line()
            .contains("Qwen Research License Agreement")
    );
    assert_eq!(manifest.assets[0].sha256.len(), 64);
}

#[tokio::test]
async fn seeds_fall_back_to_consecutive_values() {
    let dir = tempfile::tempdir().unwrap();
    let job = job(2, false);
    for i in 0..2 {
        std::fs::write(
            dir.path().join(format!("coin_{i:03}.png")),
            png(256, 256, false),
        )
        .unwrap();
    }
    let manifest = collect(dir.path(), &job, set(&job), &[]).await.unwrap();
    assert_eq!(manifest.assets[0].seed, 10);
    assert_eq!(manifest.assets[1].seed, 11);
    assert!(!manifest.transparency_missing);
}

#[tokio::test]
async fn missing_wrong_and_extra_images_are_rejected() {
    let job = job(1, false);
    let dir = tempfile::tempdir().unwrap();
    let err = collect(dir.path(), &job, set(&job), &[]).await.unwrap_err();
    assert!(err.to_string().contains("was not written"), "{err}");

    std::fs::write(dir.path().join("coin_000.png"), png(128, 256, false)).unwrap();
    let err = collect(dir.path(), &job, set(&job), &[]).await.unwrap_err();
    assert!(err.to_string().contains("128x256"), "{err}");

    std::fs::write(dir.path().join("coin_000.png"), b"not a png").unwrap();
    let err = collect(dir.path(), &job, set(&job), &[]).await.unwrap_err();
    assert!(err.to_string().contains("is not a PNG"), "{err}");

    std::fs::write(dir.path().join("coin_000.png"), png(256, 256, false)).unwrap();
    std::fs::write(dir.path().join("coin_001.png"), png(256, 256, false)).unwrap();
    let err = collect(dir.path(), &job, set(&job), &[]).await.unwrap_err();
    assert!(err.to_string().contains("found 2"), "{err}");
}
