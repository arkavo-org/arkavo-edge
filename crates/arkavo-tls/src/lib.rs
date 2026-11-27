//! Central application-layer TLS configuration and provider initialization.
//!
//! When compiled with the `crypto-profile` feature, this crate resolves a
//! `crypto-profile-awslc4` policy, verifies the AWS-LC FIPS 4.x module at
//! startup, installs the profile's `rustls::crypto::CryptoProvider`, and
//! constructs `rustls::ClientConfig`, `rustls::ServerConfig`, and
//! `reqwest::Client` instances bound to that policy.
//!
//! Without `crypto-profile`, it falls back to `rustls`'s AWS-LC provider (when
//! `fallback-aws-lc-rs` is active) or an already-installed process default so
//! Windows and C++-free builds continue to compile and run without requiring
//! CMake, Go, or Perl.

#[cfg(feature = "crypto-profile")]
mod profile_backend;

#[cfg(not(feature = "crypto-profile"))]
mod fallback_backend;

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use thiserror::Error;

/// Environment variable selecting the named `crypto-profile` profile.
pub const ENV_TLS_PROFILE: &str = "ARKAVO_TLS_PROFILE";
/// Environment variable requiring post-quantum / hybrid key exchange (TLS 1.3 only).
pub const ENV_TLS_PQ_REQUIRED: &str = "ARKAVO_TLS_PQ_REQUIRED";
/// Environment variable requiring peer certificates to match the resolved profile.
pub const ENV_TLS_PEER_CERTS_MATCH_PROFILE: &str = "ARKAVO_TLS_PEER_CERTS_MATCH_PROFILE";
/// Environment variable failing startup unless every resolved algorithm is module-approved.
pub const ENV_TLS_REQUIRE_MODULE_APPROVAL: &str = "ARKAVO_TLS_REQUIRE_MODULE_APPROVAL";

/// Default named TLS profile (`preferred`).
///
/// Uses TLS 1.3 with post-quantum key exchange preferred and classical
/// P-384/P-256 fallback so endpoints such as `platform.arkavo.net` remain
/// reachable while PQ-capable peers negotiate hybrid ML-KEM.
pub const DEFAULT_PROFILE_NAME: &str = "preferred";

/// Errors returned while resolving or constructing TLS configurations.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum TlsError {
    #[error("invalid TLS configuration: {0}")]
    InvalidConfig(String),
    #[error("FIPS cryptographic module check failed: {0}")]
    FipsModuleUnavailable(String),
    #[error("TLS profile resolution failed: {0}")]
    ProfileResolution(String),
    #[error("module approval check failed: {0}")]
    ModuleApproval(String),
    #[error("failed to install default rustls crypto provider: {0}")]
    ProviderInstall(String),
    #[error("installed rustls crypto provider drifted from active TLS policy: {0}")]
    ProviderDrift(String),
    #[error("failed to build rustls TLS configuration: {0}")]
    Builder(String),
    #[error("failed to build HTTP client: {0}")]
    HttpClient(String),
    #[error("no rustls CryptoProvider is installed or compiled in")]
    NoCryptoProvider,
}

/// Client authentication material for mutual TLS (mTLS).
pub struct ClientAuth {
    pub certs: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
}

/// Application-level TLS configuration parsed from environment variables.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsOptions {
    pub profile: String,
    pub pq_key_exchange_required: bool,
    pub peer_certificates_match_profile: bool,
    pub require_module_approval: bool,
}

impl Default for TlsOptions {
    fn default() -> Self {
        Self {
            profile: DEFAULT_PROFILE_NAME.to_string(),
            pq_key_exchange_required: false,
            peer_certificates_match_profile: false,
            require_module_approval: false,
        }
    }
}

impl TlsOptions {
    /// Reads TLS policy options from process environment variables.
    ///
    /// Invalid boolean values fail closed instead of silently falling back.
    pub fn from_env() -> Result<Self, TlsError> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup<F>(lookup: F) -> Result<Self, TlsError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let profile = lookup(ENV_TLS_PROFILE)
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_PROFILE_NAME.to_string());
        let pq_key_exchange_required =
            parse_bool_env(ENV_TLS_PQ_REQUIRED, lookup(ENV_TLS_PQ_REQUIRED).as_deref())?;
        let peer_certificates_match_profile = parse_bool_env(
            ENV_TLS_PEER_CERTS_MATCH_PROFILE,
            lookup(ENV_TLS_PEER_CERTS_MATCH_PROFILE).as_deref(),
        )?;
        let require_module_approval = parse_bool_env(
            ENV_TLS_REQUIRE_MODULE_APPROVAL,
            lookup(ENV_TLS_REQUIRE_MODULE_APPROVAL).as_deref(),
        )?;

        Ok(Self {
            profile,
            pq_key_exchange_required,
            peer_certificates_match_profile,
            require_module_approval,
        })
    }
}

