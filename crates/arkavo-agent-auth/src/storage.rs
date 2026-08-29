use crate::{error::AgentAuthError, types::StoredToken};
use std::path::{Path, PathBuf};
use tokio::io::AsyncWriteExt;

/// Get the platform-specific token storage path.
pub(crate) fn get_token_path() -> Result<PathBuf, AgentAuthError> {
    let base_dir = if cfg!(target_os = "macos") {
        dirs::data_dir()
            .map(|p| p.join("arkavo"))
            .ok_or_else(|| AgentAuthError::Storage("Could not determine data directory".into()))?
    } else if cfg!(target_os = "windows") {
        dirs::data_local_dir()
            .map(|p| p.join("arkavo"))
            .ok_or_else(|| {
                AgentAuthError::Storage("Could not determine local data directory".into())
            })?
    } else {
        // Linux and other Unix-like systems
        dirs::data_dir()
            .or_else(dirs::home_dir)
            .map(|p| p.join(".arkavo"))
            .ok_or_else(|| AgentAuthError::Storage("Could not determine data directory".into()))?
    };

    Ok(base_dir.join("agent_token"))
}

/// Parse a stored token from its on-disk JSON representation. Shared by the
/// async and blocking readers so their notion of "what a stored token looks
/// like" cannot drift apart.
fn parse_stored_token(json: &str) -> Result<StoredToken, AgentAuthError> {
    Ok(serde_json::from_str(json)?)
}

/// Store a token to disk.
pub async fn store_token(token: &StoredToken) -> Result<(), AgentAuthError> {
    let path = get_token_path()?;

    // Create parent directory if it doesn't exist
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let json = serde_json::to_string_pretty(token)?;
    write_private(&path, json.as_bytes()).await
}

