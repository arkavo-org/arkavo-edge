use arkavo_protocol::agent_config::parse_agents_config;
use arkavo_protocol::error::{A2aError, Result};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_protocol::types::{
    AgentConfigGetRequest, AgentConfigGetResponse, AgentConfigRestoreRequest,
    AgentConfigRestoreResponse, AgentConfigUpdateRequest, AgentConfigUpdateResponse,
    AgentConfigValidateRequest, AgentConfigValidateResponse, ConfigBackup, ConfigError,
};
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;
use tracing::{info, warn};

use super::super::config_helpers::{AgentMetadata, cleanup_old_backups, validate_agent_config};

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

    let config_path = std::path::Path::new(".arkavo/AGENTS.md");
    let content = match tokio::fs::read_to_string(config_path).await {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            timer.error();
            return Err(ErrorObjectOwned::owned(
                -32002,
                "Configuration file not found",
                Some(serde_json::json!({"error": ConfigError::AgentOffline})),
            ));
        }
        Err(e) => {
            timer.error();
            return Err(ErrorObjectOwned::owned(
                -32603,
                format!("Failed to read configuration: {e}"),
                Some(serde_json::json!({"error": ConfigError::ReadOnlyFilesystem})),
            ));
        }
    };

    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    let version = format!("{:x}", hasher.finalize());

    let backups = if request.include_backups {
        Some(list_backups().await)
    } else {
        None
    };

    let writable = config_path
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