fn parse_bool_env(name: &str, raw: Option<&str>) -> Result<bool, TlsError> {
    let Some(value) = raw else {
        return Ok(false);
    };
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        other => Err(TlsError::InvalidConfig(format!(
            "{name} has invalid boolean value {other:?} (expected 1/0/true/false)"
        ))),
    }
}

/// Summary of the initialized process-wide TLS configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessTlsReport {
    pub backend: &'static str,
    pub profile: String,
    pub module_version: Option<&'static str>,
    pub module_certificate: Option<String>,
    pub excluded_algorithms: Vec<String>,
    pub module_approval_notice: Option<String>,
}

static PROCESS_TLS: OnceLock<Result<ProcessTlsReport, TlsError>> = OnceLock::new();

/// Initializes process-wide TLS once at startup.
///
/// Under `crypto-profile`, this verifies the AWS-LC FIPS 4.x module, resolves
/// the configured profile, logs module metadata and any excluded algorithms
/// without claiming FIPS validation for Arkavo Edge itself, checks module
/// approval (report-only by default; fail-closed if
/// `ARKAVO_TLS_REQUIRE_MODULE_APPROVAL=1`), and installs the profile's
/// `CryptoProvider` as the process default.
pub fn init_process_tls() -> Result<&'static ProcessTlsReport, TlsError> {
    match PROCESS_TLS.get_or_init(|| {
        let options = TlsOptions::from_env()?;
        init_with_options(&options)
    }) {
        Ok(report) => Ok(report),
        Err(err) => Err(err.clone()),
    }
}

fn init_with_options(options: &TlsOptions) -> Result<ProcessTlsReport, TlsError> {
    #[cfg(feature = "crypto-profile")]
    {
        profile_backend::init_process_tls(options)
    }
    #[cfg(not(feature = "crypto-profile"))]
    {
        fallback_backend::init_process_tls(options)
    }
}

/// Returns the active `rustls::crypto::CryptoProvider` for the process.
pub fn crypto_provider() -> Result<Arc<rustls::crypto::CryptoProvider>, TlsError> {
    let _ = init_process_tls()?;
    #[cfg(feature = "crypto-profile")]
    {
        profile_backend::crypto_provider()
    }
    #[cfg(not(feature = "crypto-profile"))]
    {
        fallback_backend::crypto_provider()
    }
}

/// Returns a `RootCertStore` populated with `webpki-roots`.
pub fn default_root_store() -> RootCertStore {
    let mut roots = RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    roots
}

/// Builds a `rustls::ClientConfig` under the active TLS profile using `roots`
/// and optional mTLS `client_auth`.
pub fn client_config(
    roots: RootCertStore,
    client_auth: Option<ClientAuth>,
) -> Result<ClientConfig, TlsError> {
    let _ = init_process_tls()?;
    #[cfg(feature = "crypto-profile")]
    {
        profile_backend::client_config(roots, client_auth)
    }
    #[cfg(not(feature = "crypto-profile"))]
    {
        fallback_backend::client_config(roots, client_auth)
    }
}

/// Builds a `rustls::ClientConfig` with `webpki-roots` and no client auth.
pub fn default_client_config() -> Result<ClientConfig, TlsError> {
    client_config(default_root_store(), None)
}

