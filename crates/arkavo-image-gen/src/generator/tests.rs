// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use super::*;
use crate::catalog::{PinnedFile, SetLabel, resolve};
use crate::host::Platform;
use crate::testing::{Behavior, fake_cli, png};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::time::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const GIB: u64 = 1 << 30;

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn pinned(name: &'static str, body: &[u8]) -> &'static PinnedFile {
    Box::leak(Box::new(PinnedFile {
        repo: "test/weights",
        revision: "1111111111111111111111111111111111111111",
        path: name,
        size: body.len() as u64,
        sha256: leak(hex::encode(Sha256::digest(body))),
    }))
}

struct Fixture {
    server: MockServer,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
    generator: Generator,
}

/// A generator whose runtime is the sd-cli stand-in and whose weights come
/// from a mock hub, each served `fetches` times.
async fn fixture(fetches: u64, delay: Duration) -> Fixture {
    let server = MockServer::start().await;
    let tools = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let image = tools.path().join("fixture.png");
    std::fs::write(&image, png(256, 256, true)).unwrap();
    let script = std::fs::read(fake_cli(tools.path(), &image, Behavior::Generate)).unwrap();

    let mut zip_bytes = std::io::Cursor::new(Vec::new());
    let mut zip = zip::ZipWriter::new(&mut zip_bytes);
    zip.start_file("sd-cli", zip::write::SimpleFileOptions::default())
        .unwrap();
    zip.write_all(&script).unwrap();
    zip.finish().unwrap();
    let zip_bytes = zip_bytes.into_inner();
    let runtime: &'static RuntimePin = Box::leak(Box::new(RuntimePin {
        build: 956,
        tag: "master-956-test",
        url: leak(format!("{}/sd.zip", server.uri())),
        size: zip_bytes.len() as u64,
        sha256: leak(hex::encode(Sha256::digest(&zip_bytes))),
        members: &["sd-cli"],
        cli: "sd-cli",
    }));
    Mock::given(method("GET"))
        .and(path("/sd.zip"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(zip_bytes))
        .mount(&server)
        .await;

    let files = [
        ("d.gguf", &b"diffusion"[..]),
        ("te.gguf", b"encoder"),
        ("vae.safetensors", b"vae"),
    ];
    let pins: Vec<_> = files
        .iter()
        .map(|(name, body)| pinned(name, body))
        .collect();
    for ((name, body), _) in files.iter().zip(&pins) {
        Mock::given(method("GET"))
            .and(path(format!(
                "/test/weights/resolve/1111111111111111111111111111111111111111/{name}"
            )))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_bytes(*body)
                    .set_delay(delay),
            )
            .expect(fetches)
            .mount(&server)
            .await;
    }
    let set: &'static WeightSet = Box::leak(Box::new(WeightSet {
        label: SetLabel::L,
        diffusion: pins[0],
        text_encoder: pins[1],
        vae: pins[2],
    }));
    let turbo = resolve("qwen-image-2.1-turbo").unwrap().entry;
    let host = Host {
        platform: Platform::AppleSilicon,
        chip: Some("Apple M6".into()),
        memory_bytes: 32 * GIB,
    };
    let store = Store::with_hub(root.path().to_path_buf(), server.uri());
    let generator = Generator::new(
        host,
        store,
        vec![Offer::for_test(turbo, set, 20 * GIB)],
        runtime,
        Sandbox::Off,
    )
    .unwrap()
    .with_producer(Some("did:key:z6Mtest".into()));
    Fixture {
        server,
        _dirs: (tools, root),
        generator,
    }
}

fn request(count: u32) -> ImageRequest {
    ImageRequest {
        prompt: "a gold coin".into(),
        width: Some(256),
        height: Some(256),
        count: Some(count),
        seed: Some(5),
        name: Some("coin".into()),
        transparent: Some(true),
        ..ImageRequest::default()
    }
}

fn soon(seconds: u64) -> Instant {
    Instant::now() + Duration::from_secs(seconds)
}