async fn list_backups() -> Vec<ConfigBackup> {
    let backup_dir = std::path::Path::new(".arkavo/backups");
    let mut backup_list = Vec::new();

    if !backup_dir.exists() {
        return backup_list;
    }

    let Ok(mut entries) = tokio::fs::read_dir(backup_dir).await else {
        return backup_list;
    };

    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }

        let filename = entry.file_name().to_string_lossy().to_string();
        if !filename.starts_with("AGENTS.md.") || !filename.ends_with(".backup") {
            continue;
        }

        let timestamp_str = filename
            .strip_prefix("AGENTS.md.")
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

    backup_list.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
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

    let config_path = std::path::Path::new(".arkavo/AGENTS.md");

    // Check expected version for optimistic locking
    if let Some(expected_version) = &request.expected_version
        && let Ok(current_content) = tokio::fs::read_to_string(config_path).await
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

    if let Err(validation_error) = validate_agent_config(&request.content) {
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
        create_backup(config_path).await
    } else {
        None
    };

    if let Err(_e) = tokio::fs::write(config_path, &request.content).await {
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
            info!("Configuration updated and reloaded successfully");
            true
        }
        Err(e) => {
            warn!(
                "Configuration saved but reload failed: {}. Agent restart may be required.",
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

async fn create_backup(config_path: &std::path::Path) -> Option<String> {
    let backup_dir = std::path::Path::new(".arkavo/backups");
    if !backup_dir.exists()
        && let Err(e) = tokio::fs::create_dir_all(backup_dir).await
    {
        warn!("Failed to create backup directory: {}", e);
    }

    let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M%S");
    let backup_filename = format!("AGENTS.md.{timestamp}.backup");
    let backup_path = backup_dir.join(&backup_filename);

    if config_path.exists() {
        if let Err(e) = tokio::fs::copy(config_path, &backup_path).await {
            warn!("Failed to create backup: {}", e);
            None
        } else {
            cleanup_old_backups(backup_dir, 10).await;
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
    let mut warnings = Vec::new();

    match parse_agents_config(&request.content) {
        Ok(agents) => {
            if agents.is_empty() {
                errors.push("No agent configurations found".to_string());
            } else {
                for agent in agents {
                    if agent.name.is_empty() {
                        errors.push("Agent name is required".to_string());
                    }
                    if agent.purpose.is_empty() {
                        warnings.push(format!("Agent '{}' has no purpose defined", agent.name));
                    }
                    if agent.model.is_empty() {
                        errors.push(format!("Agent '{}' requires a model", agent.name));
                    }
                    if agent.listen.is_empty() {
                        errors.push(format!("Agent '{}' requires a listen address", agent.name));
                    }

                    if !agent.listen.is_empty()
                        && agent.listen.parse::<std::net::SocketAddr>().is_err()
                    {
                        errors.push(format!(
                            "Agent '{}' has invalid listen address: {}",
                            agent.name, agent.listen
                        ));
                    }

                    if !agent.model.is_empty() && !agent.model.contains("://") {
                        warnings.push(format!(
                            "Agent '{}' model should use protocol format (e.g., ollama://...)",
                            agent.name
                        ));
                    }
                }
            }
        }
        Err(e) => {
            errors.push(format!("Configuration parse error: {e}"));
        }
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

    if let Err(validation_error) = validate_agent_config(&backup_content) {
        timer.error();
        return Ok(AgentConfigRestoreResponse {
            success: false,
            new_version: None,
            error: Some(ConfigError::ValidationFailed {
                details: format!("Backup validation failed: {validation_error}"),
            }),
        });
    }

    // Create pre-restore backup
    let config_path = std::path::Path::new("AGENTS.md");
    if config_path.exists() {
        let timestamp = chrono::Utc::now().format("%Y-%m-%d-%H%M%S");
        let pre_restore_backup =
            backup_dir.join(format!("AGENTS.md.{timestamp}.pre-restore.backup"));
        let _ = tokio::fs::copy(config_path, &pre_restore_backup).await;
    }

    match tokio::fs::write(config_path, &backup_content).await {
        Ok(_) => {
            use sha2::{Digest, Sha256};
            let mut hasher = Sha256::new();
            hasher.update(backup_content.as_bytes());
            let new_version = format!("{:x}", hasher.finalize());

            info!(
                "Configuration restored from backup: {}",
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

pub async fn reload_configuration(
    content: &str,
    agent_metadata: &Arc<tokio::sync::RwLock<AgentMetadata>>,
    has_llm_adapter: bool,
    mcp_registry: &Arc<McpRegistry>,
) -> Result<()> {
    if content.trim().is_empty() {
        return Err(A2aError::Configuration(
            "Configuration file is empty".to_string(),
        ));
    }

    let agents = parse_agents_config(content)
        .map_err(|e| A2aError::Configuration(format!("Failed to parse configuration: {e}")))?;

    if agents.is_empty() {
        return Err(A2aError::Configuration(
            "No agent configurations found".to_string(),
        ));
    }

    let metadata_read = agent_metadata.read().await;
    let our_agent_name = metadata_read.name.clone();
    let old_model = metadata_read.model.clone();
    drop(metadata_read);

    let new_config = agents
        .iter()
        .find(|a| a.name == our_agent_name)
        .ok_or_else(|| {
            A2aError::Configuration(format!(
                "Agent '{our_agent_name}' not found in new configuration"
            ))
        })?;

    if new_config.purpose.is_empty() {
        return Err(A2aError::Configuration(
            "Agent purpose cannot be empty".to_string(),
        ));
    }
    if new_config.model.is_empty() {
        return Err(A2aError::Configuration(
            "Agent model cannot be empty".to_string(),
        ));
    }

    {
        let mut metadata = agent_metadata.write().await;
        metadata.purpose.clone_from(&new_config.purpose);
        metadata.endpoint.clone_from(&new_config.listen);
        metadata.api_keys.clone_from(&new_config.api_keys);

        info!("Updated agent metadata for '{}'", metadata.name);
        info!("  Purpose: {}", metadata.purpose);
        info!("  Endpoint: {}", metadata.endpoint);
        info!("  API keys updated: {}", metadata.api_keys.len());
    }

    if new_config.model != old_model && has_llm_adapter {
        warn!(
            "Model change detected from '{}' to '{}', but LLM adapter recreation not yet implemented",
            old_model, new_config.model
        );

        let mut metadata = agent_metadata.write().await;
        metadata.model.clone_from(&new_config.model);
    }

    if !new_config.mcp_servers.is_empty() {
        info!("MCP server configuration changed, clearing existing connections");
        mcp_registry.clear_connections().await;

        for mcp_server in &new_config.mcp_servers {
            info!("MCP server '{}' needs restart:", mcp_server.name);
            if let Some(cmd) = &mcp_server.command {
                info!("  Command: {} {:?}", cmd, mcp_server.args);
            } else if let Some(url) = &mcp_server.url {
                info!("  URL: {}", url);
            }
        }

        warn!("MCP servers cleared. Manual restart of MCP servers required for full reload");
    }

    info!("Configuration hot-reload completed successfully");
    Ok(())
}
