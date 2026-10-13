//! Fixtures for image generation tests.
//!
//! Real PNG bytes, an sd-cli stand-in that parses the same flags and prints
//! sd-cli's progress formats, and a generator that runs it with weights from
//! a mock hub. Other crates enable the `testing` feature to exercise the
//! swarm paths without Metal or real weights.

// Fixtures fail their test by panicking; there is nothing for a caller to
// handle.
#![allow(clippy::missing_panics_doc)]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

use crate::catalog::{PinnedFile, RuntimePin, SetLabel, WeightSet, resolve};
use crate::generator::Generator;
use crate::host::{Host, Platform};
use crate::output::{AssetEntry, Generated, Manifest};
use crate::plan::Offer;
use crate::run::Sandbox;
use crate::store::Store;

/// A valid PNG of the given size; RGBA when `alpha`, RGB otherwise.
pub fn png(width: u32, height: u32, alpha: bool) -> Vec<u8> {
    let channels: usize = if alpha { 4 } else { 3 };
    let mut raw = Vec::new();
    for _ in 0..height {
        raw.push(0u8);
        raw.extend(std::iter::repeat_n(0x7f, width as usize * channels));
    }
    let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(&raw).unwrap();
    let data = encoder.finish().unwrap();

    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([8, if alpha { 6 } else { 2 }, 0, 0, 0]);

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, body) in [(&b"IHDR"[..], ihdr), (b"IDAT", data), (b"IEND", Vec::new())] {
        out.extend((body.len() as u32).to_be_bytes());
        let mut crc = flate2::Crc::new();
        crc.update(kind);
        crc.update(&body);
        out.extend(kind);
        out.extend(&body);
        out.extend(crc.sum().to_be_bytes());
    }
    out
}

/// 32x32 RGBA PNGs named `names` written to `dir`, described as the
/// generator describes its output.
pub fn generated(dir: &Path, names: &[&str]) -> Generated {
    let assets: Vec<AssetEntry> = names
        .iter()
        .enumerate()
        .map(|(seed, name)| {
            let bytes = png(32, 32, true);
            std::fs::write(dir.join(name), &bytes).unwrap();
            AssetEntry {
                name: (*name).to_string(),
                bytes: bytes.len() as u64,
                sha256: hex::encode(Sha256::digest(&bytes)),
                width: 32,
                height: 32,
                has_alpha: true,
                tileable: false,
                seed: u32::try_from(seed).unwrap(),
            }
        })
        .collect();
    let manifest: Manifest = serde_json::from_value(serde_json::json!({
        "model": {"id": "qwen-image-2.1-turbo", "repo": "Qwen/Qwen-Image-2.1-Turbo", "revision": "v",
                  "diffusion": "Q8_0", "text_encoder": "Q8_0"},
        "license": "Qwen Research License Agreement", "license_url": "u", "runtime": "rt",
        "producer": "did:key:z6Mkproducer", "prompt": "a coin", "assets": assets
    }))
    .unwrap();
    Generated::owning(dir.to_path_buf(), manifest)
}

/// How the stand-in behaves.
#[derive(Debug, Clone, Copy)]
pub enum Behavior {
    /// Writes `-b` copies of the fixture and prints sd-cli's progress.
    Generate,
    /// Prints an error and exits 3.
    Fail,
    /// Sleeps far past any test deadline.
    Hang,
    /// Writes its environment to env.txt in its working directory.
    DumpEnv,
}

/// Write an executable sd-cli stand-in in `dir` that copies `fixture` for
/// each image.
pub fn fake_cli(dir: &Path, fixture: &Path, behavior: Behavior) -> PathBuf {
    let body = match behavior {
        Behavior::Generate => format!(
            r#"out=""; batch=1; steps=8; seed=0
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out="$2"; shift 2;;
    -b) batch="$2"; shift 2;;
    --steps) steps="$2"; shift 2;;
    -s) seed="$2"; shift 2;;
    *) shift;;
  esac
