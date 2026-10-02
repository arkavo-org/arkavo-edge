//! Screening the environment a kit declares for an MCP server.
//!
//! A kit is shared and content-addressed, so this is the only point where a
//! literal credential or a variable that makes the server load other code
//! can be refused before it is published or started.

use crate::runtime_config::{RuntimeMcpServer, RuntimeValidationError};
use arkavo_process_env::{EnvRefusal, screen_entry};

pub(crate) fn validate_mcp_server_env(
    server: &RuntimeMcpServer,
) -> Result<(), RuntimeValidationError> {
    let literals = server.env.keys().map(|name| (name, true));
    let passed_through = server.env_passthrough.iter().map(|name| (name, false));
    for (name, literal) in literals.chain(passed_through) {
        let server = server.name.clone();
        match screen_entry(name, literal) {
            Ok(()) => {}
            Err(EnvRefusal::InvalidName) => {
                return Err(RuntimeValidationError::McpServerInvalidEnvName {
                    server,
                    name: name.clone(),
                });
            }
            Err(EnvRefusal::LoaderName(name)) => {
                return Err(RuntimeValidationError::McpServerLoaderEnvName { server, name });
            }
            Err(EnvRefusal::CredentialLiteral(name)) => {
                return Err(RuntimeValidationError::McpServerCredentialInEnv { server, name });
            }
        }
    }
    Ok(())
}
