#![allow(clippy::redundant_pub_crate)]

use std::str::FromStr;
use std::sync::{Arc, OnceLock};

use crypto_profile_awslc4::{
    self as crypto, CryptoError, Options, Profile, Resolved, TlsError as ProfileTlsError,
    assert_module_approves, fips, resolve, tls,
};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::{ClientConfig, RootCertStore, ServerConfig};
use tracing::{info, warn};

use crate::{ClientAuth, ProcessTlsReport, TlsError, TlsOptions};

static RESOLVED: OnceLock<Resolved> = OnceLock::new();

pub(super) fn resolve_options(options: &TlsOptions) -> Result<Resolved, TlsError> {
    let profile = Profile::from_str(&options.profile).map_err(|e| {
        TlsError::InvalidConfig(format!(
            "{e}; supported profiles: {}",
            Profile::ALL
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    })?;
    let mut profile_options = Options::default();
    profile_options.pq_key_exchange_required = options.pq_key_exchange_required;
    profile_options.peer_certificates_match_profile = options.peer_certificates_match_profile;
    resolve(profile, profile_options).map_err(|e| TlsError::ProfileResolution(e.to_string()))
}

pub(super) fn evaluate_module_approval(
    resolved: &Resolved,
    require_module_approval: bool,
) -> Result<Option<String>, TlsError> {
    match assert_module_approves(resolved) {
        Ok(()) => Ok(None),
        Err(CryptoError::NotFips) => Err(TlsError::FipsModuleUnavailable(
            "AWS-LC module is not running in FIPS mode".to_string(),
        )),
        Err(CryptoError::PolicyEmpty(family)) => Err(TlsError::ProfileResolution(format!(
            "resolved TLS policy has an empty {family:?} family"
        ))),
        Err(err @ CryptoError::NotApproved { .. }) => {
            if require_module_approval {
                Err(TlsError::ModuleApproval(err.to_string()))
            } else {
                Ok(Some(err.to_string()))
            }
        }
        Err(err) => Err(TlsError::ModuleApproval(err.to_string())),
    }
}

pub(super) fn init_process_tls(options: &TlsOptions) -> Result<ProcessTlsReport, TlsError> {
    fips::assert_fips_module().map_err(|e| TlsError::FipsModuleUnavailable(e.to_string()))?;

    let resolved = resolve_options(options)?;
    let module = crypto::MODULE;
    let version = crypto::awslc_version();
    let cert = fips::certificate();
    let excluded: Vec<String> = resolved
        .excluded
        .iter()
        .map(|alg| format!("{alg:?}"))
        .collect();
    let approval_notice = evaluate_module_approval(&resolved, options.require_module_approval)?;

    install_or_verify_default(&resolved.tls)?;
    let _ = RESOLVED.set(resolved.clone());

    let cert_summary = format!(
        "{} ({:?}, number={:?})",
        cert.module_name, cert.status, cert.number
    );

    info!(
        backend = "crypto-profile-awslc4",
        profile = %resolved.profile,
        options = %resolved.options,
        module_line = ?module.line,
        module_linkage = ?module.linkage,
        awslc_version = version,
        module_certificate = %cert_summary,
        excluded_count = excluded.len(),
        excluded = ?excluded,
        "Initialized application TLS provider (module metadata reported; Arkavo Edge itself does not claim FIPS validation)"
    );

    if let Some(notice) = &approval_notice {
        warn!(
            profile = %resolved.profile,
            module_certificate = %cert_summary,
            notice = %notice,
            "Module approval check is non-blocking on AWS-LC 4.x while CMVP review is in process; set ARKAVO_TLS_REQUIRE_MODULE_APPROVAL=1 to fail closed"
        );
    }

    Ok(ProcessTlsReport {
        backend: "crypto-profile-awslc4",
        profile: resolved.profile.as_str().to_string(),
        module_version: Some(version),
        module_certificate: Some(cert_summary),
        excluded_algorithms: excluded,
        module_approval_notice: approval_notice,
    })
}

fn install_or_verify_default(policy: &tls::Policy) -> Result<(), TlsError> {
    match tls::install_default(policy) {
        Ok(()) => Ok(()),
        Err(CryptoError::Tls(ProfileTlsError::DefaultProviderInstalled)) => {
            let installed = rustls::crypto::CryptoProvider::get_default().ok_or_else(|| {
                TlsError::ProviderInstall(
                    "rustls reported an installed provider but get_default() returned None"
                        .to_string(),
                )
            })?;
            tls::assert_provider_matches(installed.as_ref(), policy)
                .map_err(|e| TlsError::ProviderDrift(e.to_string()))
        }
        Err(err) => Err(TlsError::ProviderInstall(err.to_string())),
    }
}

fn active_resolved() -> Result<&'static Resolved, TlsError> {
    RESOLVED
        .get()
        .ok_or_else(|| TlsError::ProviderInstall("TLS has not been initialized".to_string()))
}

pub(super) fn crypto_provider() -> Result<Arc<rustls::crypto::CryptoProvider>, TlsError> {
    let policy = &active_resolved()?.tls;
    tls::crypto_provider(policy)
        .map(Arc::new)
        .map_err(|e| TlsError::Builder(e.to_string()))
}

pub(super) fn build_client_config_for_policy(
    policy: &tls::Policy,
    roots: RootCertStore,
    client_auth: Option<ClientAuth>,
) -> Result<ClientConfig, TlsError> {
    if policy.peer_certificate_rule.is_some() {
        let verifier = tls::server_cert_verifier_builder(Arc::new(roots), policy)
            .map_err(|e| TlsError::Builder(e.to_string()))?
            .build()
            .map_err(|e| TlsError::Builder(e.to_string()))?;
        let builder = tls::strict_client_builder(policy, verifier)
            .map_err(|e| TlsError::Builder(e.to_string()))?;
        finish_client_auth(builder, client_auth)
    } else {
        let builder = tls::client_builder(policy)
            .map_err(|e| TlsError::Builder(e.to_string()))?
            .with_root_certificates(roots);
        finish_client_auth(builder, client_auth)
    }
}

fn finish_client_auth(
    builder: rustls::ConfigBuilder<ClientConfig, rustls::client::WantsClientCert>,
    client_auth: Option<ClientAuth>,
) -> Result<ClientConfig, TlsError> {
    match client_auth {
        Some(auth) => builder
            .with_client_auth_cert(auth.certs, auth.key)
            .map_err(|e| TlsError::Builder(e.to_string())),
        None => Ok(builder.with_no_client_auth()),
    }
}

pub(super) fn client_config(
    roots: RootCertStore,
    client_auth: Option<ClientAuth>,
) -> Result<ClientConfig, TlsError> {
    let policy = &active_resolved()?.tls;
    build_client_config_for_policy(policy, roots, client_auth)
}

pub(super) fn server_config_no_client_auth(
    certs: Vec<CertificateDer<'static>>,
    key: PrivateKeyDer<'static>,
) -> Result<ServerConfig, TlsError> {
    let policy = &active_resolved()?.tls;
    // Server configurations without client authentication have no peer
    // certificate to enforce; strip any peer rule so `server_builder` accepts
    // the policy while retaining versions, suites, groups, and signing schemes.
    let mut server_policy = policy.clone();
    server_policy.peer_certificate_rule = None;
    tls::server_builder(&server_policy)
        .map_err(|e| TlsError::Builder(e.to_string()))?
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|e| TlsError::Builder(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_root_store;

    #[test]
    fn fips_module_is_active_and_default_provider_matches() {
        fips::assert_fips_module().expect("AWS-LC FIPS 4.x module must report FIPS mode");

        let report = crate::init_process_tls().expect("init_process_tls");
        assert_eq!(report.backend, "crypto-profile-awslc4");
        assert!(report.module_version.is_some());
        assert!(report.module_certificate.is_some());

        let policy = &active_resolved().expect("resolved").tls;
        let installed =
            rustls::crypto::CryptoProvider::get_default().expect("default provider installed");
        tls::assert_provider_matches(installed, policy)
            .expect("installed default provider must match resolved policy");
    }

    #[test]
    fn preferred_profile_is_fips_classified_and_includes_classical_fallback() {
        let resolved = resolve_options(&TlsOptions::default()).expect("resolve preferred");
        let cfg = build_client_config_for_policy(&resolved.tls, default_root_store(), None)
            .expect("client config");
        fips::assert_fips_client_config(&cfg)
            .expect("preferred profile client config must classify as FIPS");

        // Preferred keeps classical P-384/P-256 after PQ/hybrid groups so
        // non-PQ endpoints such as platform.arkavo.net remain reachable.
        assert!(resolved.tls.kx_groups.contains(&crypto::KxGroup::Secp384r1));
        assert!(resolved.tls.kx_groups.contains(&crypto::KxGroup::Secp256r1));
    }

    #[test]
    fn pq_required_refuses_classical_only_profiles() {
        for profile in ["cnsa1", "bsi", "fips1402"] {
            let opts = TlsOptions {
                profile: profile.to_string(),
                pq_key_exchange_required: true,
                ..TlsOptions::default()
            };
            let err = resolve_options(&opts)
                .expect_err("PQ-only must fail on profiles without PQ groups");
            assert!(matches!(err, TlsError::ProfileResolution(_)));
        }
    }

    #[test]
    fn unknown_profile_is_refused() {
        let opts = TlsOptions {
            profile: "not-a-profile".to_string(),
            ..TlsOptions::default()
        };
        let err = resolve_options(&opts).expect_err("unknown profile must fail");
        assert!(matches!(err, TlsError::InvalidConfig(_)));
    }

    #[test]
    fn peer_certificates_match_profile_uses_strict_builder() {
        let opts = TlsOptions {
            peer_certificates_match_profile: true,
            ..TlsOptions::default()
        };
        let resolved = resolve_options(&opts).expect("resolve strict preferred");
        assert!(resolved.tls.peer_certificate_rule.is_some());
        let cfg = build_client_config_for_policy(&resolved.tls, default_root_store(), None)
            .expect("strict_client_builder path must succeed");
        fips::assert_fips_client_config(&cfg).expect("strict preferred client config is FIPS");
    }

    #[test]
    fn module_approval_report_only_vs_strict_on_awslc4() {
        let resolved = resolve_options(&TlsOptions::default()).expect("resolve preferred");

        // On AWS-LC 4.x (CMVP in-process, no published security policy),
        // report-only returns Ok(Some(notice)) while strict fails closed.
        let notice = evaluate_module_approval(&resolved, false)
            .expect("report-only mode should record non-approved notice");
        assert!(
            notice.is_some(),
            "expected non-approved notice on AWS-LC 4.x in-process module"
        );

        let err = evaluate_module_approval(&resolved, true)
            .expect_err("strict module approval must fail closed on 4.x");
        assert!(matches!(err, TlsError::ModuleApproval(_)));
    }
}
