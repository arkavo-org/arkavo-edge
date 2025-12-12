//! Configuration for local OpenTDF stack.

use serde::{Deserialize, Serialize};

/// Default network name for OpenTDF containers.
#[allow(unreachable_pub)]
pub const DEFAULT_NETWORK: &str = "opentdf_platform";

/// Container specifications for the OpenTDF stack.
#[derive(Debug, Clone)]
pub struct ContainerSpec {
    /// Container name.
    pub name: String,
    /// Docker image to use.
    pub image: String,
    /// Port mappings (host:container).
    pub ports: Vec<(u16, u16)>,
    /// Environment variables.
    pub env: Vec<(String, String)>,
    /// Volume mounts (host_dir:container_dir).
    pub mounts: Vec<(String, String)>,
    /// Command arguments.
    pub args: Vec<String>,
    /// Dependencies (containers that must be running first).
    pub depends_on: Vec<String>,
}

/// OpenTDF stack configuration.
#[derive(Debug, Clone)]
pub struct StackConfig {
    /// Network name.
    pub network: String,
    /// Container specifications.
    pub containers: Vec<ContainerSpec>,
    /// Configuration directory for OpenTDF.
    pub config_dir: String,
    /// Keys directory for OpenTDF.
    pub keys_dir: String,
}

impl Default for StackConfig {
    fn default() -> Self {
        let home = dirs::home_dir()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "/tmp".to_string());

        let config_dir = format!("{home}/.arkavo/opentdf/config");
        let keys_dir = format!("{home}/.arkavo/opentdf/keys");

        Self {
            network: DEFAULT_NETWORK.to_string(),
            config_dir: config_dir.clone(),
            keys_dir: keys_dir.clone(),
            containers: vec![
                // PostgreSQL for OpenTDF
                ContainerSpec {
                    name: "opentdfdb".to_string(),
                    image: "postgres:15-alpine".to_string(),
                    ports: vec![(5432, 5432)],
                    env: vec![
                        ("POSTGRES_USER".to_string(), "postgres".to_string()),
                        ("POSTGRES_PASSWORD".to_string(), "changeme".to_string()),
                        ("POSTGRES_DB".to_string(), "opentdf".to_string()),
                    ],
                    mounts: vec![],
                    args: vec![],
                    depends_on: vec![],
                },
                // OpenTDF Platform (uses orchestrator for auth)
                ContainerSpec {
                    name: "opentdf".to_string(),
                    image: "registry.opentdf.io/platform:nightly".to_string(),
                    ports: vec![(8080, 8080)],
                    env: vec![],
                    mounts: vec![
                        (config_dir, "/config".to_string()),
                        (keys_dir, "/keys".to_string()),
                    ],
                    args: vec![
                        "start".to_string(),
                        "--config-file".to_string(),
                        "/config/opentdf.yaml".to_string(),
                    ],
                    depends_on: vec!["opentdfdb".to_string()],
                },
            ],
        }
    }
}

/// Endpoints for the local OpenTDF stack.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenTdfEndpoints {
    /// KAS URL (Key Access Service).
    pub kas_url: String,
    /// OAuth URL (orchestrator OIDC provider).
    pub oauth_url: String,
    /// OAuth token endpoint.
    pub token_endpoint: String,
    /// Default OAuth client ID.
    pub client_id: String,
    /// Default OAuth client secret.
    pub client_secret: String,
}

impl Default for OpenTdfEndpoints {
    fn default() -> Self {
        Self {
            kas_url: "http://localhost:8080".to_string(),
            oauth_url: "http://localhost:3000".to_string(),
            token_endpoint: "http://localhost:3000/token".to_string(),
            client_id: "opentdf-sdk".to_string(),
            client_secret: "secret".to_string(),
        }
    }
}
