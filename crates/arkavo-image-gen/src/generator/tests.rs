// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use super::*;
use crate::testing::fake_generator;
use std::time::Duration;

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
#[arkavo_test_macros::spec("IMG-005", "IMG-010", "IMG-013")]
#[tokio::test]
async fn first_request_fetches_then_generates() {
    let f = fake_generator(1, Duration::ZERO).await;
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
    assert_eq!(m.producer_agent.as_deref(), Some("test-host"));
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

/// IMG-007, IMG-019: refusals come before any fetch.
#[arkavo_test_macros::spec("IMG-007", "IMG-019")]
#[tokio::test]
async fn refusals_fetch_nothing() {
    let f = fake_generator(0, Duration::ZERO).await;
    let too_large = ImageRequest {
        width: Some(2752),
        height: Some(2752),
        ..request(1)
    };
    let err = f
        .generator
        .generate(&too_large, soon(60), &|_| {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), "invalid_params");
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
#[arkavo_test_macros::spec("IMG-005")]
#[tokio::test]
async fn concurrent_requests_share_one_fetch() {
    let f = fake_generator(1, Duration::from_millis(200)).await;
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
#[arkavo_test_macros::spec("IMG-012")]
#[tokio::test]
async fn fetches_outlive_requesters_that_time_out() {
    let f = fake_generator(1, Duration::from_millis(600)).await;
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
#[arkavo_test_macros::spec("IMG-011")]
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

/// Generate one image while another agent on the host holds the lock
/// `name`, releasing it once the request says it is queued. Returns what
/// the request reported.
async fn generate_behind(generator: &Generator, name: &str) -> Vec<Progress> {
    let held = HostLock::acquire(generator.store.root(), name, None, || {})
        .await
        .unwrap();
    let seen = Mutex::new(Vec::new());
    let (one, record) = (request(1), |p| seen.lock().unwrap().push(p));
    let generating = generator.generate(&one, soon(60), &record);
    let release = async {
        while !seen.lock().unwrap().contains(&Progress::Queued) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        drop(held);
    };
    let (generated, ()) = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::join!(generating, release)
    })
    .await
    .expect("the request said it was queued");
    generated.unwrap();
    seen.into_inner().unwrap()
}

/// IMG-011, ASSET-009: a request waiting on another agent's generation says
/// it is queued, so a delegator does not take the wait for a stall.
#[arkavo_test_macros::spec("IMG-011", "ASSET-009")]
#[tokio::test]
async fn waiting_on_another_agents_run_reports_queued() {
    let f = fake_generator(1, Duration::ZERO).await;
    f.generator.prefetch("qwen-image", &|_| {}).await.unwrap();
    let seen = generate_behind(&f.generator, lock::GENERATE).await;
    let queued = seen.iter().position(|p| *p == Progress::Queued).unwrap();
    assert!(
        seen[queued..]
            .iter()
            .any(|p| matches!(p, Progress::Step { .. })),
        "{seen:?}"
    );
}

/// IMG-011, ASSET-009: a request waiting on another agent's download, of
/// the runtime or of weights, says it is queued.
#[arkavo_test_macros::spec("IMG-011", "ASSET-009")]
#[tokio::test]
async fn waiting_on_another_agents_fetch_reports_queued() {
    let f = fake_generator(1, Duration::ZERO).await;
    let seen = generate_behind(&f.generator, lock::FETCH).await;
    assert!(seen.iter().any(|p| matches!(p, Progress::Step { .. })));

    let f = fake_generator(1, Duration::ZERO).await;
    f.generator.runtime(&|_| {}).await.unwrap();
    let seen = generate_behind(&f.generator, lock::FETCH).await;
    let queued = seen.iter().position(|p| *p == Progress::Queued).unwrap();
    assert!(
        seen[queued..]
            .iter()
            .any(|p| matches!(p, Progress::Fetching { .. })),
        "{seen:?}"
    );
}

#[arkavo_test_macros::spec("IMG-005", "CLI-020")]
#[tokio::test]
async fn prefetch_readies_an_entry() {
    let f = fake_generator(1, Duration::ZERO).await;
    f.generator.prefetch("qwen-image", &|_| {}).await.unwrap();
    assert!(f.generator.descriptor().models[0].ready);
    let err = f
        .generator
        .prefetch("qwen-image-quality", &|_| {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), "no_capable_member");
}

#[arkavo_test_macros::spec("IMG-001")]
#[test]
fn non_qualifying_hosts_get_no_generator() {
    if !cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        assert!(Generator::for_this_host(0, Producer::default()).is_none());
    }
}
