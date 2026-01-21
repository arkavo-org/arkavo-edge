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
    let token: StoredToken = serde_json::from_str(&json)?;

    // Auto-cleanup expired tokens
    if token.is_expired() {
        let _ = delete_token().await;
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
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use std::sync::Mutex;

    // Use a mutex to serialize storage tests since they share the same file
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[tokio::test]
    async fn test_token_storage_roundtrip() {
        let _guard = TEST_LOCK.lock().unwrap();

        // Cleanup before test
        let _ = delete_token().await;

        let token = StoredToken::new(
            "test_token".to_string(),
            "did:key:z6MkTest".to_string(),
            Utc::now() + Duration::hours(1),
            vec!["agent.capability.chat".to_string()],
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
        let _guard = TEST_LOCK.lock().unwrap();

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
}