/// IMG-005, IMG-010, IMG-013: the first request fetches the runtime and
/// weights, then generates verified assets described by a manifest.
#[tokio::test]
async fn first_request_fetches_then_generates() {
    let f = fixture(1, Duration::ZERO).await;
    assert!(!f.generator.descriptor().models[0].ready);
    let seen = Mutex::new(Vec::new());
    let generated = f
        .generator
        .generate(&request(2), soon(60), &|p| seen.lock().unwrap().push(p))
        .await
        .unwrap();
    let seen = seen.into_inner().unwrap();
    assert!(seen.iter().any(|p| matches!(p, Progress::Fetching { .. })));
    assert!(seen.contains(&Progress::Step {
        image: 2,
        images: 2,
        step: 8,
        steps: 8
    }));
    let m = &generated.manifest;
    assert_eq!(m.assets.len(), 2);
    assert_eq!(m.assets[1].seed, 6);
    assert!(m.assets.iter().all(|a| a.has_alpha));
    assert_eq!(m.producer.as_deref(), Some("did:key:z6Mtest"));
    assert!(generated.path_of(&m.assets[0]).exists());
    assert!(!generated.dir.join(".prompt").exists());

    let row = &f.generator.descriptor().models[0];
    assert!(row.ready);
    assert_eq!(row.bytes_remaining, 0);
    assert!(row.seconds_per_step.is_some());
    assert_eq!(row.aliases, ["qwen-image"]);

    // Weights and runtime are on disk now; a second request fetches nothing
    // (each weight mock expects exactly one GET).
    f.generator
        .generate(&request(1), soon(60), &|_| {})
        .await
        .unwrap();
    drop(f.server);
}

/// IMG-018, IMG-019: refusals come before any fetch.
#[tokio::test]
async fn refusals_fetch_nothing() {
    let f = fixture(0, Duration::ZERO).await;
    let too_large = ImageRequest {
        width: Some(2048),
        height: Some(2048),
        ..request(1)
    };
    let err = f
        .generator
        .generate(&too_large, soon(60), &|_| {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), "too_large_for_host");
    let other = ImageRequest {
        model: Some("qwen-image-2.1".into()),
        width: Some(512),
        height: Some(512),
        ..request(1)
    };
    let err = f
        .generator
        .generate(&other, soon(60), &|_| {})
        .await
        .unwrap_err();
    assert!(matches!(err, ImageGenError::NotOffered { .. }), "{err}");
    let invalid = ImageRequest {
        prompt: " ".into(),
        ..request(1)
    };
    let err = f
        .generator
        .generate(&invalid, soon(60), &|_| {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), "invalid_params");
}

/// IMG-005: concurrent requests share one fetch.
#[tokio::test]
async fn concurrent_requests_share_one_fetch() {
    let f = fixture(1, Duration::from_millis(200)).await;
    let (first, second) = (request(1), request(1));
    let (a, b) = tokio::join!(
        f.generator.generate(&first, soon(60), &|_| {}),
        f.generator.generate(&second, soon(60), &|_| {}),
    );
    a.unwrap();
    b.unwrap();
}

/// IMG-012: a request that gives up during a fetch stops waiting, and the
/// fetch carries on for the next request.
#[tokio::test]
async fn fetches_outlive_requesters_that_time_out() {
    let f = fixture(1, Duration::from_millis(600)).await;
    let err = f
        .generator
        .generate(
            &request(1),
            Instant::now() + Duration::from_millis(100),
            &|_| {},
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ImageGenError::TimedOut), "{err}");
    f.generator
        .generate(&request(1), soon(60), &|_| {})
        .await
        .unwrap();
}

/// IMG-011: four may wait; the fifth is told to go elsewhere.
#[test]
fn the_queue_admits_four() {
    let waiting = AtomicUsize::new(0);
    let admitted: Vec<_> = (0..MAX_QUEUE)
        .map(|_| Admission::enter(&waiting).unwrap())
        .collect();
    let err = Admission::enter(&waiting).err().unwrap();
    assert!(err.try_elsewhere());
    assert_eq!(err.code(), "busy");
    drop(admitted);
    assert_eq!(waiting.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn prefetch_readies_an_entry() {
    let f = fixture(1, Duration::ZERO).await;
    f.generator.prefetch("qwen-image", &|_| {}).await.unwrap();
    assert!(f.generator.descriptor().models[0].ready);
    let err = f
        .generator
        .prefetch("qwen-image-quality", &|_| {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), "no_capable_member");
}

#[test]
fn non_qualifying_hosts_get_no_generator() {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        assert!(Generator::for_this_host(0, None).is_none());
    }
}
