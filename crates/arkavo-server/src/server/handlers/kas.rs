//! KAS A2A RPC handlers for TDF key operations.

use arkavo_protocol::metrics::{MetricsCollector, RpcTimer};
use arkavo_protocol::rate_limit::RateLimiter;
use arkavo_protocol::types::{
    KasPublicKeyRequest, KasPublicKeyResponse, KasRewrapRequest, KasRewrapResponse,
};
use arkavo_tdf::KasA2aHandler;
use base64::{Engine as _, engine::general_purpose};
use jsonrpsee::types::ErrorObjectOwned;
use std::sync::Arc;

/// Convert trusted roots from the AGENTS.md KAS YAML config into
/// delegation verifier roots.
///
/// Roots with an undecodable `public_key` are still trusted by DID (the
/// verifying key is recovered from the DID:key at verification time), but
/// the decode failure is logged since it likely indicates a config mistake.
/// A `public_key` that decodes but does not match the DID:key-embedded key
/// would be silently ignored by the verifier, so the root is dropped as a
/// config error instead of being trusted under false pretenses.
pub fn trusted_roots_from_config(
    config: &arkavo_router::KasYamlConfig,
) -> Vec<arkavo_tdf::TrustedRoot> {
    config
        .trusted_roots
        .iter()
        .filter_map(|root| {
            let public_key_bytes = match &root.public_key {
                Some(encoded) => general_purpose::STANDARD
                    .decode(encoded)
                    .unwrap_or_else(|e| {
                        tracing::warn!(
                            did = %root.did,
                            error = %e,
                            "Invalid base64 public_key for KAS trusted root; trusting by DID only"
                        );
                        Vec::new()
                    }),
                None => Vec::new(),
            };
            if !public_key_bytes.is_empty()
                && let Ok(did_key) = arkavo_crypto::AgentPublicKey::from_did_key(&root.did)
                && did_key.to_bytes() != public_key_bytes
            {
                tracing::warn!(
                    did = %root.did,
                    "KAS trusted root public_key does not match the DID:key-embedded \
                     key; dropping root (config error)"
                );
                return None;
            }
            Some(arkavo_tdf::TrustedRoot {
                did: root.did.clone(),
                public_key_bytes,
            })
        })
        .collect()
}

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

    // Convert local types to arkavo_tdf types
    let tdf_request = arkavo_tdf::KasRewrapRequest {
        wrapped_key: request.wrapped_key,
        policy_binding: arkavo_tdf::PolicyBinding {
            alg: request.policy_binding.alg,
            hash: request.policy_binding.hash,
        },
        policy: request.policy,
        delegation_token: request.delegation_token,
        client_public_key: request.client_public_key,
    };

    match handler.handle_rewrap(tdf_request, caller_did).await {
        Ok(response) => {
            timer.success();
            Ok(KasRewrapResponse {
                entity_wrapped_key: response.entity_wrapped_key,
            })
        }
        Err(e) => {
            timer.error();
            let (code, message) = match &e {
                arkavo_tdf::KasError::AccessDenied => (-32001, "Access denied"),
                arkavo_tdf::KasError::Delegation(_) => (-32002, "Delegation verification failed"),
                arkavo_tdf::KasError::PolicyBindingInvalid(_) => (-32003, "Invalid policy binding"),
                arkavo_tdf::KasError::KeypairNotConfigured => {
                    (-32603, "KAS keypair not configured")
                }
                _ => (-32000, "KAS error"),
            };
            Err(ErrorObjectOwned::owned(code, message, Some(e.to_string())))
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

    // Convert local types to arkavo_tdf types
    let tdf_request = arkavo_tdf::KasPublicKeyRequest {
        algorithm: request.algorithm,
    };

    match handler.handle_public_key(tdf_request).await {
        Ok(response) => {
            timer.success();
            Ok(KasPublicKeyResponse {
                public_key: response.public_key,
                key_id: response.key_id,
                algorithm: response.algorithm,
            })
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
    use arkavo_protocol::rate_limit::RateLimitConfig;
    use arkavo_protocol::types::KasPolicyBinding;

    use std::sync::Arc;

    fn create_test_metrics() -> Arc<MetricsCollector> {
        Arc::new(MetricsCollector::new(false))
    }

    fn create_test_rate_limiter() -> RateLimiter {
        RateLimiter::new(RateLimitConfig::default())
    }

    fn kas_config_with_root(
        did: String,
        public_key: Option<String>,
    ) -> arkavo_router::KasYamlConfig {
        arkavo_router::KasYamlConfig {
            enabled: true,
            key_id: None,
            algorithm: None,
            trusted_roots: vec![arkavo_router::KasTrustedRootYaml {
                did,
                name: None,
                public_key,
            }],
        }
    }

    #[test]
    fn test_trusted_root_matching_public_key_kept() {
        let keypair = arkavo_crypto::AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();
        let encoded = general_purpose::STANDARD.encode(keypair.public_key().to_bytes());
        let roots = trusted_roots_from_config(&kas_config_with_root(did, Some(encoded)));
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].public_key_bytes, keypair.public_key().to_bytes());
    }

    #[test]
    fn test_trusted_root_mismatched_public_key_dropped() {
        let keypair = arkavo_crypto::AgentKeypair::generate();
        let other = arkavo_crypto::AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();
        // public_key belongs to a different key than the DID:key embeds
        let encoded = general_purpose::STANDARD.encode(other.public_key().to_bytes());
        let roots = trusted_roots_from_config(&kas_config_with_root(did, Some(encoded)));
        assert!(
            roots.is_empty(),
            "a public_key contradicting the DID:key must drop the root"
        );
    }

    #[test]
    fn test_trusted_root_invalid_base64_trusted_by_did_only() {
        let keypair = arkavo_crypto::AgentKeypair::generate();
        let did = keypair.public_key().to_did_key();
        let roots =
            trusted_roots_from_config(&kas_config_with_root(did, Some("!!not-base64!!".into())));
        assert_eq!(roots.len(), 1);
        assert!(roots[0].public_key_bytes.is_empty());
    }

    #[tokio::test]
    async fn test_kas_rewrap_no_handler() {
        let metrics = create_test_metrics();
        let rate_limiter = create_test_rate_limiter();

        let request = KasRewrapRequest {
            wrapped_key: "dGVzdA==".to_string(),
            policy_binding: KasPolicyBinding {
                alg: "HS256".to_string(),
                hash: "hash".to_string(),
            },
            policy: "eyJhdHRyaWJ1dGVzIjpbXX0=".to_string(),
            delegation_token: "{}".to_string(),
            client_public_key: "-----BEGIN PUBLIC KEY-----".to_string(),
        };

        let result =
            handle_kas_rewrap(&metrics, &rate_limiter, None, request, "did:key:z6MkTest").await;

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