/// Builds a `rustls::ServerConfig` with no client authentication under the
/// active TLS profile.
pub fn server_config_no_client_auth(
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<ServerConfig, TlsError> {
    let _ = init_process_tls()?;
    #[cfg(feature = "crypto-profile")]
    {
        profile_backend::server_config_no_client_auth(certs, key)
    }
    #[cfg(not(feature = "crypto-profile"))]
    {
        fallback_backend::server_config_no_client_auth(certs, key)
    }
}

/// Starts a `reqwest::ClientBuilder` preconfigured with the active TLS policy
/// without imposing a total request timeout (for long-lived SSE streams).
pub fn http_client_builder_unbounded() -> Result<reqwest::ClientBuilder, TlsError> {
    let tls_config = default_client_config()?;
    Ok(reqwest::Client::builder().use_preconfigured_tls(tls_config))
}

/// Starts a `reqwest::ClientBuilder` preconfigured with the active TLS policy
/// and a bounded request timeout.
pub fn http_client_builder(timeout: Duration) -> Result<reqwest::ClientBuilder, TlsError> {
    Ok(http_client_builder_unbounded()?.timeout(timeout))
}

/// Builds a `reqwest::Client` using the active TLS policy and `timeout`.
pub fn http_client(timeout: Duration) -> Result<reqwest::Client, TlsError> {
    http_client_builder(timeout)?
        .build()
        .map_err(|e| TlsError::HttpClient(e.to_string()))
}

/// Builds a `reqwest::Client` from a custom `rustls::ClientConfig` (for mTLS or
/// custom trust roots) with a bounded timeout.
pub fn http_client_with_tls(
    tls_config: ClientConfig,
    timeout: Duration,
) -> Result<reqwest::Client, TlsError> {
    reqwest::Client::builder()
        .use_preconfigured_tls(tls_config)
        .timeout(timeout)
        .connect_timeout(timeout)
        .build()
        .map_err(|e| TlsError::HttpClient(e.to_string()))
}

/// Builds a `reqwest::Client` for credentialed KAS and identity endpoints.
///
/// Preconfigured with the active TLS policy, bounded `timeout`, and redirects
/// disabled so bearer tokens and key fetches cannot be redirected to an
/// unvetted host.
pub fn kas_http_client(timeout: Duration) -> Result<reqwest::Client, TlsError> {
    http_client_builder(timeout)?
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| TlsError::HttpClient(e.to_string()))
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // #[tokio::test] expands to Runtime::block_on
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn options_defaults_when_env_unset() {
        let opts = TlsOptions::from_lookup(|_| None).expect("default options");
        assert_eq!(opts.profile, DEFAULT_PROFILE_NAME);
        assert!(!opts.pq_key_exchange_required);
        assert!(!opts.peer_certificates_match_profile);
        assert!(!opts.require_module_approval);
    }

    #[test]
    fn options_parse_explicit_values() {
        let env = HashMap::from([
            (ENV_TLS_PROFILE, "fips1403"),
            (ENV_TLS_PQ_REQUIRED, "1"),
            (ENV_TLS_PEER_CERTS_MATCH_PROFILE, "true"),
            (ENV_TLS_REQUIRE_MODULE_APPROVAL, "yes"),
        ]);
        let opts = TlsOptions::from_lookup(|k| env.get(k).map(|v| (*v).to_string()))
            .expect("explicit options");
        assert_eq!(opts.profile, "fips1403");
        assert!(opts.pq_key_exchange_required);
        assert!(opts.peer_certificates_match_profile);
        assert!(opts.require_module_approval);
    }

    #[test]
    fn options_reject_invalid_boolean() {
        let err =
            TlsOptions::from_lookup(|k| (k == ENV_TLS_PQ_REQUIRED).then(|| "maybe".to_string()))
                .expect_err("invalid boolean must fail closed");
        assert!(matches!(err, TlsError::InvalidConfig(_)));
        assert!(err.to_string().contains(ENV_TLS_PQ_REQUIRED));
    }

    #[cfg(any(feature = "crypto-profile", feature = "fallback-aws-lc-rs"))]
    #[tokio::test]
    async fn kas_http_client_disables_redirects_and_injects_into_opentdf() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local listener");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            let _ = stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
            let _ = stream.shutdown().await;
        });

        let http = kas_http_client(Duration::from_secs(5)).expect("kas_http_client");
        let resp = http
            .get(format!("http://{addr}/kas/v2/kas_public_key"))
            .send()
            .await
            .expect("request should not follow redirect");
        assert_eq!(resp.status().as_u16(), 302);

        // Verify opentdf KasClient accepts the profile-configured reqwest::Client.
        let cfg = opentdf::kas_discovery::OpentdfConfiguration::for_kas_connect(
            "https://platform.arkavo.net",
        );
        let _kas = opentdf::kas::KasClient::with_http_client(&cfg, "test-token", http)
            .expect("KasClient::with_http_client accepts profile HTTP client");
    }
}
