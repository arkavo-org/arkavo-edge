//! Integration checks against a real GGUF (spec §15.4).
//!
//! Both tests need a model on disk and are skipped unless
//! `ARKAVO_TEST_MODEL` points at one, so an ordinary `cargo test` run stays
//! hermetic. Neither test contacts a production KAS: the payload key is
//! recorded at wrap time and handed straight back at unlock time.
//!
//! Run with:
//!   ARKAVO_TEST_MODEL=/path/to/gemma-270m.gguf cargo test -p arkavo-gguf-tdf --test integration
//!
//! Expect minutes, not seconds, on a debug build: these move the whole model
//! through AES-GCM twice (wrap, then read back) and the cipher is unoptimized
//! without `--release`. A 500 MB model takes roughly four minutes.

mod common;

use arkavo_gguf_tdf::{
    GgufTdfArchive, GgufTdfError, PayloadKeyUnwrapper, PayloadKeyWrapper, ProtectOptions,
    WrappedKey, protect,
};
use base64::Engine as _;
use opentdf::TdfManifest;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct RecordingKas {
    key: Arc<Mutex<Option<[u8; 32]>>>,
}

impl PayloadKeyWrapper for RecordingKas {
    fn wrap(&self, payload_key: &[u8; 32]) -> Result<WrappedKey, GgufTdfError> {
        *self.key.lock().unwrap() = Some(*payload_key);
        Ok(WrappedKey {
            kas_url: "https://kas.example.invalid".to_string(),
            kid: Some("test".to_string()),
            wrapped_key: base64::engine::general_purpose::STANDARD.encode(payload_key),
        })
    }
}

impl PayloadKeyUnwrapper for RecordingKas {
    fn unwrap_key(&self, _manifest: &TdfManifest) -> Result<[u8; 32], GgufTdfError> {
        self.key
            .lock()
            .unwrap()
            .ok_or_else(|| GgufTdfError::KasDenied("no key recorded".to_string()))
    }
}

fn test_model() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("ARKAVO_TEST_MODEL")?;
    let path = std::path::PathBuf::from(path);
    if path.exists() {
        Some(path)
    } else {
        eprintln!(
            "ARKAVO_TEST_MODEL points at a missing file: {}",
            path.display()
        );
        None
    }
}

/// A real model wraps, and every byte the reader serves matches the source.
///
/// This is the byte-level half of I1: if the virtual GGUF is identical to the
/// source, any loader reading through the callback sees exactly the model it
/// would have mmapped, so logits cannot diverge.
#[test]
fn real_model_round_trips_byte_for_byte() {
    let Some(source) = test_model() else {
        eprintln!("skipping: set ARKAVO_TEST_MODEL to a .gguf file");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("model.gguf.tdf");
    let kas = RecordingKas::default();

    let report = protect(&source, &archive, &kas, &ProtectOptions::default())
        .expect("a real GGUF must wrap");
    let expected_len = std::fs::metadata(&source).unwrap().len();
    assert_eq!(report.virtual_size, expected_len);

    let mut vg = GgufTdfArchive::open(&archive)
        .unwrap()
        .unlock(&kas)
        .expect("unlock with the recorded key");

    let mut file = std::fs::File::open(&source).unwrap();
    let mut want = vec![0u8; 1 << 20];
    let mut got = vec![0u8; 1 << 20];
    let mut offset = 0u64;

    while offset < expected_len {
        let want_len = ((expected_len - offset) as usize).min(want.len());
        file.seek(SeekFrom::Start(offset)).unwrap();
        file.read_exact(&mut want[..want_len]).unwrap();

        let n = vg.read_at(offset, &mut got[..want_len]);
        assert_eq!(
            n,
            want_len,
            "short read at {offset}: {:?}",
            vg.error().map(|e| e.code())
        );
        assert_eq!(
            got[..want_len],
            want[..want_len],
            "virtual GGUF diverges from the source at offset {offset}"
        );
        offset += want_len as u64;
    }

    assert!(vg.error().is_none());
}

/// I2: extra anonymous plaintext during a full read stays near
/// `headerBytes + maxSegment`, not near the file size.
///
/// Measured as process RSS growth across a whole-file sequential read. The
/// archive is opened before the baseline is taken so the zip's own file-backed
/// pages are not counted as decrypt working set.
#[test]
fn load_working_set_stays_bounded() {
    let Some(source) = test_model() else {
        eprintln!("skipping: set ARKAVO_TEST_MODEL to a .gguf file");
        return;
    };

    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("model.gguf.tdf");
    let kas = RecordingKas::default();
    let opts = ProtectOptions::default();
    let report = protect(&source, &archive, &kas, &opts).expect("wrap");

    let mut vg = GgufTdfArchive::open(&archive)
        .unwrap()
        .unlock(&kas)
        .unwrap();

    let baseline = resident_bytes().expect("RSS is readable on this platform");
    let mut buf = vec![0u8; 1 << 20];
    let mut offset = 0u64;
    let mut peak = baseline;
    let mut since_sample = 0usize;
    while offset < report.virtual_size {
        let len = ((report.virtual_size - offset) as usize).min(buf.len());
        let n = vg.read_at(offset, &mut buf[..len]);
        assert_eq!(n, len);
        offset += len as u64;

        // Sampling RSS means a syscall or subprocess, so do it every 32 MiB
        // rather than every megabyte: the working set is steady-state, and
        // sampling per iteration dominates the test's runtime.
        since_sample += 1;
        if since_sample >= 32 {
            since_sample = 0;
            peak = peak.max(resident_bytes().unwrap_or(baseline));
        }
    }
    peak = peak.max(resident_bytes().unwrap_or(baseline));

    let growth = peak.saturating_sub(baseline);
    // The bound is headerBytes + maxSegment plus room for the ciphertext
    // copy-out and allocator slack. The point of the assertion is that growth
    // tracks the segment size rather than the model size.
    let bound = report.header_bytes + opts.max_segment * 4 + (16 << 20);
    assert!(
        growth < bound,
        "RSS grew {growth} bytes reading a {} byte model; bound is {bound}",
        report.virtual_size
    );
    assert!(
        growth < report.virtual_size,
        "working set must not scale with the model"
    );
}

/// Resident set size of this process, in bytes.
fn resident_bytes() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
        let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
        Some(pages * 4096)
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()?;
        let kb: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        Some(kb * 1024)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}
