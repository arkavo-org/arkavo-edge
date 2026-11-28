#![allow(clippy::redundant_pub_crate)]

use std::sync::Arc;

use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use tracing::debug;

use crate::{ClientAuth, ProcessTlsReport, TlsError, TlsOptions};

pub(super) fn validate_fallback_options(options: &TlsOptions) -> Result<(), TlsError> {
    if options.profile.is_some()
        || options.pq_key_exchange_required
        || options.peer_certificates_match_profile
        || options.require_module_approval
    {
        return Err(TlsError::InvalidConfig(format!(
            "TLS policy enforcement ({options:?}) requires a build compiled with `--features crypto-profile`"
        )));
    }
    Ok(())
}

pub(super) fn init_process_tls(options: &TlsOptions) -> Result<ProcessTlsReport, TlsError> {
    validate_fallback_options(options)?;

    #[cfg(feature = "fallback-aws-lc-rs")]
    {
        if rustls::crypto::CryptoProvider::get_default().is_none() {
            let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
        }
        debug!(
            backend = "rustls-aws-lc-rs",
            requested_profile = options.effective_profile(),
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
            requested_profile = options.effective_profile(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_accepts_default_options() {
        validate_fallback_options(&TlsOptions::default())
            .expect("unset policy env vars must be accepted by fallback backend");
    }

    #[test]
    fn fallback_rejects_explicit_profile_and_strict_flags() {
        let cases = [
            TlsOptions {
                profile: Some("fips1403".to_string()),
                ..TlsOptions::default()
            },
            TlsOptions {
                profile: Some(crate::DEFAULT_PROFILE_NAME.to_string()),
                ..TlsOptions::default()
            },
            TlsOptions {
                profile: Some("bogus".to_string()),
                ..TlsOptions::default()
            },
            TlsOptions {
                pq_key_exchange_required: true,
                ..TlsOptions::default()
            },
            TlsOptions {
                peer_certificates_match_profile: true,
                ..TlsOptions::default()
            },
            TlsOptions {
                require_module_approval: true,
                ..TlsOptions::default()
            },
        ];

        for opts in &cases {
            let err = init_process_tls(opts)
                .expect_err("fallback backend must fail closed on strict TLS policy requests");
            assert!(
                matches!(err, TlsError::InvalidConfig(_)),
                "expected InvalidConfig for {opts:?}, got {err:?}"
            );
            assert!(err.to_string().contains("crypto-profile"));
        }
    }
}
