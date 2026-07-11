use arkavo_protocol::error::Result;
use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_protocol::types::{
    AgentConfigGetRequest, AgentConfigGetResponse, AgentConfigRestoreRequest,
    AgentConfigRestoreResponse, AgentConfigUpdateRequest, AgentConfigUpdateResponse,
    AgentConfigValidateRequest, AgentConfigValidateResponse, ConfigBackup, ConfigError,
};
use jsonrpsee::types::ErrorObjectOwned;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{info, warn};

use super::super::config_helpers::{
    DEFAULT_KIT_PATH, cleanup_old_backups, kit_filename_of, resolve_kit_path, validate_kit_yaml,
};

pub async fn handle_config_get(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    request: AgentConfigGetRequest,
) -> std::result::Result<AgentConfigGetResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("agent_config_get".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let Some(kit_path) = resolve_kit_path() else {
        timer.error();
        return Err(ErrorObjectOwned::owned(
            -32002,
            "No SwarmKit kit configured",
            Some(serde_json::json!({"error": ConfigError::AgentOffline})),
        ));
    };

    let content = match tokio::fs::read_to_string(&kit_path).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            timer.error();
            return Err(ErrorObjectOwned::owned(
                -32002,
                "No SwarmKit kit configured",
                Some(serde_json::json!({"error": ConfigError::AgentOffline})),
            ));
        }
        Err(e) => {
            timer.error();
            return Err(ErrorObjectOwned::owned(
                -32603,
                format!("Failed to read kit configuration: {e}"),
                Some(serde_json::json!({"error": ConfigError::ReadOnlyFilesystem})),
            ));
        }
    };

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let version = format!("{:x}", hasher.finalize());

    let kit_filename = kit_filename_of(&kit_path);

    let backups = if request.include_backups {
        Some(list_backups(&kit_filename).await)
    } else {
        None
    };

    let writable = kit_path
        .metadata()
        .map(|m| !m.permissions().readonly())
        .unwrap_or(false);

    timer.success();
    Ok(AgentConfigGetResponse {
        content,
        version,
        backups,
        writable,
    })
}

async fn list_backups(kit_filename: &str) -> Vec<ConfigBackup> {
    let backup_dir = std::path::Path::new(".arkavo/backups");
    let mut backup_list = Vec::new();

    if !backup_dir.exists() {
        return backup_list;
    }

    let Ok(mut entries) = tokio::fs::read_dir(backup_dir).await else {
        return backup_list;
    };

    let prefix = format!("{kit_filename}.");

    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }

        let filename = entry.file_name().to_string_lossy().to_string();
        if !filename.starts_with(&prefix) || !filename.ends_with(".backup") {
            continue;
        }

        let timestamp_str = filename
            .strip_prefix(&prefix)
            .and_then(|s| s.strip_suffix(".backup"))
            .unwrap_or("");

        let timestamp = chrono::NaiveDateTime::parse_from_str(timestamp_str, "%Y-%m-%d-%H%M%S")
            .ok()
            .map(|dt| chrono::DateTime::<chrono::Utc>::from_naive_utc_and_offset(dt, chrono::Utc))
            .unwrap_or_else(chrono::Utc::now);

        if let Ok(backup_content) = tokio::fs::read_to_string(entry.path()).await {
            use sha2::{Digest, Sha256};
            let mut backup_hasher = Sha256::new();
            backup_hasher.update(backup_content.as_bytes());
            let backup_version = format!("{:x}", backup_hasher.finalize());

            backup_list.push(ConfigBackup {
                filename,
                timestamp,
                size: metadata.len(),
                version: backup_version,
            });
        }
    }

    backup_list.sort_by_key(|b| std::cmp::Reverse(b.timestamp));
    backup_list
}

