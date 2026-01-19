//! KAS A2A RPC handlers for TDF key operations.

use crate::metrics::{MetricsCollector, RpcTimer};
use crate::rate_limit::RateLimiter;
use arkavo_tdf::{KasA2aHandler, KasPublicKeyRequest, KasPublicKeyResponse, KasRewrapRequest, KasRewrapResponse};
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;

/// Handle kas.rewrap RPC method.
///
/// Verifies delegation token, evaluates ABAC policy, and rewraps the TDF key
/// for the requesting client.
pub async fn handle_kas_rewrap(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    kas_handler: Option<&Arc<KasA2aHandler>>,
    request: KasRewrapRequest,
    caller_did: &str,
) -> Result<KasRewrapResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("kas.rewrap".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let handler = match kas_handler {
        Some(h) => h,
        None => {
            timer.error();
            return Err(ErrorObjectOwned::owned(
                -32603,
                "KAS capability not enabled",
                Some("The KAS capability is not configured on this agent".to_string()),
            ));
        }
    };

    match handler.handle_rewrap(request, caller_did).await {
        Ok(response) => {
            timer.success();
            Ok(response)
        }
        Err(e) => {
            timer.error();
            let (code, message) = match &e {
                arkavo_tdf::KasError::AccessDenied => (-32001, "Access denied"),
                arkavo_tdf::KasError::Delegation(_) => (-32002, "Delegation verification failed"),
                arkavo_tdf::KasError::PolicyBindingInvalid(_) => (-32003, "Invalid policy binding"),
                arkavo_tdf::KasError::KeypairNotConfigured => (-32603, "KAS keypair not configured"),
                _ => (-32000, "KAS error"),
            };
            Err(ErrorObjectOwned::owned(
                code,
                message,
                Some(e.to_string()),
            ))
        }
    }
}

/// Handle kas.publicKey RPC method.
///
/// Returns the KAS public key for TDF encryption.
pub async fn handle_kas_public_key(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    kas_handler: Option<&Arc<KasA2aHandler>>,
    request: KasPublicKeyRequest,
) -> Result<KasPublicKeyResponse, ErrorObjectOwned> {
    let timer = RpcTimer::new("kas.publicKey".to_string(), metrics.clone());

    if let Err(e) = rate_limiter.check_rate_limit() {
        metrics.record_rate_limit_blocked(None);
        timer.error();
        return Err(e);
    }

    let handler = match kas_handler {
        Some(h) => h,
        None => {
            timer.error();
            return Err(ErrorObjectOwned::owned(
                -32603,
                "KAS capability not enabled",
                Some("The KAS capability is not configured on this agent".to_string()),
            ));
        }
    };

    match handler.handle_public_key(request).await {
        Ok(response) => {
            timer.success();
            Ok(response)
        }
        Err(e) => {
            timer.error();
            Err(ErrorObjectOwned::owned(
                -32000,
                "Failed to get KAS public key",
                Some(e.to_string()),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::MetricsCollector;
    use crate::rate_limit::{RateLimitConfig, RateLimiter};
    use arkavo_tdf::PolicyBinding;
    use std::sync::Arc;

    fn create_test_metrics() -> Arc<MetricsCollector> {
        Arc::new(MetricsCollector::new(false))
    }

    fn create_test_rate_limiter() -> RateLimiter {
        RateLimiter::new(RateLimitConfig::default())
    }

    #[tokio::test]
    async fn test_kas_rewrap_no_handler() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        let request = KasRewrapRequest {
            wrapped_key: "dGVzdA==".to_string(),
            policy_binding: PolicyBinding::new("hash"),
            policy: "eyJhdHRyaWJ1dGVzIjpbXX0=".to_string(),
            delegation_token: "{}".to_string(),
            client_public_key: "-----BEGIN PUBLIC KEY-----".to_string(),
        };

        let result = handle_kas_rewrap(
            &metrics,
            &rate_limiter,
            None,
            request,
            "did:key:z6MkTest",
        )
        .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), -32603);
    }

    #[tokio::test]
    async fn test_kas_public_key_no_handler() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        let request = KasPublicKeyRequest::default();

        let result = handle_kas_public_key(&metrics, &rate_limiter, None, request).await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.code(), -32603);
    }
}
