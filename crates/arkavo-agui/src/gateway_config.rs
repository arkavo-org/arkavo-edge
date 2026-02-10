use crate::agent_connection::AgentConnection;
use crate::types::*;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

pub async fn handle_get_config(
    agent_id: String,
    include_backups: bool,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let agent_conns = agent_connections.read().await;
    if let Some(agent_conn) = agent_conns.get(&agent_id) {
        match agent_conn.get_config(include_backups).await {
            Ok(r) => {
                tx.send(AgUiEvent::AgentConfigSnapshot {
                    content: r.content,
                    version: r.version,
                    backups: r.backups.map(|b| {
                        b.into_iter()
                            .map(|b| ConfigBackupInfo {
                                filename: b.filename,
                                timestamp: b.timestamp,
                                size: b.size,
                                version: b.version,
                            })
                            .collect()
                    }),
                    writable: r.writable,
                })
                .await?;
            }
            Err(e) => {
                tx.send(AgUiEvent::Error {
                    code: "CONFIG_GET_FAILED".to_string(),
                    message: format!("Failed to get configuration: {e}"),
                })
                .await?;
            }
        }
    } else {
        tx.send(not_connected(&agent_id)).await?;
    }
    Ok(())
}

pub async fn handle_update_config(
    agent_id: String,
    content: String,
    expected_version: Option<String>,
    create_backup: bool,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_protocol::types::ConfigError;
    let agent_conns = agent_connections.read().await;
    if let Some(agent_conn) = agent_conns.get(&agent_id) {
        match agent_conn
            .update_config(content, expected_version, create_backup)
            .await
        {
            Ok(r) => {
                tx.send(AgUiEvent::ConfigUpdateResult {
                    success: r.success,
                    new_version: r.new_version,
                    backup_path: r.backup_path,
                    error: r.error.map(|e| match e {
                        ConfigError::AgentOffline => ConfigErrorInfo::AgentOffline,
                        ConfigError::ReadOnlyFilesystem => ConfigErrorInfo::ReadOnlyFilesystem,
                        ConfigError::ValidationFailed { details } => {
                            ConfigErrorInfo::ValidationFailed { details }
                        }
                        ConfigError::Conflict { current_version } => {
                            ConfigErrorInfo::Conflict { current_version }
                        }
                        ConfigError::Unauthorized => ConfigErrorInfo::Unauthorized,
                    }),
                    reload_required: r.reload_required,
                })
                .await?;
            }
            Err(e) => {
                tx.send(AgUiEvent::Error {
                    code: "CONFIG_UPDATE_FAILED".to_string(),
                    message: format!("Failed to update configuration: {e}"),
                })
                .await?;
            }
        }
    } else {
        tx.send(not_connected(&agent_id)).await?;
    }
    Ok(())
}

pub async fn handle_validate_config(
    agent_id: String,
    content: String,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let agent_conns = agent_connections.read().await;
    if let Some(agent_conn) = agent_conns.get(&agent_id) {
        match agent_conn.validate_config(content).await {
            Ok(r) => {
                tx.send(AgUiEvent::ConfigValidationResult {
                    valid: r.valid,
                    errors: r.errors,
                    warnings: r.warnings,
                })
                .await?;
            }
            Err(e) => {
                tx.send(AgUiEvent::Error {
                    code: "CONFIG_VALIDATE_FAILED".to_string(),
                    message: format!("Failed to validate configuration: {e}"),
                })
                .await?;
            }
        }
    } else {
        tx.send(not_connected(&agent_id)).await?;
    }
    Ok(())
}

pub async fn handle_restore_config(
    agent_id: String,
    backup_filename: String,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_protocol::types::ConfigError;
    let agent_conns = agent_connections.read().await;
    if let Some(agent_conn) = agent_conns.get(&agent_id) {
        match agent_conn.restore_config(backup_filename).await {
            Ok(r) => {
                tx.send(AgUiEvent::ConfigRestoreResult {
                    success: r.success,
                    new_version: r.new_version,
                    error: r.error.map(|e| match e {
                        ConfigError::AgentOffline => ConfigErrorInfo::AgentOffline,
                        ConfigError::ReadOnlyFilesystem => ConfigErrorInfo::ReadOnlyFilesystem,
                        ConfigError::ValidationFailed { details } => {
                            ConfigErrorInfo::ValidationFailed { details }
                        }
                        ConfigError::Conflict { current_version } => {
                            ConfigErrorInfo::Conflict { current_version }
                        }
                        ConfigError::Unauthorized => ConfigErrorInfo::Unauthorized,
                    }),
                })
                .await?;
            }
            Err(e) => {
                tx.send(AgUiEvent::Error {
                    code: "CONFIG_RESTORE_FAILED".to_string(),
                    message: format!("Failed to restore configuration: {e}"),
                })
                .await?;
            }
        }
    } else {
        tx.send(not_connected(&agent_id)).await?;
    }
    Ok(())
}

fn not_connected(agent_id: &str) -> AgUiEvent {
    AgUiEvent::Error {
        code: "AGENT_NOT_CONNECTED".to_string(),
        message: format!("Agent {agent_id} is not connected"),
    }
}
