//! Development key files: one Ed25519 seed per file, base64url, created with mode 600 and
//! never overwritten. On Unix a key file that others can read is refused.

use anyhow::{Context, bail};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::SigningKey;
use rand::RngCore as _;
use rand::rngs::OsRng;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::Path;

/// Generates a key and writes it to `path`. Fails if the file exists.
pub fn create(path: &Path) -> anyhow::Result<SigningKey> {
    let mut seed = [0u8; 32];
    OsRng.fill_bytes(&mut seed);
    let key = SigningKey::from_bytes(&seed);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path).with_context(|| {
        format!(
            "creating {} (existing keys are never overwritten)",
            path.display()
        )
    })?;
    writeln!(file, "{}", URL_SAFE_NO_PAD.encode(key.to_bytes()))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(key)
}

/// Reads the key at `path`.
pub fn load(path: &Path) -> anyhow::Result<SigningKey> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(path)
            .with_context(|| format!("reading {}", path.display()))?
            .permissions()
            .mode();
        if mode & 0o077 != 0 {
            bail!(
                "{} is readable by others (mode {:o}); run chmod 600",
                path.display(),
                mode & 0o777
            );
        }
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let seed: [u8; 32] = URL_SAFE_NO_PAD
        .decode(text.trim())
        .ok()
        .and_then(|b| b.try_into().ok())
        .with_context(|| format!("{} is not a key file", path.display()))?;
    Ok(SigningKey::from_bytes(&seed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_once_and_loads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.key");
        let key = create(&path).unwrap();
        assert_eq!(load(&path).unwrap().to_bytes(), key.to_bytes());
        assert!(create(&path).is_err(), "never overwrites");
        let other = create(&dir.path().join("b.key")).unwrap();
        assert_ne!(other.to_bytes(), key.to_bytes());
    }

    #[test]
    fn refuses_unreadable_or_exposed_files() {
        let dir = tempfile::tempdir().unwrap();
        let junk = dir.path().join("junk.key");
        std::fs::write(&junk, "not a key").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&junk, std::fs::Permissions::from_mode(0o600)).unwrap();
            assert!(load(&junk).is_err());
            let path = dir.path().join("open.key");
            create(&path).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
            assert!(
                load(&path)
                    .unwrap_err()
                    .to_string()
                    .contains("readable by others")
            );
        }
        assert!(load(&dir.path().join("missing.key")).is_err());
    }
}
