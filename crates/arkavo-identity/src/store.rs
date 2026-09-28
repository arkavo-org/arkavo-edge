//! Persist OIDC tokens under the platform data directory at mode `0o600`.

use crate::error::IdentityError;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: i64, // unix seconds
}

pub fn token_path() -> Result<PathBuf, IdentityError> {
    let base = if cfg!(target_os = "macos") {
        dirs::data_dir().map(|p| p.join("arkavo"))
    } else if cfg!(target_os = "windows") {
        dirs::data_local_dir().map(|p| p.join("arkavo"))
    } else {
        dirs::data_dir()
            .or_else(dirs::home_dir)
            .map(|p| p.join(".arkavo"))
    }
    .ok_or_else(|| IdentityError::Store("Could not determine data directory".into()))?;
    Ok(base.join("identity_token"))
}

pub fn save(tokens: &StoredTokens, path: &Path) -> Result<(), IdentityError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| IdentityError::Store(format!("create token dir: {e}")))?;
    }

    let json = serde_json::to_string_pretty(tokens)
        .map_err(|e| IdentityError::Store(format!("serialize tokens: {e}")))?;

    let tmp = {
        let mut name = path.as_os_str().to_os_string();
        name.push(".tmp");
        PathBuf::from(name)
    };
    // Leftover tmp must not keep a 0o644 mode; mode() applies only on create.
    let _ = fs::remove_file(&tmp);

    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }

    {
        let mut file = match opts.open(&tmp) {
            Ok(file) => file,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(IdentityError::Store(format!("open token file: {e}")));
            }
        };
        if let Err(e) = file.write_all(json.as_bytes()) {
            let _ = fs::remove_file(&tmp);
            return Err(IdentityError::Store(format!("write token file: {e}")));
        }
    }

    #[cfg(windows)]
    {
        // Windows `rename` cannot replace an existing destination.
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        IdentityError::Store(format!("replace token file: {e}"))
    })?;
    Ok(())
}

pub fn load(path: &Path) -> Result<Option<StoredTokens>, IdentityError> {
    if !path.exists() {
        return Ok(None);
    }
    let json = fs::read_to_string(path)
        .map_err(|e| IdentityError::Store(format!("read token file: {e}")))?;
    let tokens = serde_json::from_str(&json)
        .map_err(|e| IdentityError::Store(format!("parse token file: {e}")))?;
    Ok(Some(tokens))
}

pub fn delete(path: &Path) -> Result<(), IdentityError> {
    if path.exists() {
        fs::remove_file(path)
            .map_err(|e| IdentityError::Store(format!("delete token file: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_mode_600_at_creation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity_token");
        let tokens = StoredTokens {
            access_token: "atk".into(),
            refresh_token: Some("rtk".into()),
            expires_at: 1_700_000_000,
        };
        save(&tokens, &path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded.access_token, "atk");
        assert_eq!(loaded.refresh_token.as_deref(), Some("rtk"));
        delete(&path).unwrap();
        assert!(load(&path).unwrap().is_none());
    }

    #[test]
    fn token_path_is_arkavo_identity_token_under_data_dir() {
        let path = token_path().unwrap();
        assert_eq!(path.file_name().unwrap(), "identity_token");
        assert!(
            path.parent().unwrap().ends_with("arkavo")
                || path.parent().unwrap().ends_with(".arkavo")
        );
    }

    #[test]
    fn save_replaces_644_with_600_and_never_empties_dest() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("identity_token");
        std::fs::write(&path, b"stale-not-json").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o644);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        let tokens = StoredTokens {
            access_token: "atk2".into(),
            refresh_token: Some("rtk2".into()),
            expires_at: 1_800_000_000,
        };
        save(&tokens, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.is_empty(), "dest must not be left empty");
        let loaded = load(&path).unwrap().unwrap();
        assert_eq!(loaded.access_token, "atk2");
        assert_eq!(loaded.refresh_token.as_deref(), Some("rtk2"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(!dir.path().join("identity_token.tmp").exists());
    }
}
