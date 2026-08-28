use crate::{error::AgentAuthError, types::StoredToken};
use std::path::PathBuf;

/// Get the platform-specific token storage path.
fn get_token_path() -> Result<PathBuf, AgentAuthError> {
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
    tokio::fs::write(&path, &json).await?;

    // Set restrictive permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = std::fs::Permissions::from_mode(0o600);
        tokio::fs::set_permissions(&path, perms).await?;
    }

    Ok(())
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
    use arkavo_test_macros::spec;
    use chrono::{Duration, Utc};

    #[tokio::test]
    async fn test_token_storage_roundtrip() {
        let _guard = TEST_LOCK.lock().await;

        // Cleanup before test
        let _ = delete_token().await;

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
        let _guard = TEST_LOCK.lock().await;

        // Cleanup before test to ensure no stale tokens
        let _ = delete_token().await;

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
        let _guard = TEST_LOCK.lock().await;
        let _ = delete_token().await;

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

        delete_token().await.unwrap();
    }

    /// Regression test for C1: the sync reader used by
    /// `KasConfig::from_env` must see exactly what the async reader wrote,
    /// via the shared `parse_stored_token` helper.
    #[spec("AAUTH-002")]
    #[tokio::test]
    async fn load_token_blocking_reads_what_store_token_wrote() {
        let _guard = TEST_LOCK.lock().await;
        let _ = delete_token().await;

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

        delete_token().await.unwrap();
    }

    /// Regression test for C1: an expired token must never be handed back
    /// by the blocking reader (matching `load_token`'s behavior), and the
    /// stale file is removed as a side effect.
    #[spec("AAUTH-002")]
    #[tokio::test]
    async fn load_token_blocking_ignores_expired_token() {
        let _guard = TEST_LOCK.lock().await;
        let _ = delete_token().await;

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
        let _guard = TEST_LOCK.lock().await;
        let _ = delete_token().await;

        assert!(load_token_blocking().unwrap().is_none());
    }
}
