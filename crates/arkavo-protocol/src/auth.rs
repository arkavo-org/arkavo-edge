use crate::error::{A2aError, Result};
use async_trait::async_trait;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::{debug, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionAuth {
    pub sub: String,
    pub scopes: Vec<String>,
    pub exp: Option<i64>,
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    pub sub: String,
    pub exp: Option<i64>,
    pub iat: Option<i64>,
    pub scopes: Option<Vec<String>>,
    #[serde(flatten)]
    pub additional: serde_json::Map<String, serde_json::Value>,
}

#[async_trait]
pub trait AuthBackend: Send + Sync {
    async fn validate_token(&self, token: &str) -> Result<SessionAuth>;

    async fn validate_scopes(&self, auth: &SessionAuth, required_scopes: &[String]) -> bool {
        let user_scopes: HashSet<_> = auth.scopes.iter().collect();
        required_scopes
            .iter()
            .all(|scope| user_scopes.contains(scope))
    }
}

pub struct JwtAuthBackend {
    decoding_key: DecodingKey,
    validation: Validation,
    required_audience: Option<String>,
    required_issuer: Option<String>,
}

impl JwtAuthBackend {
    pub fn new(secret: &str) -> Self {
        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = true;

        Self {
            decoding_key: DecodingKey::from_secret(secret.as_ref()),
            validation,
            required_audience: None,
            required_issuer: None,
        }
    }

    pub fn with_rsa_public_key(public_key_pem: &str) -> Result<Self> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = true;

        Ok(Self {
            decoding_key: DecodingKey::from_rsa_pem(public_key_pem.as_bytes())
                .map_err(|e| A2aError::Auth(format!("Invalid RSA public key: {}", e)))?,
            validation,
            required_audience: None,
            required_issuer: None,
        })
    }

    pub fn with_audience(mut self, audience: String) -> Self {
        self.validation.set_audience(&[&audience]);
        self.required_audience = Some(audience);
        self
    }

    pub fn with_issuer(mut self, issuer: String) -> Self {
        self.validation.set_issuer(&[&issuer]);
        self.required_issuer = Some(issuer);
        self
    }
}

#[async_trait]
impl AuthBackend for JwtAuthBackend {
    async fn validate_token(&self, token: &str) -> Result<SessionAuth> {
        let token_data =
            decode::<JwtClaims>(token, &self.decoding_key, &self.validation).map_err(|e| {
                warn!("JWT validation failed: {}", e);
                A2aError::Auth(format!("Invalid JWT: {}", e))
            })?;

        let claims = token_data.claims;
        debug!("JWT validated for subject: {}", claims.sub);

        Ok(SessionAuth {
            sub: claims.sub,
            scopes: claims.scopes.unwrap_or_default(),
            exp: claims.exp,
            metadata: Some(serde_json::Value::Object(claims.additional)),
        })
    }
}

pub struct NoOpAuthBackend;

#[async_trait]
impl AuthBackend for NoOpAuthBackend {
    async fn validate_token(&self, _token: &str) -> Result<SessionAuth> {
        Ok(SessionAuth {
            sub: "anonymous".to_string(),
            scopes: vec!["public".to_string()],
            exp: None,
            metadata: None,
        })
    }
}

#[derive(Default)]
pub struct MultiAuthBackend {
    backends: Vec<Arc<dyn AuthBackend>>,
}

impl MultiAuthBackend {
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
        }
    }

    pub fn add_backend(mut self, backend: Arc<dyn AuthBackend>) -> Self {
        self.backends.push(backend);
        self
    }
}

#[async_trait]
impl AuthBackend for MultiAuthBackend {
    async fn validate_token(&self, token: &str) -> Result<SessionAuth> {
        for backend in &self.backends {
            if let Ok(auth) = backend.validate_token(token).await {
                return Ok(auth);
            }
        }

        Err(A2aError::Auth(
            "No authentication backend accepted the token".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};

    #[tokio::test]
    async fn test_jwt_auth_backend() {
        let secret = "test_secret";
        let backend = JwtAuthBackend::new(secret);

        let claims = JwtClaims {
            sub: "user123".to_string(),
            exp: Some(chrono::Utc::now().timestamp() + 3600),
            iat: Some(chrono::Utc::now().timestamp()),
            scopes: Some(vec!["read".to_string(), "write".to_string()]),
            additional: serde_json::Map::new(),
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_ref()),
        )
        .unwrap();

        let auth = backend.validate_token(&token).await.unwrap();
        assert_eq!(auth.sub, "user123");
        assert_eq!(auth.scopes, vec!["read", "write"]);
    }

    #[tokio::test]
    async fn test_noop_auth_backend() {
        let backend = NoOpAuthBackend;
        let auth = backend.validate_token("any_token").await.unwrap();
        assert_eq!(auth.sub, "anonymous");
        assert_eq!(auth.scopes, vec!["public"]);
    }

    #[tokio::test]
    async fn test_validate_scopes() {
        let backend = NoOpAuthBackend;
        let auth = SessionAuth {
            sub: "user".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
            exp: None,
            metadata: None,
        };

        assert!(backend.validate_scopes(&auth, &["read".to_string()]).await);
        assert!(
            backend
                .validate_scopes(&auth, &["read".to_string(), "write".to_string()])
                .await
        );
        assert!(!backend.validate_scopes(&auth, &["admin".to_string()]).await);
    }
}
