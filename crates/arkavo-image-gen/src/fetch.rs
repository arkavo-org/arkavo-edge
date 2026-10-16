//! Pinned downloads: resumable, size- and hash-checked (IMG-004).

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::{info, warn};

use crate::error::{ImageGenError, Result};

const ATTEMPTS: u32 = 3;
const READ_BUFFER: usize = 1 << 20;

/// Bytes fetched of bytes needed for one file, reported as it arrives.
pub type Progress<'a> = &'a (dyn Fn(u64) + Send + Sync);

/// What one download must produce.
#[derive(Debug, Clone, Copy)]
pub struct Pin<'a> {
    pub url: &'a str,
    pub size: u64,
    pub sha256: &'a str,
}

/// HTTP client for pinned files. Sends no credentials: every pin is public.
#[derive(Debug, Clone)]
pub struct Fetcher {
    client: reqwest::Client,
    /// Wait before the second attempt; it grows linearly after that.
    backoff: Duration,
}

impl Fetcher {
    pub fn new() -> Result<Self> {
        let client = reqwest::Client::builder()
            .user_agent(concat!("arkavo-edge/", env!("CARGO_PKG_VERSION")))
            .connect_timeout(Duration::from_secs(30))
            .read_timeout(Duration::from_mins(2))
            .build()
            .map_err(|e| ImageGenError::Download {
                url: String::new(),
                reason: e.to_string(),
            })?;
        Ok(Self {
            client,
            backoff: Duration::from_secs(2),
        })
    }

    #[cfg(test)]
    pub(crate) fn without_backoff() -> Self {
        Self {
            backoff: Duration::ZERO,
            ..Self::new().expect("client")
        }
    }

    /// Make `dest` the pinned file. A file already at `dest` with the
    /// pinned size passed its hash check when it was renamed there.
    pub async fn fetch(&self, pin: Pin<'_>, dest: &Path, progress: Progress<'_>) -> Result<()> {
        if tokio::fs::metadata(dest)
            .await
            .is_ok_and(|m| m.len() == pin.size)
        {
            progress(pin.size);
            return Ok(());
        }
        let part = part_path(dest);
        let mut last_error = None;
        for attempt in 1..=ATTEMPTS {
            match self.download(pin, &part, progress).await {
                Ok(()) => {
                    verify(pin, &part).await?;
                    tokio::fs::rename(&part, dest).await?;
                    info!(url = pin.url, bytes = pin.size, "pinned file verified");
                    return Ok(());
                }
                Err(e @ (ImageGenError::SizeMismatch { .. } | ImageGenError::DiskSpace { .. })) => {
                    return Err(e);
                }
                Err(e) => {
                    warn!(url = pin.url, attempt, error = %e, "download interrupted; resuming");
                    last_error = Some(e);
                    if attempt < ATTEMPTS {
                        tokio::time::sleep(self.backoff * attempt).await;
                    }
                }
            }
        }
        Err(last_error.unwrap_or_else(|| ImageGenError::Download {
            url: pin.url.to_string(),
            reason: "no attempt ran".to_string(),
        }))
    }

    /// Append to `part` from where it ends, or start over when the server
    /// ignores the range.
    async fn download(&self, pin: Pin<'_>, part: &Path, progress: Progress<'_>) -> Result<()> {
        let have = tokio::fs::metadata(part).await.map_or(0, |m| m.len());
        if have > pin.size {
            tokio::fs::remove_file(part).await?;
            return Box::pin(self.download(pin, part, progress)).await;
        }
        if have == pin.size {
            return Ok(());
        }
        if let Some(parent) = part.parent() {
            tokio::fs::create_dir_all(parent).await?;
            check_space(parent, pin.size - have)?;
        }
        let failed = |reason: String| ImageGenError::Download {
            url: pin.url.to_string(),
            reason,
        };
        let mut request = self.client.get(pin.url);
        if have > 0 {
            request = request.header(reqwest::header::RANGE, format!("bytes={have}-"));
        }
        let response = request.send().await.map_err(|e| failed(e.to_string()))?;
        let status = response.status();
        let resumed = status == reqwest::StatusCode::PARTIAL_CONTENT;
        if !status.is_success() {
            return Err(failed(format!("HTTP {status}")));
        }
        let mut file = tokio::fs::OpenOptions::new()
            .create(true)
            .append(resumed)
            .write(true)
            .truncate(!resumed)
            .open(part)
            .await?;
        let mut written = if resumed { have } else { 0 };
        progress(written);
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk.map_err(|e| failed(e.to_string()))?;
            written += chunk.len() as u64;
            if written > pin.size {
                drop(file);
                tokio::fs::remove_file(part).await?;
                return Err(size_mismatch(pin, written));
            }
            file.write_all(&chunk).await?;
            progress(written);
        }
        file.flush().await?;
        if written != pin.size {
            return Err(failed(format!(
                "stream ended at {written} of {} bytes",
                pin.size
            )));
        }
        Ok(())
    }
}

fn size_mismatch(pin: Pin<'_>, actual: u64) -> ImageGenError {
    ImageGenError::SizeMismatch {
        name: pin.url.to_string(),
        expected: pin.size,
        actual,
    }
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".part");
    dest.with_file_name(name)
}

/// Hash `part`; on mismatch delete it, so the next attempt starts clean.
async fn verify(pin: Pin<'_>, part: &Path) -> Result<()> {
    let actual = sha256_file(part).await?;
    if actual != pin.sha256 {
        tokio::fs::remove_file(part).await?;
        return Err(ImageGenError::HashMismatch {
            name: pin.url.to_string(),
            expected: pin.sha256.to_string(),
            actual,
        });
    }
    Ok(())
}

/// SHA-256 of a file, as lowercase hex.
pub async fn sha256_file(path: &Path) -> Result<String> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; READ_BUFFER];
    loop {
        let read = file.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Refuse before fetching when `dir`'s filesystem lacks `needed` bytes
/// plus 10 percent.
pub fn check_space(dir: &Path, needed: u64) -> Result<()> {
    let Some(available) = free_space(dir) else {
        return Ok(());
    };
    let needed = needed.saturating_add(needed / 10);
    if available < needed {
        return Err(ImageGenError::DiskSpace {
            dir: dir.to_path_buf(),
            needed,
            available,
        });
    }
    Ok(())
}

/// Free bytes on the filesystem holding `dir`, `None` where unknown.
#[cfg(unix)]
pub fn free_space(dir: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStrExt;

    let path = CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut stat = MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `path` is a valid NUL-terminated string and `stat` is written
    // by statvfs before it is read; the return value is checked first.
    let stat = unsafe {
        if libc::statvfs(path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return None;
        }
        stat.assume_init()
    };
    // The field widths differ between macOS and Linux.
    #[allow(clippy::unnecessary_cast, clippy::useless_conversion)]
    let bytes = u64::from(stat.f_bavail as u64) * u64::from(stat.f_frsize as u64);
    Some(bytes)
}

#[cfg(not(unix))]
pub fn free_space(_dir: &Path) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests;
