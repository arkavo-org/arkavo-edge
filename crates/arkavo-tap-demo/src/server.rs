//! Wiring the registry and the merchant from the configuration.

use crate::config::DemoConfig;
use crate::keyfile;
use crate::merchant::{self, Merchant};
use crate::registry::Registry;
use anyhow::Context;
use arkavo_tap::{
    Clock, RegistryConfig, RegistryResolver, SqliteReplayStore, TapVerifier, VerifierConfig,
};
use arkavo_tap_http::{GateConfig, TapGate};
use axum::Router;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

pub type SharedClock = Arc<dyn Clock>;

/// The registry the configuration describes.
pub async fn registry(config: &DemoConfig, clock: SharedClock) -> anyhow::Result<Arc<Registry>> {
    let registry = Registry::open(
        &config.path(&config.trust.registry_database),
        config.authority(),
        clock,
        config.trust.key_max_age_seconds,
    )
    .await?;
    Ok(Arc::new(registry))
}

/// The merchant the configuration describes, and its routes behind the TAP gate.
pub async fn merchant(
    config: &DemoConfig,
    clock: SharedClock,
) -> anyhow::Result<(Arc<Merchant>, Router)> {
    let origin = config.origin()?;
    let receipt_key = keyfile::load(&config.path(&config.keys.merchant))?;
    let issuer = keyfile::load(&config.path(&config.keys.consent_issuer))?;
    let issuers = HashMap::from([(
        config.authorization.issuer_key_id.clone(),
        issuer.verifying_key(),
    )]);
    let resolver = RegistryResolver::new(
        RegistryConfig {
            origin: arkavo_tap::ExternalOrigin::parse(&config.trust.registry_origin)
                .map_err(anyhow::Error::msg)?,
            authority: config.authority(),
            key_path: "/dev/v1/keys/".into(),
            allow_loopback_http: config.trust.allow_loopback_http,
            timeout: Duration::from_secs(5),
            max_document_bytes: 16 * 1024,
            max_concurrent_lookups: 8,
            unknown_key_retry_seconds: 30,
        },
        clock.clone(),
    )
    .map_err(anyhow::Error::msg)?;
    let replay = SqliteReplayStore::open(
        &config.path(&config.replay.path),
        clock.clone(),
        config.replay.cleanup_margin_seconds,
    )
    .await
    .map_err(anyhow::Error::msg)?;
    let verifier = TapVerifier::new(
        VerifierConfig {
            authority: config.authority(),
            audience: origin.clone(),
            max_header_bytes: 16 * 1024,
        },
        resolver,
        replay,
        clock.clone(),
    );
    let gate = TapGate::new(
        verifier,
        GateConfig {
            origin: origin.clone(),
            max_body_bytes: 64 * 1024,
            max_verifications_per_second: 100,
        },
        merchant::protected_routes(config.profile()?),
    );
    let merchant = Arc::new(
        Merchant::open(
            &config.path(&config.merchant.database),
            origin,
            issuers,
            receipt_key,
            clock,
        )
        .await
        .context("opening the merchant database")?,
    );
    let app = merchant::app(merchant.clone(), Arc::new(gate));
    Ok((merchant, app))
}
