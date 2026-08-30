//! Recover a 32-byte payload key from platform KAS for a protected GGUF.

use arkavo_identity::{IdentityError, IdentitySession, Prompt};
use opentdf::kas::KasClient;
use opentdf::kas_discovery::OpentdfConfiguration;
use opentdf::{KasError, TdfManifest};
use std::future::Future;
use std::path::Path;
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

trait BearerSource: Send + Sync {
    fn bearer(&self, prompt: Prompt) -> impl Future<Output = Result<String, IdentityError>> + Send;
    fn invalidate(&self);
}

trait KeyRecoverer: Send + Sync {
    fn recover(
        &self,
        path: &Path,
        bearer: &str,
    ) -> impl Future<Output = Result<[u8; 32], ProtectedLoadError>> + Send;
}

trait ProtectedRegistry: Send + Sync {
    fn is_loaded(&self, name: &str) -> bool;
    fn load_plain(&self, name: &str, path: &str) -> Result<(), String>;
    fn load_protected(&self, name: &str, path: &str, key: [u8; 32]) -> Result<(), String>;
}

async fn load_protected_path<S, R, G>(
    session: &S,
    recover: &R,
    registry: &G,
    name: &str,
    path: &Path,
) -> Result<(), String>
where
    S: BearerSource,
    R: KeyRecoverer,
    G: ProtectedRegistry,
{
    if registry.is_loaded(name) {
        return Ok(());
    }

    let path_str = path.to_string_lossy();
    if !arkavo_llm::gguf_tdf::is_protected_model_path(path_str.as_ref()) {
        return registry
            .load_plain(name, path_str.as_ref())
            .map_err(|e| format!("Failed to load {name}: {e}"));
    }

    let mut from_cache = true;
    let mut bearer = session
        .bearer(Prompt::Interactive)
        .await
        .map_err(|e| e.kas_denied_message())?;
    loop {
        match recover.recover(path, &bearer).await {
            Ok(key) => return registry.load_protected(name, path_str.as_ref(), key),
            Err(ProtectedLoadError::NotEntitled(_)) => {
                return Err("GGUFTDF_KAS_DENIED: not entitled to this model".into());
            }
            Err(ProtectedLoadError::Unauthenticated(_)) if from_cache => {
                session.invalidate();
                bearer = session
                    .bearer(Prompt::Interactive)
                    .await
                    .map_err(|e| e.kas_denied_message())?;
                from_cache = false;
            }
            Err(ProtectedLoadError::Unauthenticated(_)) => {
                return Err("GGUFTDF_KAS_DENIED: identity token rejected by KAS".into());
            }
            Err(e) => return Err(format!("GGUFTDF_KAS_DENIED: {e}")),
        }
    }
}

/// Opens only the chosen path. Discovery already preferred any sibling `.gguf`.
struct KasKeyRecoverer {
    platform_url: String,
}

impl KasKeyRecoverer {
    fn from_env() -> Self {
        Self {
            platform_url: std::env::var("ARKAVO_PLATFORM_URL")
                .ok()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| arkavo_identity::DEFAULT_PLATFORM_URL.to_string()),
        }
    }
}

impl KeyRecoverer for KasKeyRecoverer {
    async fn recover(&self, path: &Path, bearer: &str) -> Result<[u8; 32], ProtectedLoadError> {
        let archive = arkavo_gguf_tdf::GgufTdfArchive::open(path)
            .map_err(|e| ProtectedLoadError::Other(e.to_string()))?;
        recover_payload_key(archive.manifest(), bearer, &self.platform_url).await
    }
}

impl crate::Router {
    /// Load `path` into the registry. A `.gguf.tdf` is unwrapped through identity + KAS;
    /// plaintext GGUF uses the ordinary loader. Never opens a sibling file.
    pub(crate) async fn ensure_loaded(
        &self,
        registry_name: &str,
        path: &Path,
    ) -> crate::Result<()> {
        load_protected_path(
            self.identity.as_ref(),
            &KasKeyRecoverer::from_env(),
            self.model_registry.as_ref(),
            registry_name,
            path,
        )
        .await
        .map_err(crate::Error::ModelExecution)
    }
}

impl BearerSource for IdentitySession {
    fn bearer(&self, prompt: Prompt) -> impl Future<Output = Result<String, IdentityError>> + Send {
        IdentitySession::bearer(self, prompt)
    }

    fn invalidate(&self) {
        IdentitySession::invalidate(self);
    }
}

impl ProtectedRegistry for arkavo_llm::ModelRegistry {
    fn is_loaded(&self, name: &str) -> bool {
        arkavo_llm::ModelRegistry::is_loaded(self, name)
    }

    fn load_plain(&self, name: &str, path: &str) -> Result<(), String> {
        arkavo_llm::ModelRegistry::load(self, name, path).map_err(|e| e.to_string())
    }

