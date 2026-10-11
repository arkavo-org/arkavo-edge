//! Installing and checking the pinned stable-diffusion.cpp release
//! (IMG-003).

use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::catalog::RuntimePin;
use crate::error::{ImageGenError, Result};
use crate::fetch::{Fetcher, Pin, sha256_file};
use crate::store::Store;

const MARKER: &str = ".installed.json";

/// An installed runtime.
#[derive(Debug, Clone)]
pub struct Runtime {
    dir: PathBuf,
    cli: PathBuf,
    files: Vec<InstalledFile>,
}

/// Size and hash of one extracted file, recorded at install.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct InstalledFile {
    name: String,
    size: u64,
    sha256: String,
}

impl Runtime {
    pub fn cli(&self) -> &Path {
        &self.cli
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Compare every file with what was recorded at install; done before
    /// each start, since these files are executed.
    pub async fn verify(&self) -> Result<()> {
        for file in &self.files {
            let path = self.dir.join(&file.name);
            let size = tokio::fs::metadata(&path).await?.len();
            if size != file.size {
                return Err(ImageGenError::SizeMismatch {
                    name: path.display().to_string(),
                    expected: file.size,
                    actual: size,
                });
            }
            let actual = sha256_file(&path).await?;
            if actual != file.sha256 {
                return Err(ImageGenError::HashMismatch {
                    name: path.display().to_string(),
                    expected: file.sha256.clone(),
                    actual,
                });
            }
        }
        Ok(())
    }
}

/// The installed runtime for `pin`, installing it when it is missing or no
/// longer matches its install record.
pub async fn install(store: &Store, fetcher: &Fetcher, pin: &RuntimePin) -> Result<Runtime> {
    let dir = store.runtime_dir().join(pin.tag);
    if let Some(runtime) = load(&dir, pin).await
        && runtime.verify().await.is_ok()
    {
        return Ok(runtime);
    }
    if tokio::fs::try_exists(&dir).await? {
        tokio::fs::remove_dir_all(&dir).await?;
    }
    let archive = store.runtime_dir().join(format!("{}.zip", pin.tag));
    fetcher
        .fetch(
            Pin {
                url: pin.url,
                size: pin.size,
                sha256: pin.sha256,
            },
            &archive,
            &|_| {},
        )
        .await?;
    let staging = store.runtime_dir().join(format!("{}.staging", pin.tag));
    let members = pin.members;
    let (archive_path, staging_path) = (archive.clone(), staging.clone());
    tokio::task::spawn_blocking(move || extract(&archive_path, &staging_path, members))
        .await
        .map_err(|e| ImageGenError::RuntimeInstall(e.to_string()))??;
    let mut files = Vec::with_capacity(members.len());
    for name in members {
        let path = staging.join(name);
        files.push(InstalledFile {
            name: (*name).to_string(),
            size: tokio::fs::metadata(&path).await?.len(),
            sha256: sha256_file(&path).await?,
        });
    }
    let record = serde_json::to_vec_pretty(&files)
        .map_err(|e| ImageGenError::RuntimeInstall(e.to_string()))?;
    tokio::fs::write(staging.join(MARKER), record).await?;
    tokio::fs::rename(&staging, &dir).await?;
    tokio::fs::remove_file(&archive).await?;
    Ok(Runtime {
        cli: dir.join(pin.cli),
        dir,
        files,
    })
}

async fn load(dir: &Path, pin: &RuntimePin) -> Option<Runtime> {
    let record = tokio::fs::read(dir.join(MARKER)).await.ok()?;
    let files: Vec<InstalledFile> = serde_json::from_slice(&record).ok()?;
    let names: Vec<&str> = files.iter().map(|f| f.name.as_str()).collect();
    (names == pin.members).then(|| Runtime {
        cli: dir.join(pin.cli),
        dir: dir.to_path_buf(),
        files,
    })
}

/// Extract the listed members into `staging`. Other entries are skipped;
/// a name with a path in it, or a listed member that is missing, fails.
fn extract(archive: &Path, staging: &Path, members: &[&str]) -> Result<()> {
    let fail = |reason: String| ImageGenError::RuntimeInstall(reason);
    if staging.exists() {
        std::fs::remove_dir_all(staging)?;
    }
    std::fs::create_dir_all(staging)?;
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive)?)
        .map_err(|e| fail(format!("unreadable archive: {e}")))?;
    let mut found = Vec::new();
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| fail(format!("unreadable entry {index}: {e}")))?;
        let name = entry.name().to_string();
        if name.contains(['/', '\\']) || name == ".." || name.is_empty() {
            return Err(fail(format!("archive entry {name:?} names a path")));
        }
        if !members.contains(&name.as_str()) {
            continue;
        }
        let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
        entry.read_to_end(&mut bytes)?;
        let path = staging.join(&name);
        std::fs::write(&path, bytes)?;
        set_mode(&path, name.ends_with(".txt"))?;
        found.push(name);
    }
    if let Some(missing) = members.iter().find(|m| !found.iter().any(|f| f == *m)) {
        return Err(fail(format!("archive has no {missing}")));
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, text: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if text { 0o644 } else { 0o755 };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _text: bool) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
