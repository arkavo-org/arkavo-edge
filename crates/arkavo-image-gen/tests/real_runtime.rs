//! The pinned stable-diffusion.cpp release running real Qwen-Image weights.
//!
//! Ignored: the first run fetches about 10 GB into the arkavo data
//! directory (or `ARKAVO_IMAGE_GEN_ROOT`), which the agent then reuses. On
//! Apple Silicon it runs the pinned Metal build under the sandbox profile;
//! on x86_64 Linux it runs the same release's CPU build, which checks the
//! command line and the model files but says nothing about Metal.
//!
//! `cargo test -p arkavo-image-gen --test real_runtime -- --ignored --nocapture`

// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use std::path::PathBuf;
use std::time::Duration;

use arkavo_image_gen::catalog::RuntimePin;
use arkavo_image_gen::run::Sandbox;
use arkavo_image_gen::{Generator, Host, ImageRequest, Platform, Progress, RUNTIME, Store, offers};
use tokio::time::Instant;

const GIB: u64 = 1 << 30;

/// The Linux CPU build of the pinned release, for checking on hosts that
/// cannot run Metal.
const LINUX_X86_64: RuntimePin = RuntimePin {
    build: 956,
    tag: "master-956-1b0ba10-linux-x86_64",
    url: "https://github.com/leejet/stable-diffusion.cpp/releases/download/master-956-1b0ba10/sd-master-1b0ba10-bin-Linux-Ubuntu-24.04-x86_64.zip",
    size: 26_130_308,
    sha256: "4f6b7e110b641c311f7537b143aae33a3e17b532ead690e391deff972b567da2",
    members: &[
        "sd-cli",
        "libstable-diffusion.so",
        "libggml.so.0",
        "libggml-base.so.0",
        "libggml-cpu-alderlake.so",
        "libggml-cpu-cannonlake.so",
        "libggml-cpu-cascadelake.so",
        "libggml-cpu-cooperlake.so",
        "libggml-cpu-haswell.so",
        "libggml-cpu-icelake.so",
        "libggml-cpu-ivybridge.so",
        "libggml-cpu-piledriver.so",
        "libggml-cpu-sandybridge.so",
        "libggml-cpu-sapphirerapids.so",
        "libggml-cpu-skylakex.so",
        "libggml-cpu-sse42.so",
        "libggml-cpu-x64.so",
        "libggml-cpu-zen4.so",
        "libwebp.so.7",
        "libwebpmux.so.3",
        "libwebm.so",
        "libsharpyuv.so.0",
        "ggml.txt",
        "stable-diffusion.cpp.txt",
    ],
    cli: "sd-cli",
};

fn root() -> PathBuf {
    std::env::var_os("ARKAVO_IMAGE_GEN_ROOT")
        .map(PathBuf::from)
        .or_else(Store::default_root)
        .expect("a data directory")
}

/// The generator for this machine: the real one on Apple Silicon, the Linux
/// CPU build elsewhere, with the small weight set either way.
fn generator() -> Generator {
    let detected = Host::detect();
    let (host, pin, sandbox) = if detected.qualifies() {
        (detected, &RUNTIME, Sandbox::MacOs)
    } else {
        let host = Host {
            platform: Platform::Other,
            chip: None,
            memory_bytes: 0,
        };
        (host, &LINUX_X86_64, Sandbox::Off)
    };
    // 12.5 GiB holds the small set and a 1 MP working set, nothing larger.
    let offers = offers(12 * GIB + GIB / 2);
    Generator::new(host, Store::new(root()), offers, pin, sandbox).expect("generator")
}

fn report(progress: Progress) {
    eprintln!("{progress}");
}

/// IMG-008, IMG-009: Turbo renders a transparent sprite with its distilled
/// schedule, and the PNG comes back verified.
#[tokio::test]
#[ignore = "fetches about 10 GB of weights and runs the model"]
async fn turbo_renders_a_transparent_sprite() {
    let generator = generator();
    let side = if cfg!(target_os = "macos") { 1024 } else { 256 };
    let request = ImageRequest {
        prompt: "A shiny gold coin game icon, front view, flat shading".into(),
        width: Some(side),
        height: Some(side),
        seed: Some(42),
        name: Some("coin".into()),
        transparent: Some(true),
        ..ImageRequest::default()
    };
    let started = std::time::Instant::now();
    let generated = generator
        .generate(&request, Instant::now() + Duration::from_hours(3), &report)
        .await
        .expect("generation");
    let manifest = &generated.manifest;
    eprintln!(
        "{} in {:.0}s; {}",
        generated.path_of(&manifest.assets[0]).display(),
        started.elapsed().as_secs_f64(),
        manifest.license_line()
    );
    assert_eq!(manifest.assets.len(), 1);
    assert_eq!(
        (manifest.assets[0].width, manifest.assets[0].height),
        (side, side)
    );
    assert_eq!(manifest.assets[0].seed, 42);
    assert_eq!(manifest.model.id, "qwen-image-2.1-turbo");
    let row = &generator.descriptor().models[0];
    assert!(row.ready);
    eprintln!("seconds per step at 1 MP: {:?}", row.seconds_per_step);
}
