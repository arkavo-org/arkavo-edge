//! A specialist's binding and active delegation on disk, kept beside its
//! key so they survive restarts (spec §7.3.1, §7.4).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::protocol::Binding;

const BINDING_FILE: &str = "swarmkit-binding.json";
const DELEGATION_FILE: &str = "swarmkit-delegation.json";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("{path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    /// A binding that cannot be read must not read as no binding: that
    /// would let the next host to reach the specialist bind it on first use.
    #[error("{path} is not a binding ({reason}); fix or remove it with `arkavo agent release`")]
    CorruptBinding { path: PathBuf, reason: String },
    #[error("{0} is not an Ed25519 did:key")]
    NotADidKey(String),
}

/// The directory a specialist's state lives in.
#[derive(Debug, Clone)]
pub struct StateStore {
    dir: PathBuf,
}

impl StateStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn load_binding(&self) -> Result<Option<Binding>, StoreError> {
        let path = self.dir.join(BINDING_FILE);
        let Some(text) = read(&path)? else {
            return Ok(None);
        };
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| StoreError::CorruptBinding {
                path,
                reason: e.to_string(),
            })
    }

    /// The stored envelope, or `None` when there is none or it is not JSON.
    /// Losing a delegation only loses the role; the caller re-verifies what
    /// it gets.
    pub fn load_delegation(&self) -> Result<Option<Value>, StoreError> {
        let path = self.dir.join(DELEGATION_FILE);
        Ok(read(&path)?.and_then(|text| serde_json::from_str(&text).ok()))
    }

    /// # Panics
    ///
    /// Never: a binding is strings and a time, which always encode as JSON.
    pub fn save_binding(&self, binding: Option<&Binding>) -> Result<(), StoreError> {
        let value = binding.map(|b| serde_json::to_value(b).expect("a binding serializes"));
        self.save(BINDING_FILE, value.as_ref())
    }

    pub fn save_delegation(&self, wire: Option<&Value>) -> Result<(), StoreError> {
        self.save(DELEGATION_FILE, wire)
    }

    fn save(&self, name: &str, value: Option<&Value>) -> Result<(), StoreError> {
        let path = self.dir.join(name);
        let io = |source| StoreError::Io {
            path: path.clone(),
            source,
        };
        let Some(value) = value else {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io(e)),
                _ => Ok(()),
            };
        };
        std::fs::create_dir_all(&self.dir).map_err(io)?;
        let text = serde_json::to_string_pretty(value).expect("JSON serializes");
        write_replacing(&path, text.as_bytes()).map_err(io)
    }
}

fn read(path: &Path) -> Result<Option<String>, StoreError> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(source) => Err(StoreError::Io {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// Write through a sibling file and rename it over `path`, so a crash
/// never leaves half a binding, and only the owner can change it.
fn write_replacing(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let tmp = path.with_extension(format!("{}.tmp", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let written = (|| {
        let mut file = options.open(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::BindingMode;

    #[arkavo_test_macros::spec("SKD-004")]
    #[test]
    fn a_corrupt_binding_is_an_error_not_an_absent_one() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path());
        assert!(store.load_binding().unwrap().is_none());
        let binding = Binding {
            orchestrator: "did:key:z6Mkorchestrator".into(),
            mode: BindingMode::FirstUse,
            since: chrono::Utc::now(),
        };
        store.save_binding(Some(&binding)).unwrap();
        assert_eq!(store.load_binding().unwrap(), Some(binding));

        std::fs::write(dir.path().join(BINDING_FILE), "{not json").unwrap();
        assert!(matches!(
            store.load_binding(),
            Err(StoreError::CorruptBinding { .. })
        ));

        std::fs::write(dir.path().join(DELEGATION_FILE), "{not json").unwrap();
        assert!(store.load_delegation().unwrap().is_none());
        store.save_binding(None).unwrap();
        store.save_binding(None).unwrap();
        assert!(store.load_binding().unwrap().is_none());
    }
}