done
echo "  |====>   | 12/731 - 9.0it/s"
i=0
while [ $i -lt $batch ]; do
  n=$((i+1))
  echo "[INFO ] stable-diffusion.cpp:1 - generating image: $n/$batch - seed $((seed+i))"
  s=1
  while [ $s -le $steps ]; do printf '  |==>   | %d/%d - 0.01s/it\r' $s $steps; s=$((s+1)); done
  cp "{fixture}" "$(printf "$out" $i)"
  i=$((i+1))
done
echo "[INFO ] generate_image completed""#,
            fixture = fixture.display()
        ),
        Behavior::Fail => "echo 'ggml_metal: unsupported op PAD' >&2; exit 3".to_string(),
        Behavior::Hang => "sleep 60".to_string(),
        Behavior::DumpEnv => "env > env.txt".to_string(),
    };
    let path = dir.join("sd-cli");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    path
}

const GIB: u64 = 1 << 30;

/// Answers a resolve URL the way the Hub does for an LFS file: the commit
/// and the file's SHA-256 as its etag, and the requested range of the body.
struct HubFile {
    body: Vec<u8>,
    revision: String,
    etag: String,
    delay: Duration,
}

impl Respond for HubFile {
    fn respond(&self, request: &Request) -> ResponseTemplate {
        let last = self.body.len().saturating_sub(1);
        let (start, stop) = request
            .headers
            .get("range")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("bytes="))
            .and_then(|v| v.split_once('-'))
            .and_then(|(a, b)| Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)))
            .map_or((0, last), |(a, b)| (a, b.min(last)));
        ResponseTemplate::new(206)
            .insert_header("x-repo-commit", self.revision.as_str())
            .insert_header("x-linked-etag", format!("\"{}\"", self.etag).as_str())
            .insert_header(
                "content-range",
                format!("bytes {start}-{stop}/{}", self.body.len()).as_str(),
            )
            .set_body_bytes(self.body[start..=stop].to_vec())
            .set_delay(self.delay)
    }
}

/// Serve `file` with `body` as the Hub would, expecting its bytes `times`.
///
/// The size and etag probes before each fetch are answered without
/// counting; a body that differs from the pin is served with the body's own
/// etag, as the Hub would serve another file.
pub async fn serve_hub_file(
    server: &MockServer,
    file: &PinnedFile,
    body: &[u8],
    times: u64,
    delay: Duration,
) {
    let route = format!("/{}/resolve/{}/{}", file.repo, file.revision, file.path);
    let respond = |delay| HubFile {
        body: body.to_vec(),
        revision: file.revision.to_string(),
        etag: hex::encode(Sha256::digest(body)),
        delay,
    };
    Mock::given(method("GET"))
        .and(path(route.clone()))
        .and(header("range", "bytes=0-0"))
        .respond_with(respond(Duration::ZERO))
        .with_priority(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(route))
        .respond_with(respond(delay))
        .expect(times)
        .mount(server)
        .await;
}

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

/// A generator on a pretend 32 GiB Apple M6 offering Turbo, and the mock hub
/// it fetches from.
pub struct FakeGenerator {
    pub server: MockServer,
    pub generator: std::sync::Arc<Generator>,
    _dirs: (tempfile::TempDir, tempfile::TempDir),
}

/// A generator whose runtime is the sd-cli stand-in and whose weights come
/// from a mock hub, each served `fetches` times.
pub async fn fake_generator(fetches: u64, delay: Duration) -> FakeGenerator {
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
    for ((_, body), pin) in files.iter().zip(&pins) {
        serve_hub_file(&server, pin, body, fetches, delay).await;
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
    let store = Store::with_hub(
        root.path().join("image-gen"),
        hf_hub::Cache::new(root.path().join("hub")),
        server.uri(),
    );
    let generator = Generator::new(
        host,
        store,
        vec![Offer::for_test(turbo, set, 20 * GIB)],
        runtime,
        Sandbox::Off,
    )
    .unwrap()
    .with_producer(Some("did:key:z6Mtest".into()));
    FakeGenerator {
        server,
        generator: std::sync::Arc::new(generator),
        _dirs: (tools, root),
    }
}
