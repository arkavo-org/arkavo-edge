//! Recover a 32-byte payload key from platform KAS for a protected GGUF.

use opentdf::kas::KasClient;
use opentdf::kas_discovery::OpentdfConfiguration;
use opentdf::{KasError, TdfManifest};
use thiserror::Error;

/// Fail-closed outcomes of a KAS rewrap. 401 is retryable after re-login;
/// 403 is an entitlement miss and must not be retried.
#[derive(Debug, Error)]
pub enum ProtectedLoadError {
    #[error("{0}")]
    Unauthenticated(String),
    #[error("{0}")]
    NotEntitled(String),
    #[error("{0}")]
    Other(String),
}

/// Ask platform KAS to rewrap the archive's wrapped key into a 32-byte AES key.
pub async fn recover_payload_key(
    manifest: &TdfManifest,
    bearer: &str,
    platform_url: &str,
) -> Result<[u8; 32], ProtectedLoadError> {
    let cfg = OpentdfConfiguration::for_kas_connect(platform_url);
    let kas = KasClient::new(&cfg, bearer).map_err(|e| classify_kas_error(&e))?;
    let key = kas
        .rewrap_standard_tdf(manifest)
        .await
        .map_err(|e| classify_kas_error(&e))?;
    key_to_array(key)
}

fn classify_kas_error(err: &KasError) -> ProtectedLoadError {
    match err {
        KasError::HttpError { status: 401, .. } | KasError::AuthenticationFailed { .. } => {
            ProtectedLoadError::Unauthenticated(err.to_string())
        }
        KasError::HttpError { status: 403, .. } | KasError::AccessDenied { .. } => {
            ProtectedLoadError::NotEntitled(err.to_string())
        }
        _ => ProtectedLoadError::Other(err.to_string()),
    }
}

fn key_to_array(key: Vec<u8>) -> Result<[u8; 32], ProtectedLoadError> {
    let len = key.len();
    key.try_into().map_err(|_| {
        ProtectedLoadError::Other(format!("KAS returned a key of {len} bytes, expected 32"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_401_is_unauthenticated() {
        let err = KasError::HttpError {
            status: 401,
            message: "unauthorized".into(),
        };
        assert!(matches!(
            classify_kas_error(&err),
            ProtectedLoadError::Unauthenticated(_)
        ));
    }

    #[test]
    fn http_403_is_not_entitled() {
        let err = KasError::HttpError {
            status: 403,
            message: "forbidden".into(),
        };
        assert!(matches!(
            classify_kas_error(&err),
            ProtectedLoadError::NotEntitled(_)
        ));
    }

    #[test]
    fn thirty_one_byte_key_is_other() {
        let err = key_to_array(vec![0u8; 31]).unwrap_err();
        assert!(matches!(err, ProtectedLoadError::Other(_)));
        assert_eq!(
            err.to_string(),
            "KAS returned a key of 31 bytes, expected 32"
        );
    }

    #[test]
    fn authentication_failed_is_unauthenticated() {
        let err = KasError::AuthenticationFailed {
            reason: "unauthenticated".into(),
        };
        assert!(matches!(
            classify_kas_error(&err),
            ProtectedLoadError::Unauthenticated(_)
        ));
    }

    #[test]
    fn access_denied_is_not_entitled() {
        let err = KasError::AccessDenied {
            resource: "KAS endpoint".into(),
            reason: "forbidden".into(),
        };
        assert!(matches!(
            classify_kas_error(&err),
            ProtectedLoadError::NotEntitled(_)
        ));
    }

    #[test]
    fn thirty_two_byte_key_is_ok() {
        assert_eq!(key_to_array(vec![7u8; 32]).unwrap(), [7u8; 32]);
    }

    #[test]
    fn other_http_status_is_other() {
        let err = KasError::HttpError {
            status: 500,
            message: "internal".into(),
        };
        assert!(matches!(
            classify_kas_error(&err),
            ProtectedLoadError::Other(_)
        ));
    }
}
