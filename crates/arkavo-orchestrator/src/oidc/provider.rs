//! OIDC Provider with client registry and JWT signing.

use super::types::{AccessTokenClaims, ClientRegistration, JsonWebKey, JsonWebKeySet};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use jsonwebtoken::{EncodingKey, Header, encode};
use rsa::traits::PublicKeyParts;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

/// OIDC Provider for issuing tokens.
#[derive(Clone)]
pub struct OidcProvider {
    clients: Arc<HashMap<String, ClientRegistration>>,
    encoding_key: Arc<EncodingKey>,
    kid: String,
    /// RSA modulus (n) for JWKS.
    rsa_n: String,
    /// RSA exponent (e) for JWKS.
    rsa_e: String,
    issuer: String,
    token_expiry_secs: u64,
}

impl OidcProvider {
    /// Create a new OIDC provider with generated RSA key.
    ///
    /// # Panics
    /// Panics if RSA key generation fails.
    #[must_use]
    pub fn new(issuer: String) -> Self {
        // Generate RSA key pair for signing
        let rsa = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048)
            .expect("Failed to generate RSA key");

        let pkcs8 = rsa::pkcs8::EncodePrivateKey::to_pkcs8_pem(&rsa, rsa::pkcs8::LineEnding::LF)
            .expect("Failed to encode RSA key");

        let encoding_key =
            EncodingKey::from_rsa_pem(pkcs8.as_bytes()).expect("Failed to create encoding key");

        // Extract public key components for JWKS
        let public_key = rsa.to_public_key();
        let n_bytes = public_key.n().to_bytes_be();
        let e_bytes = public_key.e().to_bytes_be();

        let rsa_n = URL_SAFE_NO_PAD.encode(n_bytes);
        let rsa_e = URL_SAFE_NO_PAD.encode(e_bytes);

        // Generate key ID
        let kid = format!("arkavo-{}", &uuid::Uuid::new_v4().to_string()[..8]);

        // Pre-register default OpenTDF client
        let mut clients = HashMap::new();
        clients.insert(
            "opentdf-sdk".to_string(),
            ClientRegistration {
                client_id: "opentdf-sdk".to_string(),
                client_secret: "secret".to_string(),
                scopes: vec!["openid".to_string()],
            },
        );

        Self {
            clients: Arc::new(clients),
            encoding_key: Arc::new(encoding_key),
            kid,
            rsa_n,
            rsa_e,
            issuer,
            token_expiry_secs: 3600,
        }
    }

    /// Validate client credentials.
    pub fn validate_client(&self, client_id: &str, client_secret: Option<&str>) -> bool {
        if let Some(client) = self.clients.get(client_id) {
            match client_secret {
                Some(secret) => secret == client.client_secret,
                None => false,
            }
        } else {
            false
        }
    }

    /// Issue an access token for a client.
    pub fn issue_token(&self, client_id: &str, scope: Option<&str>) -> Result<String, String> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_secs();

        let claims = AccessTokenClaims {
            sub: client_id.to_string(),
            iss: self.issuer.clone(),
            aud: vec!["opentdf".to_string()],
            iat: now as i64,
            exp: (now + self.token_expiry_secs) as i64,
            scope: scope.map(String::from),
        };

        let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(self.kid.clone());

        encode(&header, &claims, &self.encoding_key).map_err(|e| e.to_string())
    }

    /// Get token expiry in seconds.
    #[must_use]
    pub const fn token_expiry_secs(&self) -> u64 {
        self.token_expiry_secs
    }

    /// Get the issuer URL.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Get JWKS for token verification.
    #[must_use]
    pub fn jwks(&self) -> JsonWebKeySet {
        JsonWebKeySet {
            keys: vec![JsonWebKey {
                kty: "RSA".to_string(),
                use_: "sig".to_string(),
                kid: self.kid.clone(),
                alg: "RS256".to_string(),
                n: self.rsa_n.clone(),
                e: self.rsa_e.clone(),
            }],
        }
    }
}
