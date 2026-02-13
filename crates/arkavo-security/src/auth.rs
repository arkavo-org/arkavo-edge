//! Authentication backend implementations

use crate::error::{Result, SecurityError};
use async_trait::async_trait;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::info;

/// Session authentication data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionAuth {
    /// Subject (user ID)
    pub sub: String,
    /// Scopes granted
    pub scopes: Vec<String>,
    /// Expiration timestamp
    pub exp: Option<i64>,
    /// Additional metadata
    pub metadata: Option<serde_json::Value>,
}

/// JWT claims structure
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JwtClaims {
    /// Subject (user ID)
    pub sub: String,
    /// Expiration timestamp
    pub exp: Option<i64>,
    /// Issued at timestamp
    pub iat: Option<i64>,
    /// Scopes granted
    pub scopes: Option<Vec<String>>,
    /// Additional claims
    #[serde(flatten)]
    pub additional: serde_json::Map<String, serde_json::Value>,
}

/// Authentication backend trait
#[async_trait]
pub trait AuthBackend: Send + Sync {
    /// Validate a token and return session auth data
    async fn validate_token(&self, token: &str) -> Result<SessionAuth>;

    /// Validate that the auth has all required scopes
    async fn validate_scopes(&self, auth: &SessionAuth, required_scopes: &[String]) -> bool {
        let user_scopes: HashSet<_> = auth.scopes.iter().collect();
        required_scopes
            .iter()
            .all(|scope| user_scopes.contains(scope))
    }
}

/// JWT authentication backend
pub struct JwtAuthBackend {
    decoding_key: DecodingKey,
    validation: Validation,
    required_audience: Option<String>,
    required_issuer: Option<String>,
}

impl JwtAuthBackend {
    /// Create a new JWT auth backend with a shared secret
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

    /// Create a new JWT auth backend with an RSA public key
    pub fn with_rsa_public_key(public_key_pem: &str) -> Result<Self> {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_exp = true;

        Ok(Self {
            decoding_key: DecodingKey::from_rsa_pem(public_key_pem.as_bytes())
                .map_err(|e| SecurityError::Auth(format!("Invalid RSA public key: {e}")))?,
            validation,
            required_audience: None,
            required_issuer: None,
        })
    }

    /// Set the required audience
    pub fn with_audience(mut self, audience: String) -> Self {
        self.validation.set_audience(&[&audience]);
        self.required_audience = Some(audience);
        self
    }

    /// Set the required issuer
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
                info!(event = "auth_decision", action = "deny", resource = "jwt", reason = %e, "JWT validation failed");
                SecurityError::Auth(format!("Invalid JWT: {e}"))
            })?;

        let claims = token_data.claims;
        info!(event = "auth_decision", action = "permit", subject = %claims.sub, resource = "jwt", "JWT validated");

        Ok(SessionAuth {
            sub: claims.sub,
            scopes: claims.scopes.unwrap_or_default(),
            exp: claims.exp,
            metadata: Some(serde_json::Value::Object(claims.additional)),
        })
    }
}

/// Multi-backend authentication that tries multiple backends
#[derive(Default)]
pub struct MultiAuthBackend {
    backends: Vec<Arc<dyn AuthBackend>>,
}

impl MultiAuthBackend {
    /// Create a new multi-auth backend
    pub fn new() -> Self {
        Self {
            backends: Vec::new(),
        }
    }

    /// Add a backend to the chain
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

        Err(SecurityError::Auth(
            "No authentication backend accepted the token".to_string(),
        ))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
#[allow(clippy::field_reassign_with_default)]
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
    async fn test_validate_scopes() {
        // Create a mock backend for testing
        struct MockAuthBackend;

        #[async_trait]
        impl AuthBackend for MockAuthBackend {
            async fn validate_token(&self, _token: &str) -> Result<SessionAuth> {
                Ok(SessionAuth {
                    sub: "user".to_string(),
                    scopes: vec!["read".to_string(), "write".to_string()],
                    exp: None,
                    metadata: None,
                })
            }
        }

        let backend = MockAuthBackend;
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
