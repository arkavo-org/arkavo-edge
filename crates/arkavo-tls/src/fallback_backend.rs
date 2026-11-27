#![allow(clippy::redundant_pub_crate)]

use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use tracing::debug;

use crate::{ClientAuth, ProcessTlsReport, TlsError, TlsOptions};

pub(super) fn init_process_tls(options: &TlsOptions) -> Result<ProcessTlsReport, TlsError> {
    #[cfg(feature = "fallback-aws-lc-rs")]
    {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        }
        debug!(
            backend = "rustls-aws-lc-rs",
            requested_profile = %options.profile,
            "Initialized fallback rustls AWS-LC provider (compile with `crypto-profile` for FIPS 4.x policy enforcement)"
        );
        Ok(ProcessTlsReport {
            backend: "rustls-aws-lc-rs",
            profile: "fallback".to_string(),
            module_version: None,
            module_certificate: None,
            excluded_algorithms: Vec::new(),
            module_approval_notice: None,
        })
    }

    #[cfg(not(feature = "fallback-aws-lc-rs"))]
    {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            return Err(TlsError::NoCryptoProvider);
        }
        debug!(
            backend = "rustls-default",
            requested_profile = %options.profile,
            "Using pre-installed rustls default CryptoProvider"
        );
        Ok(ProcessTlsReport {
            backend: "rustls-default",
            profile: "fallback".to_string(),
            module_version: None,
            module_certificate: None,
            excluded_algorithms: Vec::new(),
            module_approval_notice: None,
        })
    }
}

pub(super) fn crypto_provider() -> Result<Arc<rustls::crypto::CryptoProvider>, TlsError> {
    if let Some(installed) = rustls::crypto::CryptoProvider::get_default() {
        return Ok(Arc::clone(installed));
    }

    #[cfg(feature = "fallback-aws-lc-rs")]
    {
        Ok(Arc::new(rustls::crypto::aws_lc_rs::default_provider()))
    }

    #[cfg(not(feature = "fallback-aws-lc-rs"))]
    {
        Err(TlsError::NoCryptoProvider)
    }
}

pub(super) fn client_config(
    roots: RootCertStore,
    client_auth: Option<ClientAuth>,
) -> Result<ClientConfig, TlsError> {
    let provider = crypto_provider()?;
    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Builder(e.to_string()))?
        .with_root_certificates(roots);
    match client_auth {
        Some(auth) => builder
            .with_client_auth_cert(auth.certs, auth.key)
            .map_err(|e| TlsError::Builder(e.to_string())),
        None => Ok(builder.with_no_client_auth()),
    }
}

pub(super) fn server_config_no_client_auth(
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<ServerConfig, TlsError> {
    let provider = crypto_provider()?;
    ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| TlsError::Builder(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| TlsError::Builder(e.to_string()))
}
