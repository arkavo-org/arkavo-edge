//! Screening the environment a kit declares for an MCP server.
//!
//! A kit is shared and content-addressed, so this is the only point where a
//! literal credential or a variable that makes the server load other code
//! can be refused before it is published or started.

use crate::runtime_config::{RuntimeMcpServer, RuntimeValidationError};

pub(crate) fn validate_mcp_server_env(
    server: &RuntimeMcpServer,
) -> Result<(), RuntimeValidationError> {
    if let Some(name) = server
        .env
        .keys()
        .chain(&server.env_passthrough)
        .find(|name| !arkavo_process_env::is_valid_name(name))
    {
        return Err(RuntimeValidationError::McpServerInvalidEnvName {
            server: server.name.clone(),
            name: name.clone(),
        });
    }
    if let Some(name) = server
        .env
        .keys()
        .chain(&server.env_passthrough)
        .find(|name| arkavo_process_env::is_loader_or_hijack_name(name))
    {
        return Err(RuntimeValidationError::McpServerLoaderEnvName {
            server: server.name.clone(),
            name: name.clone(),
        });
    }
    if let Some(name) = server
        .env
        .keys()
        .find(|name| arkavo_process_env::is_secret_name(name))
    {
        return Err(RuntimeValidationError::McpServerCredentialInEnv {
            server: server.name.clone(),
            name: name.clone(),
        });
    }
    Ok(())
}