pub async fn handle_config_update<F, Fut>(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    request: AgentConfigUpdateRequest,
    reload_fn: F,
) -> std::result::Result<AgentConfigUpdateResponse, ErrorObjectOwned>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<()>>,
{
    let timer = RpcTimer::new("agent_config_update".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    // No kit yet is not an error here: update creates one at the default
    // location, mirroring how the AGENTS.md-era update always targeted a
    // fixed default path regardless of whether that file already existed.
    let kit_path = resolve_kit_path().unwrap_or_else(|| PathBuf::from(DEFAULT_KIT_PATH));

    // Check expected version for optimistic locking
    if let Some(expected_version) = &request.expected_version
        && let Ok(current_content) = tokio::fs::read_to_string(&kit_path).await
    {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(current_content.as_bytes());
        let current_version = format!("{:x}", hasher.finalize());

        if &current_version != expected_version {
            timer.error();
            return Ok(AgentConfigUpdateResponse {
                success: false,
                new_version: None,
                backup_path: None,
                error: Some(ConfigError::Conflict { current_version }),
                reload_required: false,
            });
        }
    }

    if let Err(validation_error) = validate_kit_yaml(&request.content) {
        timer.error();
        return Ok(AgentConfigUpdateResponse {
            success: false,
            new_version: None,
            backup_path: None,
            error: Some(ConfigError::ValidationFailed {
                details: validation_error,
            }),
            reload_required: false,
        });
    }

    let backup_path = if request.create_backup {
        create_backup(&kit_path).await
    } else {
        None
    };

    if let Err(_e) = tokio::fs::write(&kit_path, &request.content).await {
        timer.error();
        return Ok(AgentConfigUpdateResponse {
            success: false,
            new_version: None,
            backup_path,
            error: Some(ConfigError::ReadOnlyFilesystem),
            reload_required: false,
        });
    }

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(request.content.as_bytes());
    let new_version = format!("{:x}", hasher.finalize());

    let reload_success = match reload_fn(request.content).await {
        Ok(_) => {
            info!("Kit configuration updated and reloaded successfully");
            true
        }
        Err(e) => {
            warn!(
                "Kit configuration saved but reload failed: {}. Agent restart may be required.",
                e
            );
            false
        }
    };

    timer.success();
    Ok(AgentConfigUpdateResponse {
        success: true,
        new_version: Some(new_version),
        backup_path,
        error: None,
        reload_required: !reload_success,
    })
}

async fn create_backup(kit_path: &std::path::Path) -> Option<String> {
    let backup_dir = std::path::Path::new(".arkavo/backups");
    if !backup_dir.exists()
        && let Err(e) = tokio::fs::create_dir_all(backup_dir).await
    {
        warn!("Failed to create backup directory: {}", e);
    }

    let kit_filename = kit_filename_of(kit_path);
    let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M%S");
    let backup_filename = format!("{kit_filename}.{timestamp}.backup");
    let backup_path = backup_dir.join(&backup_filename);

    if kit_path.exists() {
        if let Err(e) = tokio::fs::copy(kit_path, &backup_path).await {
            warn!("Failed to create backup: {}", e);
            None
        } else {
            cleanup_old_backups(backup_dir, 10, &kit_filename).await;
            Some(backup_path.to_string_lossy().to_string())
        }
    } else {
        None
    }
}

#[allow(clippy::unused_async)]
pub async fn handle_config_validate(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    request: AgentConfigValidateRequest,
) -> std::result::Result<AgentConfigValidateResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("agent_config_validate".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let mut errors = Vec::new();
    let warnings = Vec::new();

    if let Err(e) = arkavo_swarmkit::parse_yaml(&request.content) {
        errors.push(format!("Kit parse error: {e}"));
    }

    timer.success();
    Ok(AgentConfigValidateResponse {
        valid: errors.is_empty(),
        errors,
        warnings,
    })
}

pub async fn handle_config_restore(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    request: AgentConfigRestoreRequest,
) -> std::result::Result<AgentConfigRestoreResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("agent_config_restore".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let backup_dir = std::path::Path::new(".arkavo/backups");
    let backup_path = backup_dir.join(&request.backup_filename);

    if !backup_path.exists() {
        timer.error();
        return Ok(AgentConfigRestoreResponse {
            success: false,
            new_version: None,
            error: Some(ConfigError::ValidationFailed {
                details: format!("Backup file not found: {}", request.backup_filename),
            }),
        });
    }

    let backup_content = match tokio::fs::read_to_string(&backup_path).await {
        Ok(content) => content,
        Err(_e) => {
            timer.error();
            return Ok(AgentConfigRestoreResponse {
                success: false,
                new_version: None,
                error: Some(ConfigError::ReadOnlyFilesystem),
            });
        }
    };

    if let Err(validation_error) = validate_kit_yaml(&backup_content) {
        timer.error();
        return Ok(AgentConfigRestoreResponse {
            success: false,
            new_version: None,
            error: Some(ConfigError::ValidationFailed {
                details: format!("Backup validation failed: {validation_error}"),
            }),
        });
    }

    // Restore targets the same resolved kit path as get/update (falling
    // back to the default location if none is configured yet), unlike the
    // AGENTS.md-era restore which wrote to a bare top-level "AGENTS.md"
    // inconsistent with update's ".arkavo/AGENTS.md" target.
    let kit_path = resolve_kit_path().unwrap_or_else(|| PathBuf::from(DEFAULT_KIT_PATH));
    let kit_filename = kit_filename_of(&kit_path);

    // Create pre-restore backup
    if kit_path.exists() {
        let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M%S");
        let pre_restore_backup =
            backup_dir.join(format!("{kit_filename}.{timestamp}.pre-restore.backup"));
        let _ = tokio::fs::copy(&kit_path, &pre_restore_backup).await;
    }

    match tokio::fs::write(&kit_path, &backup_content).await {
        Ok(_) => {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(backup_content.as_bytes());
            let new_version = format!("{:x}", hasher.finalize());

            info!(
                "Kit configuration restored from backup: {}",
                request.backup_filename
            );

            timer.success();
            Ok(AgentConfigRestoreResponse {
                success: true,
                new_version: Some(new_version),
                error: None,
            })
        }
        Err(_e) => {
            timer.error();
            Ok(AgentConfigRestoreResponse {
                success: false,
                new_version: None,
                error: Some(ConfigError::ReadOnlyFilesystem),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_protocol::rate_limit::RateLimitConfig;
    use std::sync::Mutex;

    const MINIMAL_KIT_YAML: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "hello"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "say hello"
roles:
  - id: agent
    role_type: operator
    agent_provisioning: {}
    skills: []
    mcp_tools: []
    handoffs: []
coordination:
  topology: hub-spoke
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  data_classifications: ["public"]
  network:
    egress_allowed: false
    egress_allowlist: []
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures:
    - signer_did: "did:web:example.com"
      algorithm: ed25519
      signature: "AAA"
"#;

    const OTHER_VALID_KIT_YAML: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "second"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  expires: "2026-05-29T00:00:00Z"
  nonce: "abcdefghijklmnopqrstuv"
objective:
  goal: "say goodbye"
roles:
  - id: agent
    role_type: operator
    agent_provisioning: {}
    skills: []
    mcp_tools: []
    handoffs: []
coordination:
  topology: hub-spoke
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 8000
    max_cost_usd: 0.01
  data_classifications: ["public"]
  network:
    egress_allowed: false
    egress_allowlist: []
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures:
    - signer_did: "did:web:example.com"
      algorithm: ed25519
      signature: "AAA"
"#;

    fn create_test_metrics() -> Arc<MetricsCollector> {
        Arc::new(MetricsCollector::new(false))
    }

    fn create_test_rate_limiter() -> RateLimiter {
        RateLimiter::new(RateLimitConfig::default())
    }

    /// Serializes tests that change the process working directory — cwd is
    /// global process state, so parallel `cargo test` threads (unlike
    /// `cargo nextest`'s per-test processes) would otherwise race.
    static CWD_LOCK: Mutex<()> = Mutex::new(());

    struct TestCwd {
        _guard: std::sync::MutexGuard<'static, ()>,
        original: std::path::PathBuf,
        _tempdir: tempfile::TempDir,
    }

    impl TestCwd {
        fn new() -> Self {
            let guard = CWD_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let original = std::env::current_dir().unwrap();
            let tempdir = tempfile::tempdir().unwrap();
            std::env::set_current_dir(tempdir.path()).unwrap();
            std::fs::create_dir_all(".arkavo").unwrap();
            Self {
                _guard: guard,
                original,
                _tempdir: tempdir,
            }
        }
    }

    impl Drop for TestCwd {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.original);
        }
    }

    #[tokio::test]
    async fn get_returns_no_kit_error_when_none_configured() {
        let _cwd = TestCwd::new();
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        let err = handle_config_get(
            &metrics,
            &rate_limiter,
            AgentConfigGetRequest {
                agent_id: "agent".to_string(),
                include_backups: false,
            },
        )
        .await
        .unwrap_err();

        assert_eq!(err.code(), -32002);
    }

    #[tokio::test]
    async fn validate_accepts_valid_kit_and_rejects_invalid() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        let ok = handle_config_validate(
            &metrics,
            &rate_limiter,
            AgentConfigValidateRequest {
                agent_id: "agent".to_string(),
                content: MINIMAL_KIT_YAML.to_string(),
            },
        )
        .await
        .unwrap();
        assert!(ok.valid);
        assert!(ok.errors.is_empty());

        let bad = handle_config_validate(
            &metrics,
            &rate_limiter,
            AgentConfigValidateRequest {
                agent_id: "agent".to_string(),
                content: "not: valid: yaml: at: all:".to_string(),
            },
        )
        .await
        .unwrap();
        assert!(!bad.valid);
        assert!(!bad.errors.is_empty());
    }

    #[tokio::test]
    async fn update_rejects_invalid_content_without_writing() {
        let _cwd = TestCwd::new();
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        let response = handle_config_update(
            &metrics,
            &rate_limiter,
            AgentConfigUpdateRequest {
                agent_id: "agent".to_string(),
                content: "not: valid: yaml: at: all:".to_string(),
                expected_version: None,
                create_backup: true,
            },
            |_content| async { Ok(()) },
        )
        .await
        .unwrap();

        assert!(!response.success);
        assert!(matches!(
            response.error,
            Some(ConfigError::ValidationFailed { .. })
        ));
        assert!(!std::path::Path::new(DEFAULT_KIT_PATH).exists());
    }

    #[tokio::test]
    async fn update_creates_kit_at_default_path_when_none_exists() {
        let _cwd = TestCwd::new();
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        let response = handle_config_update(
            &metrics,
            &rate_limiter,
            AgentConfigUpdateRequest {
                agent_id: "agent".to_string(),
                content: MINIMAL_KIT_YAML.to_string(),
                expected_version: None,
                create_backup: true,
            },
            |_content| async { Ok(()) },
        )
        .await
        .unwrap();

        assert!(response.success, "update should succeed: {response:?}");
        assert!(response.new_version.is_some());
        // No prior file existed, so no backup should have been created.
        assert!(response.backup_path.is_none());
        assert!(std::path::Path::new(DEFAULT_KIT_PATH).exists());
    }

    #[tokio::test]
    async fn update_detects_optimistic_lock_conflict() {
        let _cwd = TestCwd::new();
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        // Seed an existing kit at the default path.
        std::fs::write(DEFAULT_KIT_PATH, MINIMAL_KIT_YAML).unwrap();

        let response = handle_config_update(
            &metrics,
            &rate_limiter,
            AgentConfigUpdateRequest {
                agent_id: "agent".to_string(),
                content: OTHER_VALID_KIT_YAML.to_string(),
                expected_version: Some("stale-version-hash".to_string()),
                create_backup: true,
            },
            |_content| async { Ok(()) },
        )
        .await
        .unwrap();

        assert!(!response.success);
        assert!(matches!(response.error, Some(ConfigError::Conflict { .. })));
        // Content on disk must be unchanged.
        assert_eq!(
            std::fs::read_to_string(DEFAULT_KIT_PATH).unwrap(),
            MINIMAL_KIT_YAML
        );
    }

    #[tokio::test]
    async fn update_creates_backup_with_kit_filename() {
        let _cwd = TestCwd::new();
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        std::fs::write(DEFAULT_KIT_PATH, MINIMAL_KIT_YAML).unwrap();

        let response = handle_config_update(
            &metrics,
            &rate_limiter,
            AgentConfigUpdateRequest {
                agent_id: "agent".to_string(),
                content: OTHER_VALID_KIT_YAML.to_string(),
                expected_version: None,
                create_backup: true,
            },
            |_content| async { Ok(()) },
        )
        .await
        .unwrap();

        assert!(response.success, "update should succeed: {response:?}");
        let backup_path = response.backup_path.expect("backup should be created");
        assert!(backup_path.contains("agent.swarmkit.yaml."));
        assert!(backup_path.ends_with(".backup"));
        assert!(std::path::Path::new(&backup_path).exists());
        assert_eq!(
            std::fs::read_to_string(&backup_path).unwrap(),
            MINIMAL_KIT_YAML
        );
    }

    #[tokio::test]
    async fn get_round_trips_after_update() {
        let _cwd = TestCwd::new();
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        std::fs::write(DEFAULT_KIT_PATH, MINIMAL_KIT_YAML).unwrap();

        let response = handle_config_get(
            &metrics,
            &rate_limiter,
            AgentConfigGetRequest {
                agent_id: "agent".to_string(),
                include_backups: true,
            },
        )
        .await
        .unwrap();

        assert_eq!(response.content, MINIMAL_KIT_YAML);
        assert!(!response.version.is_empty());
        assert!(response.backups.is_some());
    }

    #[tokio::test]
    async fn restore_round_trips_from_backup() {
        let _cwd = TestCwd::new();
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        std::fs::write(DEFAULT_KIT_PATH, MINIMAL_KIT_YAML).unwrap();

        let update_response = handle_config_update(
            &metrics,
            &rate_limiter,
            AgentConfigUpdateRequest {
                agent_id: "agent".to_string(),
                content: OTHER_VALID_KIT_YAML.to_string(),
                expected_version: None,
                create_backup: true,
            },
            |_content| async { Ok(()) },
        )
        .await
        .unwrap();
        let backup_path = update_response.backup_path.expect("backup created");
        let backup_filename = std::path::Path::new(&backup_path)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();

        let restore_response = handle_config_restore(
            &metrics,
            &rate_limiter,
            AgentConfigRestoreRequest {
                agent_id: "agent".to_string(),
                backup_filename,
            },
        )
        .await
        .unwrap();

        assert!(
            restore_response.success,
            "restore should succeed: {restore_response:?}"
        );
        assert_eq!(
            std::fs::read_to_string(DEFAULT_KIT_PATH).unwrap(),
            MINIMAL_KIT_YAML
        );
    }

    #[tokio::test]
    async fn restore_rejects_missing_backup() {
        let _cwd = TestCwd::new();
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        let response = handle_config_restore(
            &metrics,
            &rate_limiter,
            AgentConfigRestoreRequest {
                agent_id: "agent".to_string(),
                backup_filename: "agent.swarmkit.yaml.does-not-exist.backup".to_string(),
            },
        )
        .await
        .unwrap();

        assert!(!response.success);
        assert!(matches!(
            response.error,
            Some(ConfigError::ValidationFailed { .. })
        ));
    }
}
