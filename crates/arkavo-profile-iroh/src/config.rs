//! Configuration for the creator profile service.

use std::path::PathBuf;

/// Service configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Bind address for the server
    pub bind_address: String,
    /// Port to listen on
    pub port: u16,
    /// Path to TLS certificate (optional, disables TLS if not set)
    pub tls_cert_path: Option<PathBuf>,
    /// Path to TLS private key (optional)
    pub tls_key_path: Option<PathBuf>,
    /// Path to KAS private key for NTDF validation
    pub kas_key_path: Option<PathBuf>,
    /// Expected audience for NTDF tokens
    pub ntdf_expected_audience: String,
    /// Path to Iroh data storage
    pub iroh_data_path: PathBuf,
    /// Path to SQLite index database (optional)
    pub index_db_path: Option<PathBuf>,
}

impl Config {
    /// Load configuration from environment variables.
    pub fn from_env() -> Self {
        Self {
            bind_address: std::env::var("BIND_ADDRESS").unwrap_or_else(|_| "0.0.0.0".to_string()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .unwrap_or(8080),
            tls_cert_path: std::env::var("TLS_CERT_PATH").ok().map(PathBuf::from),
            tls_key_path: std::env::var("TLS_KEY_PATH").ok().map(PathBuf::from),
            kas_key_path: std::env::var("KAS_KEY_PATH").ok().map(PathBuf::from),
            ntdf_expected_audience: std::env::var("NTDF_EXPECTED_AUDIENCE")
                .unwrap_or_else(|_| "https://iroh.arkavo.net".to_string()),
            iroh_data_path: std::env::var("IROH_DATA_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("/var/lib/arkavo/iroh")),
            index_db_path: std::env::var("INDEX_DB_PATH").ok().map(PathBuf::from),
        }
    }

    /// Check if TLS is enabled.
    pub fn tls_enabled(&self) -> bool {
        self.tls_cert_path.is_some() && self.tls_key_path.is_some()
    }

    /// Get the full server URL.
    pub fn server_url(&self) -> String {
        let scheme = if self.tls_enabled() { "https" } else { "http" };
        format!("{}://{}:{}", scheme, self.bind_address, self.port)
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0".to_string(),
            port: 8080,
            tls_cert_path: None,
            tls_key_path: None,
            kas_key_path: None,
            ntdf_expected_audience: "https://iroh.arkavo.net".to_string(),
            iroh_data_path: PathBuf::from("/var/lib/arkavo/iroh"),
            index_db_path: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.bind_address, "0.0.0.0");
        assert_eq!(config.port, 8080);
        assert!(!config.tls_enabled());
    }

    #[test]
    fn test_server_url() {
        let config = Config::default();
        assert_eq!(config.server_url(), "http://0.0.0.0:8080");

        let tls_config = Config {
            tls_cert_path: Some(PathBuf::from("/cert.pem")),
            tls_key_path: Some(PathBuf::from("/key.pem")),
            ..Default::default()
        };
        assert_eq!(tls_config.server_url(), "https://0.0.0.0:8080");
    }
}