    fn load_protected(&self, name: &str, path: &str, key: [u8; 32]) -> Result<(), String> {
        arkavo_llm::ModelRegistry::load_protected(self, name, path, key).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // #[tokio::test] expands to Runtime::block_on
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

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

    struct FakeSession {
        login_required: bool,
        invalidates: AtomicUsize,
    }

    impl BearerSource for FakeSession {
        async fn bearer(&self, _prompt: Prompt) -> Result<String, IdentityError> {
            if self.login_required {
                Err(IdentityError::LoginRequired("run 'arkavo login'".into()))
            } else {
                Ok("access-token".into())
            }
        }

        fn invalidate(&self) {
            self.invalidates.fetch_add(1, Ordering::SeqCst);
        }
    }

    struct ScriptedRecover {
        calls: AtomicUsize,
        outcomes: Mutex<VecDeque<Result<[u8; 32], ProtectedLoadError>>>,
    }

    impl ScriptedRecover {
        fn new(outcomes: Vec<Result<[u8; 32], ProtectedLoadError>>) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                outcomes: Mutex::new(outcomes.into()),
            }
        }
    }

    impl KeyRecoverer for ScriptedRecover {
        async fn recover(
            &self,
            _path: &Path,
            _bearer: &str,
        ) -> Result<[u8; 32], ProtectedLoadError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.outcomes
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .pop_front()
                .unwrap_or_else(|| Err(ProtectedLoadError::Other("unexpected recover".into())))
        }
    }

    #[derive(Default)]
    struct FakeRegistry {
        load_protected_calls: AtomicUsize,
        load_plain_calls: AtomicUsize,
    }

    impl ProtectedRegistry for FakeRegistry {
        fn is_loaded(&self, _name: &str) -> bool {
            false
        }

        fn load_plain(&self, _name: &str, _path: &str) -> Result<(), String> {
            self.load_plain_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn load_protected(&self, _name: &str, _path: &str, _key: [u8; 32]) -> Result<(), String> {
            self.load_protected_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn login_required_is_denied_without_recover() {
        let session = FakeSession {
            login_required: true,
            invalidates: AtomicUsize::new(0),
        };
        let recover = ScriptedRecover::new(vec![]);
        let registry = FakeRegistry::default();
        let err = load_protected_path(
            &session,
            &recover,
            &registry,
            "gemma",
            Path::new("/models/gemma.gguf.tdf"),
        )
        .await
        .unwrap_err();
        assert!(err.contains("GGUFTDF_KAS_DENIED"), "{err}");
        assert!(err.contains("arkavo login"), "{err}");
        assert_eq!(recover.calls.load(Ordering::SeqCst), 0);
        assert_eq!(registry.load_protected_calls.load(Ordering::SeqCst), 0);
        assert_eq!(registry.load_plain_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unauthenticated_retries_once_then_loads() {
        let session = FakeSession {
            login_required: false,
            invalidates: AtomicUsize::new(0),
        };
        let recover = ScriptedRecover::new(vec![
            Err(ProtectedLoadError::Unauthenticated("401".into())),
            Ok([7u8; 32]),
        ]);
        let registry = FakeRegistry::default();
        load_protected_path(
            &session,
            &recover,
            &registry,
            "gemma",
            Path::new("/models/gemma.gguf.tdf"),
        )
        .await
        .unwrap();
        assert_eq!(recover.calls.load(Ordering::SeqCst), 2);
        assert_eq!(session.invalidates.load(Ordering::SeqCst), 1);
        assert_eq!(registry.load_protected_calls.load(Ordering::SeqCst), 1);
        assert_eq!(registry.load_plain_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn unauthenticated_twice_is_identity_token_rejected() {
        let session = FakeSession {
            login_required: false,
            invalidates: AtomicUsize::new(0),
        };
        let recover = ScriptedRecover::new(vec![
            Err(ProtectedLoadError::Unauthenticated("401".into())),
            Err(ProtectedLoadError::Unauthenticated("401".into())),
        ]);
        let registry = FakeRegistry::default();
        let err = load_protected_path(
            &session,
            &recover,
            &registry,
            "gemma",
            Path::new("/models/gemma.gguf.tdf"),
        )
        .await
        .unwrap_err();
        assert_eq!(err, "GGUFTDF_KAS_DENIED: identity token rejected by KAS");
        assert_eq!(recover.calls.load(Ordering::SeqCst), 2);
        assert_eq!(session.invalidates.load(Ordering::SeqCst), 1);
        assert_eq!(registry.load_protected_calls.load(Ordering::SeqCst), 0);
        assert_eq!(registry.load_plain_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn not_entitled_does_not_retry() {
        let session = FakeSession {
            login_required: false,
            invalidates: AtomicUsize::new(0),
        };
        let recover =
            ScriptedRecover::new(vec![Err(ProtectedLoadError::NotEntitled("403".into()))]);
        let registry = FakeRegistry::default();
        let err = load_protected_path(
            &session,
            &recover,
            &registry,
            "gemma",
            Path::new("/models/gemma.gguf.tdf"),
        )
        .await
        .unwrap_err();
        assert_eq!(err, "GGUFTDF_KAS_DENIED: not entitled to this model");
        assert_eq!(recover.calls.load(Ordering::SeqCst), 1);
        assert_eq!(session.invalidates.load(Ordering::SeqCst), 0);
        assert_eq!(registry.load_protected_calls.load(Ordering::SeqCst), 0);
        assert_eq!(registry.load_plain_calls.load(Ordering::SeqCst), 0);
    }
}