/// Replace `path` with `bytes` so the token is never readable by anyone but
/// its owner, not even for an instant: the content is staged in a sibling file
/// created with mode 0600 and then renamed over the target. Writing in place
/// and tightening the mode afterwards leaves a world-readable window over a
/// bearer credential, and lets the concurrent refresh loop read a half-written
/// file.
async fn write_private(path: &Path, bytes: &[u8]) -> Result<(), AgentAuthError> {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let stem = path.file_name().and_then(|n| n.to_str()).unwrap_or("token");
    let tmp = path.with_file_name(format!(
        ".{stem}.{}.{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    // A leftover from a killed run would defeat `create_new`, which is what
    // guarantees the mode is applied rather than inherited.
    let _ = tokio::fs::remove_file(&tmp).await;

    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);

    let staged = async {
        let mut file = options.open(&tmp).await?;
        file.write_all(bytes).await?;
        file.sync_all().await?;
        drop(file);
        tokio::fs::rename(&tmp, path).await
    }
    .await;

    if staged.is_err() {
        let _ = tokio::fs::remove_file(&tmp).await;
    }

    Ok(staged?)
}

/// Load a token from disk.
pub async fn load_token() -> Result<Option<StoredToken>, AgentAuthError> {
    let path = get_token_path()?;

    if !path.exists() {
        return Ok(None);
    }

    let json = tokio::fs::read_to_string(&path).await?;
    let token = parse_stored_token(&json)?;

    // Auto-cleanup expired tokens
    if token.is_expired() {
        let _ = delete_token().await;
        return Ok(None);
    }

    Ok(Some(token))
}

/// Synchronous counterpart to [`load_token`], for callers that must stay
/// sync (e.g. `arkavo-config-encryption`'s `KasConfig::from_env`, which has
/// its own non-async callers). Bridging to the async reader from a sync
/// context would need a runtime handle, and `block_on`/`block_in_place`
/// panics on a current-thread runtime — unacceptable in a config
/// constructor — so this reads the same path with `std::fs` instead and
/// shares `get_token_path`/`parse_stored_token` with the async path so the
/// two readers cannot drift apart.
///
/// Never returns an expired token; an expired token file is removed as a
/// side effect, same as `load_token`.
pub fn load_token_blocking() -> Result<Option<StoredToken>, AgentAuthError> {
    let path = get_token_path()?;

    if !path.exists() {
        return Ok(None);
    }

    let json = std::fs::read_to_string(&path)?;
    let token = parse_stored_token(&json)?;

    if token.is_expired() {
        let _ = std::fs::remove_file(&path);
        return Ok(None);
    }

    Ok(Some(token))
}

/// Delete the stored token.
pub async fn delete_token() -> Result<(), AgentAuthError> {
    let path = get_token_path()?;

    if path.exists() {
        tokio::fs::remove_file(&path).await?;
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::test_helpers::TEST_LOCK;
    use crate::test_utils::TokenFileGuard;
    use arkavo_test_macros::spec;
    use chrono::{Duration, Utc};

    #[tokio::test]
    async fn test_token_storage_roundtrip() {
        let _lock = TEST_LOCK.lock().await;
        let _file = TokenFileGuard::capture();

        let token = StoredToken::new(
            "test_token".to_string(),
            "did:key:z6MkTest".to_string(),
            Utc::now() + Duration::hours(1),
            vec!["https://arkavo.ai/attr/tdf/value/decrypt".to_string()],
        );

        // Store
        store_token(&token).await.unwrap();

        // Load
        let loaded = load_token().await.unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.token, token.token);
        assert_eq!(loaded.did, token.did);

        // Cleanup
        delete_token().await.unwrap();

        // Verify deleted
        let loaded = load_token().await.unwrap();
        assert!(loaded.is_none());
    }

    #[tokio::test]
    async fn test_expired_token_cleanup() {
        let _lock = TEST_LOCK.lock().await;
        let _file = TokenFileGuard::capture();

        let token = StoredToken::new(
            "expired_token".to_string(),
            "did:key:z6MkExpired".to_string(),
            Utc::now() - Duration::hours(1), // Already expired
            vec![],
        );

        // Store expired token (bypass the is_expired check in store)
        store_token(&token).await.unwrap();

        // Loading should return None and delete the file
        let loaded = load_token().await.unwrap();
        assert!(loaded.is_none());

        // Verify file was actually deleted
        let path = get_token_path().unwrap();
        assert!(!path.exists());
    }

    /// Test AAUTH-002: Store token securely.
    ///
    /// The current implementation persists the token as JSON on disk and sets
    /// restrictive Unix permissions (0o600). Full encryption of the token file is
    /// not yet implemented, so this test verifies the implemented safeguards:
    /// platform-specific path, metadata preservation, and restricted access bits.
    #[spec("AAUTH-002")]
    #[tokio::test]
    async fn test_store_token_securely() {
        let _lock = TEST_LOCK.lock().await;
        let _file = TokenFileGuard::capture();

        let expires = Utc::now() + Duration::hours(1);
        let token = StoredToken::new(
            "secure-token-value".to_string(),
            "did:key:z6MkSecure".to_string(),
            expires,
            vec![
                "https://arkavo.ai/attr/tdf/value/decrypt".to_string(),
                "https://arkavo.ai/attr/action/value/read".to_string(),
            ],
        );

        store_token(&token).await.unwrap();

        let path = get_token_path().unwrap();
        assert!(
            path.exists(),
            "token file should be written to platform storage"
        );

        // Verify restrictive permissions on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&path).unwrap();
            let mode = metadata.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "token file should be readable only by owner");
        }

        // Verify metadata is preserved.
        let json = tokio::fs::read_to_string(&path).await.unwrap();
        let parsed: StoredToken = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.token, token.token);
        assert_eq!(parsed.did, token.did);
        assert_eq!(parsed.expires_at.timestamp(), token.expires_at.timestamp());
        assert_eq!(parsed.entitlements, token.entitlements);
        assert!(parsed.stored_at <= Utc::now());

        // Regression guard for C1: unlike `TokenResponse`, `StoredToken` must
        // keep chrono's default RFC3339 string encoding on disk. Changing it
        // would orphan tokens already written by installed builds.
        let raw: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(
            raw["expires_at"].is_string(),
            "StoredToken.expires_at must stay RFC3339 on disk, not an integer"
        );
    }

    /// Regression: the token used to be written with `fs::write` and only then
    /// chmod-ed to 0600, so it existed world-readable for an instant and
    /// inherited a pre-existing file's wider mode until the chmod landed. The
    /// replacement is now staged in a sibling created 0600 and renamed over the
    /// target, so a wider mode on the old file cannot survive the write and no
    /// staging file is left behind for the refresh loop to trip over.
    #[cfg(unix)]
    #[tokio::test]
    async fn store_token_never_inherits_a_world_readable_mode() {
        use std::os::unix::fs::PermissionsExt;

        let _lock = TEST_LOCK.lock().await;
        let _file = TokenFileGuard::capture();

        let path = get_token_path().unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        let token = StoredToken::new(
            "replacement-token".to_string(),
            "did:key:z6MkReplacement".to_string(),
            Utc::now() + Duration::hours(1),
            vec![],
        );
        store_token(&token).await.unwrap();

        assert_eq!(load_token().await.unwrap().unwrap().token, token.token);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600,
            "a replacement must be created 0600, never inherit the old mode"
        );

        let leftovers: Vec<String> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "staging files left behind: {leftovers:?}"
        );
    }

    /// Regression test for C1: the sync reader used by
    /// `KasConfig::from_env` must see exactly what the async reader wrote,
    /// via the shared `parse_stored_token` helper.
    #[spec("AAUTH-002")]
    #[tokio::test]
    async fn load_token_blocking_reads_what_store_token_wrote() {
        let _lock = TEST_LOCK.lock().await;
        let _file = TokenFileGuard::capture();

        let token = StoredToken::new(
            "blocking-read-token".to_string(),
            "did:key:z6MkBlocking".to_string(),
            Utc::now() + Duration::hours(1),
            vec!["https://arkavo.ai/attr/tdf/value/decrypt".to_string()],
        );
        store_token(&token).await.unwrap();

        let loaded = load_token_blocking().unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.token, token.token);
        assert_eq!(loaded.did, token.did);
    }

    /// Regression test for C1: an expired token must never be handed back
    /// by the blocking reader (matching `load_token`'s behavior), and the
    /// stale file is removed as a side effect.
    #[spec("AAUTH-002")]
    #[tokio::test]
    async fn load_token_blocking_ignores_expired_token() {
        let _lock = TEST_LOCK.lock().await;
        let _file = TokenFileGuard::capture();

        let token = StoredToken::new(
            "expired-blocking-token".to_string(),
            "did:key:z6MkExpiredBlocking".to_string(),
            Utc::now() - Duration::hours(1),
            vec![],
        );
        store_token(&token).await.unwrap();

        assert!(load_token_blocking().unwrap().is_none());

        let path = get_token_path().unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn load_token_blocking_returns_none_when_no_file_exists() {
        let _lock = TEST_LOCK.lock().await;
        let _file = TokenFileGuard::capture();

        assert!(load_token_blocking().unwrap().is_none());
    }
}
