//! A key registry over HTTPS: its document format, and a resolver that asks only the one
//! registry it is configured with.
//!
//! The resolver follows no redirects, uses no proxy, caps response size and time, bounds
//! concurrent lookups, and remembers unknown keys for a while so a stream of made-up key IDs
//! cannot turn into a stream of lookups. A cached record is used only until its refresh
//! deadline; after that a failed lookup fails closed rather than reviving the stale record,
//! so a revocation takes effect within one refresh interval.

use crate::clock::Clock;
use crate::context::ExternalOrigin;
use crate::keys::{
    ActorBinding, Environment, KeyFields, KeyRecord, KeyStatus, ResolveError, TrustAuthority,
    TrustedKeyResolver, key_id_is_valid,
};
use crate::profile::Purpose;
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::{Mutex, Semaphore};

/// A key as a registry publishes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct KeyDocument {
    pub authority: String,
    pub environment: String,
    pub key_id: String,
    pub kty: String,
    pub crv: String,
    /// The public key, base64url without padding.
    pub x: String,
    /// `active` or `revoked`.
    pub status: String,
    pub not_before: i64,
    pub not_after: i64,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime: Option<String>,
    /// TAP tags the key may declare.
    pub purposes: Vec<String>,
    /// How long a verifier may use this document before asking again, 1 to 3600 seconds.
    pub max_age_seconds: i64,
}

impl KeyDocument {
    /// The record this document describes, if it is well formed and speaks for `authority`.
    pub fn to_record(
        &self,
        authority: &TrustAuthority,
        key_id: &str,
        now: i64,
        provenance: &str,
    ) -> Result<KeyRecord, String> {
        if self.authority != authority.name
            || Environment::parse(&self.environment)? != authority.environment
        {
            return Err("the document speaks for another authority or environment".into());
        }
        if self.key_id != key_id {
            return Err("the document describes another key".into());
        }
        if self.kty != "OKP" || self.crv != "Ed25519" {
            return Err(format!("unsupported key type {}/{}", self.kty, self.crv));
        }
        if !(1..=3600).contains(&self.max_age_seconds) {
            return Err("maxAgeSeconds must be 1 to 3600".into());
        }
        let status = match self.status.as_str() {
            "active" => KeyStatus::Active,
            "revoked" => KeyStatus::Revoked,
            other => return Err(format!("unknown key status {other:?}")),
        };
        let purposes = self
            .purposes
            .iter()
            .map(|t| Purpose::from_tag(t).ok_or_else(|| format!("unknown purpose {t:?}")))
            .collect::<Result<Vec<_>, _>>()?;
        let x = URL_SAFE_NO_PAD
            .decode(&self.x)
            .map_err(|e| format!("x is not base64url: {e}"))?;
        KeyRecord::ed25519(KeyFields {
            authority: authority.clone(),
            key_id: key_id.to_owned(),
            binding: ActorBinding {
                provider: self.provider.clone(),
                runtime: self.runtime.clone(),
            },
            public_key: &x,
            status,
            not_before: self.not_before,
            not_after: self.not_after,
            purposes,
            retrieved_at: now,
            refresh_by: now + self.max_age_seconds,
            provenance: provenance.to_owned(),
        })
    }
}

/// Where a registry is and how it may be reached.
#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub origin: ExternalOrigin,
    pub authority: TrustAuthority,
    /// Path prefix keys are served under, e.g. `/dev/v1/keys/`.
    pub key_path: String,
    /// Plain HTTP is allowed only for a loopback origin, and only when this is set.
    pub allow_loopback_http: bool,
    pub timeout: Duration,
    pub max_document_bytes: usize,
    pub max_concurrent_lookups: usize,
    /// How long an unknown key is remembered before it is looked up again.
    pub unknown_key_retry_seconds: i64,
}

enum Cached {
    Known(Box<KeyRecord>),
    Unknown { until: i64 },
}

/// Resolves keys from one configured registry.
pub struct RegistryResolver<C> {
    config: RegistryConfig,
    client: reqwest::Client,
    clock: C,
    cache: Mutex<HashMap<(String, Environment, String), Cached>>,
    lookups: Semaphore,
}

impl<C: Clock> RegistryResolver<C> {
    pub fn new(config: RegistryConfig, clock: C) -> Result<Self, String> {
        match config.origin.scheme() {
            "https" => {}
            "http" if config.allow_loopback_http && config.origin.is_loopback() => {}
            _ => {
                return Err(format!(
                    "registry {} must use TLS (plain HTTP is allowed only on loopback in development)",
                    config.origin.as_origin()
                ));
            }
        }
        if !config.key_path.starts_with('/') || !config.key_path.ends_with('/') {
            return Err("key_path must start and end with /".into());
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .timeout(config.timeout)
            .build()
            .map_err(|e| format!("building the registry client: {e}"))?;
        let lookups = Semaphore::new(config.max_concurrent_lookups.max(1));
        Ok(RegistryResolver {
            config,
            client,
            clock,
            cache: Mutex::new(HashMap::new()),
            lookups,
        })
    }

    async fn fetch(&self, key_id: &str) -> Result<Option<KeyDocument>, String> {
        let _permit = self
            .lookups
            .acquire()
            .await
            .map_err(|_| "lookups closed".to_owned())?;
        let url = format!(
            "{}{}{key_id}",
            self.config.origin.as_origin(),
            self.config.key_path
        );
        let mut response = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| format!("registry request failed: {e}"))?;
        match response.status().as_u16() {
            200 => {}
            404 => return Ok(None),
            other => return Err(format!("registry answered {other}")),
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| format!("reading the registry answer: {e}"))?
        {
            if body.len() + chunk.len() > self.config.max_document_bytes {
                return Err("registry document is too large".into());
            }
            body.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|e| format!("registry document does not parse: {e}"))
    }
}

impl<C: Clock> TrustedKeyResolver for RegistryResolver<C> {
    async fn resolve(
        &self,
        authority: &TrustAuthority,
        key_id: &str,
    ) -> Result<KeyRecord, ResolveError> {
        if *authority != self.config.authority || !key_id_is_valid(key_id) {
            return Err(ResolveError::Unknown);
        }
        let cache_key = (
            authority.name.clone(),
            authority.environment,
            key_id.to_owned(),
        );
        let now = self.clock.now();
        match self.cache.lock().await.get(&cache_key) {
            Some(Cached::Known(record)) if now < record.refresh_by => return Ok((**record).clone()),
            Some(Cached::Unknown { until }) if now < *until => return Err(ResolveError::Unknown),
            _ => {}
        }
        let provenance = format!(
            "{}{}{key_id}",
            self.config.origin.as_origin(),
            self.config.key_path
        );
        let result = match self.fetch(key_id).await {
            Ok(Some(doc)) => doc
                .to_record(authority, key_id, now, &provenance)
                .map_err(ResolveError::Unavailable),
            Ok(None) => Err(ResolveError::Unknown),
            Err(e) => Err(ResolveError::Unavailable(e)),
        };
        let mut cache = self.cache.lock().await;
        match &result {
            Ok(record) => {
                cache.insert(cache_key, Cached::Known(Box::new(record.clone())));
            }
            Err(ResolveError::Unknown) => {
                let until = now + self.config.unknown_key_retry_seconds;
                cache.insert(cache_key, Cached::Unknown { until });
            }
            Err(ResolveError::Unavailable(_)) => {
                cache.remove(&cache_key);
            }
        }
        result
    }
}
