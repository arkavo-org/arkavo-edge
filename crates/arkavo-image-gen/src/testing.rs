//! Test fixtures: real PNG bytes, and an sd-cli stand-in that parses the
//! same flags and prints sd-cli's progress formats.

// The crate-visible items of this private module are its point: other test
// modules use them, and plain `pub` would trip `unreachable_pub`.
#![allow(clippy::redundant_pub_crate)]

use std::io::Write;
use std::path::{Path, PathBuf};

/// A valid PNG of the given size; RGBA when `alpha`, RGB otherwise.
pub(crate) fn png(width: u32, height: u32, alpha: bool) -> Vec<u8> {
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

/// How the stand-in behaves.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Behavior {
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
pub(crate) fn fake_cli(dir: &Path, fixture: &Path, behavior: Behavior) -> PathBuf {
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
