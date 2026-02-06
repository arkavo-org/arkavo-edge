//! KAS (Key Access Service) client implementation for OpenTDF rewrap protocol
//!
//! This module implements the KAS v2 rewrap protocol for unwrapping encrypted payload keys.
//! It supports both Standard TDF (ZIP-based) and NanoTDF formats with RSA and EC key wrapping.
//!
//! # Protocol Flow
//!
//! ## Standard TDF (TDF3, ZTDF) - RSA-2048:
//! 1. Generate ephemeral RSA-2048 key pair
//! 2. Build unsigned rewrap request with manifest data
//! 3. Accept pre-signed JWT token (ES256) - see `examples/jwt_helper.rs`
//! 4. POST to KAS `/v2/rewrap` endpoint
//! 5. Receive RSA-encrypted wrapped key
//! 6. Unwrap key using RSA-OAEP (SHA-1 default, SHA-256 available)
//!
//! ## NanoTDF - EC P-256:
//! 1. Generate ephemeral EC P-256 key pair
//! 2. Build unsigned rewrap request with manifest data
//! 3. Accept pre-signed JWT token (ES256) - see `examples/jwt_helper.rs`
//! 4. POST to KAS `/v2/rewrap` endpoint
//! 5. Receive wrapped key and session public key
//! 6. Unwrap key using ECDH + HKDF + AES-GCM
//!
//! # JWT Token Creation
//!
//! This client requires pre-signed JWT tokens. For a complete JWT helper implementation,
//! see `examples/jwt_helper.rs`.
//!
//! # Example
//!
//! ```no_run
//! use opentdf::kas::KasClient;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let client = KasClient::new(
//!     "http://kas.example.com",
//!     "oauth_token_here",
//! )?;
//!
//! // Create a test manifest
//! let manifest = opentdf::TdfManifest::new(
//!     "0.payload".to_string(),
//!     "http://kas.example.com".to_string()
//! );
//!
//! // Unwrap a key from a TDF manifest (JWT signing handled internally)
//! let payload_key = client.rewrap_standard_tdf(&manifest).await?;
//! # Ok(())
//! # }
//! ```

// Allow deprecated warnings for Nonce::from_slice() which is the correct API for aes-gcm 0.10.x
// This will be resolved when aes-gcm updates to generic-array 1.x
#![allow(deprecated)]

use crate::manifest::TdfManifest;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use opentdf_protocol::{KasError, kas::Policy, *};

#[cfg(feature = "kas-client")]
use {
    crate::hkdf::Hkdf,
    crate::p256::{
        PublicKey, SecretKey,
        pkcs8::{DecodePublicKey, EncodePublicKey},
    },
    crate::sha2::{Digest, Sha256},
    aes_gcm::{
        Aes256Gcm, Nonce,
        aead::{Aead, KeyInit},
    },
    aws_lc_rs::{
        encoding::{AsDer, Pkcs8V1Der, PublicKeyX509Der},
        rsa::{KeySize, OAEP_SHA1_MGF1SHA1, OaepPrivateDecryptingKey, PrivateDecryptingKey},
    },
    reqwest::Client,
};

// NOTE: KAS protocol types are now imported from opentdf-protocol crate
// The types below (KasError, UnsignedRewrapRequest, etc.) are re-exported from there

/// Key type for TDF encryption
#[cfg(feature = "kas-client")]
#[derive(Debug, Clone, Copy)]
pub enum KeyType {
    /// Elliptic Curve (P-256) - used for NanoTDF
    EC,
    /// RSA-2048 - used for Standard TDF (TDF3, ZTDF)
    RSA,
}

/// Ephemeral key pair for KAS communication
pub enum EphemeralKeyPair {
    #[cfg(feature = "kas-client")]
    EC {
        private_key: SecretKey,
        public_key_pem: String,
    },
    #[cfg(feature = "kas-client")]
    RSA {
        /// aws-lc-rs RSA private key for decryption
        private_key: PrivateDecryptingKey,
        public_key_pem: String,
    },
}

#[cfg(feature = "kas-client")]
impl EphemeralKeyPair {
    /// Generate a new ephemeral key pair of the specified type
    ///
    /// Uses aws-lc-rs for RSA key generation (constant-time, FIPS validated).
    pub fn new(key_type: KeyType) -> Result<Self, KasError> {
        use rand::rngs::OsRng;

        match key_type {
            KeyType::EC => {
                // Generate EC P-256 key pair for NanoTDF
                let private_key = SecretKey::random(&mut OsRng);
                let public_key = private_key.public_key();

                let public_key_pem = public_key
                    .to_public_key_pem(crate::p256::pkcs8::LineEnding::LF)
                    .map_err(|e| KasError::Pkcs8Error(e.to_string()))?;

                Ok(EphemeralKeyPair::EC {
                    private_key,
                    public_key_pem,
                })
            }
            KeyType::RSA => {
                // Generate RSA-2048 key pair for Standard TDF using aws-lc-rs
                let private_key =
                    PrivateDecryptingKey::generate(KeySize::Rsa2048).map_err(|e| {
                        KasError::CryptoError {
                            operation: "RSA_key_generation".to_string(),
                            reason: format!("RSA key generation failed: {:?}", e),
                        }
                    })?;

                // Get public key from private key and export to DER
                let public_key = private_key.public_key();
                let public_key_der =
                    AsDer::<PublicKeyX509Der>::as_der(&public_key).map_err(|e| {
                        KasError::Pkcs8Error(format!("Failed to export public key: {:?}", e))
                    })?;

                // Convert DER to PEM (ensure trailing newline for consistency)
                let mut public_key_pem =
                    pem::encode(&pem::Pem::new("PUBLIC KEY", public_key_der.as_ref()));
                if !public_key_pem.ends_with('\n') {
                    public_key_pem.push('\n');
                }

                Ok(EphemeralKeyPair::RSA {
                    private_key,
                    public_key_pem,
                })
            }
        }
    }

    /// Get the public key PEM string
    pub fn public_key_pem(&self) -> &str {
        match self {
            EphemeralKeyPair::EC { public_key_pem, .. } => public_key_pem,
            EphemeralKeyPair::RSA { public_key_pem, .. } => public_key_pem,
        }
    }
}

/// KAS client for rewrap protocol
///
/// This client handles JWT signing internally using an ephemeral RSA key pair.
/// The JWT contains the rewrap request and is signed with RS256.
/// Uses aws-lc-rs for constant-time RSA operations (FIPS validated).
#[cfg(feature = "kas-client")]
pub struct KasClient {
    http_client: Client,
    base_url: String,
    oauth_token: String,
    /// aws-lc-rs RSA private key for JWT signing
    signing_key: PrivateDecryptingKey,
}

#[cfg(feature = "kas-client")]
impl KasClient {
    /// Create a new KAS client
    ///
    /// # Arguments
    ///
    /// * `base_url` - Base URL of the KAS service (e.g., "http://kas.example.com")
    /// * `oauth_token` - OAuth bearer token for authentication
    ///
    /// # Note
    ///
    /// This client generates an ephemeral RSA-2048 key pair for signing JWT rewrap requests.
    /// The JWT signature is verified by KAS when DPoP is enabled, otherwise it's parsed without verification.
    /// Uses aws-lc-rs for constant-time RSA operations (FIPS validated).
    pub fn new(
        base_url: impl Into<String>,
        oauth_token: impl Into<String>,
    ) -> Result<Self, KasError> {
        let http_client = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| KasError::HttpError {
                status: 0,
                message: format!("Failed to build HTTP client: {}", e),
            })?;

        // Generate RSA-2048 key pair for JWT signing (DPoP key) using aws-lc-rs
        let signing_key = PrivateDecryptingKey::generate(KeySize::Rsa2048).map_err(|e| {
            KasError::CryptoError {
                operation: "generate_signing_key".to_string(),
                reason: format!("Failed to generate RSA signing key: {:?}", e),
            }
        })?;

        Ok(Self {
            http_client,
            base_url: base_url.into(),
            oauth_token: oauth_token.into(),
            signing_key,
        })
    }

    /// Internal helper for sending signed rewrap requests to KAS
    ///
    /// Handles HTTP POST, error mapping for 401/403, and JSON response parsing.
    async fn send_rewrap_request(
        &self,
        signed_request: &SignedRewrapRequest,
    ) -> Result<RewrapResponse, KasError> {
        let rewrap_endpoint = format!("{}/kas/v2/rewrap", self.base_url);

        let response = self
            .http_client
            .post(&rewrap_endpoint)
            .header("Authorization", format!("Bearer {}", self.oauth_token))
            .header("Content-Type", "application/json")
            .json(signed_request)
            .send()
            .await
            .map_err(|e| KasError::HttpError {
                status: 0,
                message: format!("HTTP request failed: {}", e),
            })?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            return Err(match status.as_u16() {
                401 => KasError::AuthenticationFailed {
                    reason: "Invalid or missing OAuth token".to_string(),
                },
                403 => KasError::AccessDenied {
                    resource: "KAS endpoint".to_string(),
                    reason: error_body,
                },
                _ => KasError::HttpError {
                    status: status.as_u16(),
                    message: format!("HTTP {}: {}", status, error_body),
                },
            });
        }

        response
            .json()
            .await
            .map_err(|e| KasError::InvalidResponse {
                reason: format!("Failed to parse JSON response: {}", e),
                expected: Some("RewrapResponse".to_string()),
            })
    }

    /// Rewrap a key from a Standard TDF manifest
    ///
    /// This performs the complete KAS rewrap flow:
    /// 1. Generate RSA-2048 ephemeral key pair (Standard TDF uses RSA)
    /// 2. Build unsigned rewrap request
    /// 3. Sign request with internal signing key to create JWT (RS256)
    /// 4. POST to KAS /v2/rewrap endpoint with signed JWT
    /// 5. Unwrap the returned key using RSA-OAEP
    ///
    /// # Arguments
    ///
    /// * `manifest` - TDF manifest containing the wrapped key
    ///
    /// Perform KAS rewrap for NanoTDF
    ///
    /// Sends the NanoTDF header bytes to KAS which extracts the ephemeral public key
    /// and performs ECDH to derive the symmetric encryption key.
    ///
    /// # Arguments
    /// * `header_bytes` - The serialized NanoTDF header
    /// * `kas_url` - The KAS URL from the NanoTDF header
    ///
    /// # Returns
    /// The symmetric key for decrypting the NanoTDF payload
    pub async fn rewrap_nanotdf(
        &self,
        header_bytes: &[u8],
        kas_url: &str,
    ) -> Result<Vec<u8>, KasError> {
        // Build unsigned rewrap request for NanoTDF
        let unsigned_request = self.build_nanotdf_rewrap_request(header_bytes, kas_url)?;

        // Create and sign JWT
        let signed_request_token = self.sign_rewrap_request(&unsigned_request)?;

        // Wrap in SignedRewrapRequest
        let signed_request = SignedRewrapRequest {
            signed_request_token,
        };

        // Send request and parse response
        let rewrap_response = self.send_rewrap_request(&signed_request).await?;

        // For NanoTDF, extract the DEK (symmetric key) directly from response
        // The KAS has already performed ECDH and derived the key
        self.extract_nanotdf_key(&rewrap_response)
    }

    /// Returns the unwrapped payload key ready for TDF decryption
    pub async fn rewrap_standard_tdf(&self, manifest: &TdfManifest) -> Result<Vec<u8>, KasError> {
        // Generate RSA ephemeral key pair for Standard TDF
        let ephemeral_key_pair = EphemeralKeyPair::new(KeyType::RSA)?;

        // Build the unsigned rewrap request
        let unsigned_request = self.build_rewrap_request(manifest, &ephemeral_key_pair)?;

        // Create and sign JWT with the unsigned request
        let signed_request_token = self.sign_rewrap_request(&unsigned_request)?;

        // Wrap the signed token in SignedRewrapRequest per OpenTDF protocol
        let signed_request = SignedRewrapRequest {
            signed_request_token,
        };

        // Send request and parse response
        let rewrap_response = self.send_rewrap_request(&signed_request).await?;

        // Extract the wrapped key and session public key
        let (wrapped_key, session_public_key_pem) = self.extract_wrapped_key(&rewrap_response)?;

        // Unwrap the key using ECDH + HKDF + AES-GCM
        let payload_key =
            self.unwrap_key(&wrapped_key, &session_public_key_pem, &ephemeral_key_pair)?;

        Ok(payload_key)
    }

    /// Sign an unsigned rewrap request to create a JWT
    ///
    /// Creates a JWT with the following structure:
    /// ```json
    /// {
    ///   "requestBody": "<json-serialized-UnsignedRewrapRequest>",
    ///   "iat": <timestamp>,
    ///   "exp": <timestamp+60>
    /// }
    /// ```
    ///
    /// The JWT is signed with RS256 using the client's internal signing key.
    /// Uses aws-lc-rs for constant-time RSA operations (FIPS validated).
    fn sign_rewrap_request(&self, request: &UnsignedRewrapRequest) -> Result<String, KasError> {
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
        use serde::{Deserialize, Serialize};

        #[derive(Debug, Serialize, Deserialize)]
        struct Claims {
            #[serde(rename = "requestBody")]
            request_body: String,
            iat: i64,
            exp: i64,
        }

        // Serialize the unsigned request to JSON
        let request_json = serde_json::to_string(request).map_err(KasError::SerializationError)?;

        // Create JWT claims
        let now = chrono::Utc::now().timestamp();
        let claims = Claims {
            request_body: request_json,
            iat: now,
            exp: now + 60, // 60 second expiration
        };

        // Convert aws-lc-rs RSA private key to PKCS#8 PEM format for jsonwebtoken
        // jsonwebtoken's from_rsa_der expects PKCS#1, but aws-lc-rs exports PKCS#8
        // Use from_rsa_pem instead which accepts PKCS#8 PEM
        let key_der = AsDer::<Pkcs8V1Der>::as_der(&self.signing_key)
            .map_err(|e| KasError::Pkcs8Error(format!("Failed to encode signing key: {:?}", e)))?;

        let key_pem = pem::encode(&pem::Pem::new("PRIVATE KEY", key_der.as_ref()));

        let encoding_key = EncodingKey::from_rsa_pem(key_pem.as_bytes())
            .map_err(|e| KasError::Pkcs8Error(format!("Failed to parse RSA PEM: {}", e)))?;

        // Sign the JWT with RS256
        let token =
            encode(&Header::new(Algorithm::RS256), &claims, &encoding_key).map_err(|e| {
                KasError::JwtError {
                    operation: "sign_jwt".to_string(),
                    reason: format!("Failed to sign JWT: {}", e),
                }
            })?;

        Ok(token)
    }

    /// Extract policy UUID from base64-encoded policy JSON
    ///
    /// The policy is stored as base64-encoded JSON in the manifest.
    /// This function decodes and parses it to extract the UUID field.
    fn extract_policy_uuid(&self, base64_policy: &str) -> Result<String, KasError> {
        // Decode base64 policy
        let policy_bytes = BASE64
            .decode(base64_policy)
            .map_err(KasError::Base64Error)?;

        // Parse JSON
        let policy_json: serde_json::Value =
            serde_json::from_slice(&policy_bytes).map_err(|e| KasError::InvalidResponse {
                reason: format!("Failed to parse policy JSON: {}", e),
                expected: Some("valid JSON policy".to_string()),
            })?;

        // Extract UUID field
        let uuid = policy_json
            .get("uuid")
            .and_then(|v| v.as_str())
            .ok_or_else(|| KasError::InvalidResponse {
                reason: "Policy missing 'uuid' field".to_string(),
                expected: Some("uuid field in policy".to_string()),
            })?;

        // Validate UUID format (should be 36 characters with hyphens)
        if uuid.len() != 36 {
            return Err(KasError::InvalidResponse {
                reason: format!(
                    "Invalid UUID format: expected 36 characters, got {}",
                    uuid.len()
                ),
                expected: Some("36-character UUID".to_string()),
            });
        }

        Ok(uuid.to_string())
    }

    /// Build the unsigned rewrap request from manifest data
    fn build_rewrap_request(
        &self,
        manifest: &TdfManifest,
        ephemeral_key_pair: &EphemeralKeyPair,
    ) -> Result<UnsignedRewrapRequest, KasError> {
        // Convert manifest key access objects to KAS format
        let key_access_wrappers: Vec<KeyAccessObjectWrapper> = manifest
            .encryption_information
            .key_access
            .iter()
            .enumerate()
            .map(|(idx, kao)| {
                // Decode wrapped key to ensure it's valid base64
                let wrapped_key_bytes = BASE64
                    .decode(&kao.wrapped_key)
                    .map_err(KasError::Base64Error)?;

                Ok(KeyAccessObjectWrapper {
                    key_access_object_id: format!("kao-{}", idx),
                    key_access_object: KeyAccessObject {
                        key_type: kao.access_type.clone(),
                        url: kao.url.clone(),
                        protocol: kao.protocol.clone(),
                        wrapped_key: BASE64.encode(&wrapped_key_bytes),
                        policy_binding: KasPolicyBinding {
                            hash: kao.policy_binding.hash.clone(),
                            algorithm: Some(kao.policy_binding.alg.clone()),
                        },
                        encrypted_metadata: kao.encrypted_metadata.clone(),
                        kid: kao.kid.clone(),
                        header: None, // Not used for standard TDF
                        ephemeral_public_key: kao.ephemeral_public_key.clone(),
                    },
                })
            })
            .collect::<Result<Vec<_>, KasError>>()?;

        // Create policy from manifest
        // Extract UUID from the base64-encoded policy JSON
        let policy_uuid = self.extract_policy_uuid(&manifest.encryption_information.policy)?;
        let policy = Policy {
            id: policy_uuid,
            body: manifest.encryption_information.policy.clone(),
        };

        // Create policy request
        let policy_request = PolicyRequest {
            algorithm: None, // For Standard TDF, algorithm is optional and should be None
            policy,
            key_access_objects: key_access_wrappers,
        };

        Ok(UnsignedRewrapRequest {
            client_public_key: ephemeral_key_pair.public_key_pem().to_string(),
            requests: vec![policy_request],
        })
    }

    /// Build unsigned rewrap request for NanoTDF
    fn build_nanotdf_rewrap_request(
        &self,
        header_bytes: &[u8],
        kas_url: &str,
    ) -> Result<UnsignedRewrapRequest, KasError> {
        use base64::Engine;

        // For NanoTDF, we send the header bytes as base64 in the KeyAccessObject
        let header_b64 = BASE64.encode(header_bytes);

        let key_access_object = KeyAccessObjectWrapper {
            key_access_object_id: "kao-0".to_string(),
            key_access_object: KeyAccessObject {
                key_type: "wrapped".to_string(),
                url: kas_url.to_string(),
                protocol: "kas".to_string(),
                wrapped_key: String::new(), // Not used for NanoTDF
                policy_binding: KasPolicyBinding {
                    hash: String::new(), // Extracted from header by KAS
                    algorithm: Some("HS256".to_string()),
                },
                encrypted_metadata: None,
                kid: None,
                header: Some(header_b64),   // NanoTDF header bytes (base64)
                ephemeral_public_key: None, // Not used for NanoTDF rewrap
            },
        };

        // Create policy request
        let policy_request = PolicyRequest {
            algorithm: Some("ec:secp256r1".to_string()), // NanoTDF uses EC algorithm
            policy: Policy {
                id: "policy".to_string(),
                body: String::new(), // Policy is in the header
            },
            key_access_objects: vec![key_access_object],
        };

        Ok(UnsignedRewrapRequest {
            client_public_key: String::new(), // Not used for NanoTDF
            requests: vec![policy_request],
        })
    }

    /// Extract the symmetric key from NanoTDF rewrap response
    fn extract_nanotdf_key(&self, response: &RewrapResponse) -> Result<Vec<u8>, KasError> {
        let policy_result =
            response
                .responses
                .first()
                .ok_or_else(|| KasError::InvalidResponse {
                    reason: "Empty response from KAS".to_string(),
                    expected: Some("at least one policy response".to_string()),
                })?;

        let key_result =
            policy_result
                .results
                .first()
                .ok_or_else(|| KasError::InvalidResponse {
                    reason: "No key access object in response".to_string(),
                    expected: Some("at least one KAO result".to_string()),
                })?;

        // For NanoTDF, the DEK is returned directly (base64 encoded)
        let dek_b64 =
            key_result
                .entity_wrapped_key
                .as_ref()
                .ok_or_else(|| KasError::InvalidResponse {
                    reason: "No DEK in NanoTDF rewrap response".to_string(),
                    expected: Some("entity_wrapped_key field".to_string()),
                })?;

        // Decode the symmetric key
        BASE64.decode(dek_b64).map_err(KasError::Base64Error)
    }

    /// Extract wrapped key from rewrap response
    fn extract_wrapped_key(
        &self,
        response: &RewrapResponse,
    ) -> Result<(Vec<u8>, String), KasError> {
        let policy_result =
            response
                .responses
                .first()
                .ok_or_else(|| KasError::InvalidResponse {
                    reason: "Empty response from KAS".to_string(),
                    expected: Some("at least one policy response".to_string()),
                })?;

        let key_result =
            policy_result
                .results
                .first()
                .ok_or_else(|| KasError::InvalidResponse {
                    reason: "No key results in response".to_string(),
                    expected: Some("at least one key result".to_string()),
                })?;

        // Check status
        if key_result.status != "permit" {
            let error_msg = key_result
                .error
                .clone()
                .unwrap_or_else(|| "Access denied".to_string());
            return Err(KasError::AccessDenied {
                resource: "key".to_string(),
                reason: error_msg,
            });
        }

        // Get wrapped key (try kasWrappedKey first, then entityWrappedKey for legacy)
        let wrapped_key_b64 = key_result
            .kas_wrapped_key
            .as_ref()
            .or(key_result.entity_wrapped_key.as_ref())
            .ok_or_else(|| KasError::InvalidResponse {
                reason: "Missing wrapped key in response".to_string(),
                expected: Some("kasWrappedKey or entityWrappedKey".to_string()),
            })?;

        let wrapped_key = BASE64.decode(wrapped_key_b64)?;

        // Get session public key
        let session_public_key = response
            .session_public_key
            .as_ref()
            .ok_or_else(|| KasError::InvalidResponse {
                reason: "Missing session public key in response".to_string(),
                expected: Some("sessionPublicKey".to_string()),
            })?
            .clone();

        Ok((wrapped_key, session_public_key))
    }

    /// Unwrap the key based on key type (EC or RSA)
    ///
    /// # Protocol
    ///
    /// ## For RSA rewrap (Standard TDF - TDF3, ZTDF):
    /// 1. KAS returns wrapped_key directly encrypted with client's RSA public key
    /// 2. Decrypt using client's RSA private key with OAEP padding (SHA-1)
    /// 3. Return the unwrapped payload key
    ///
    /// ## For EC rewrap (NanoTDF):
    /// 1. ECDH: client_private × session_public → shared_secret
    /// 2. HKDF: salt=SHA256("TDF"), shared_secret → symmetric_key
    /// 3. AES-GCM decrypt: wrapped_key → payload_key
    ///
    /// Uses aws-lc-rs for constant-time RSA operations (FIPS validated).
    fn unwrap_key(
        &self,
        wrapped_key: &[u8],
        session_public_key_pem: &str,
        ephemeral_key_pair: &EphemeralKeyPair,
    ) -> Result<Vec<u8>, KasError> {
        match ephemeral_key_pair {
            EphemeralKeyPair::RSA { private_key, .. } => {
                // RSA-OAEP decryption for Standard TDF using aws-lc-rs
                //
                // SECURITY NOTE: Using SHA-1 with RSA-OAEP for Go SDK compatibility
                //
                // ⚠️  SHA-1 is cryptographically deprecated (collision attacks since 2017)
                // However, it's required to match the OpenTDF platform implementation.
                // The Go SDK uses SHA-1 for RSA-OAEP padding (see platform/lib/ocrypto/asym_decryption.go:104)
                //
                // This is a known limitation for cross-platform interoperability.
                // TODO: File issue to migrate entire OpenTDF ecosystem to SHA-256
                //
                // See: https://shattered.io/ for SHA-1 collision attack details
                //
                // aws-lc-rs provides constant-time RSA operations, eliminating timing attacks

                // Create OAEP decrypting key
                let oaep_key = OaepPrivateDecryptingKey::new(private_key.clone()).map_err(|e| {
                    KasError::CryptoError {
                        operation: "create_oaep_key".to_string(),
                        reason: format!("Failed to create OAEP key: {:?}", e),
                    }
                })?;

                // Allocate plaintext buffer
                let mut plaintext = vec![0u8; oaep_key.min_output_size()];

                // Decrypt using SHA-1 OAEP
                let plaintext_slice = oaep_key
                    .decrypt(&OAEP_SHA1_MGF1SHA1, wrapped_key, &mut plaintext, None)
                    .map_err(|e| KasError::UnwrapError {
                        algorithm: "RSA-OAEP".to_string(),
                        reason: format!("RSA-OAEP decryption failed: {:?}", e),
                    })?;

                Ok(plaintext_slice.to_vec())
            }
            EphemeralKeyPair::EC { private_key, .. } => {
                // EC/ECDH unwrap for NanoTDF
                // Parse session public key from PEM
                let session_public_key = PublicKey::from_public_key_pem(session_public_key_pem)
                    .map_err(|e| KasError::CryptoError {
                        operation: "parse_session_public_key".to_string(),
                        reason: format!("Failed to parse session public key: {}", e),
                    })?;

                // Perform ECDH key agreement
                let shared_secret = crate::p256::ecdh::diffie_hellman(
                    private_key.to_nonzero_scalar(),
                    session_public_key.as_affine(),
                );

                // Derive symmetric key using HKDF with salt = SHA256("TDF")
                let mut salt_hasher = Sha256::new();
                salt_hasher.update(b"TDF");
                let salt = salt_hasher.finalize();

                let hkdf = Hkdf::<Sha256>::new(Some(&salt), shared_secret.raw_secret_bytes());
                let mut symmetric_key = [0u8; 32];
                hkdf.expand(&[], &mut symmetric_key)
                    .map_err(|e| KasError::CryptoError {
                        operation: "HKDF".to_string(),
                        reason: format!("HKDF expansion failed: {}", e),
                    })?;

                // Unwrap key using AES-GCM
                // Format: nonce (12 bytes) || ciphertext || tag (16 bytes)
                if wrapped_key.len() < 28 {
                    return Err(KasError::UnwrapError {
                        algorithm: "ECDH-AES-GCM".to_string(),
                        reason: format!(
                            "Wrapped key too short: {} bytes (expected at least 28)",
                            wrapped_key.len()
                        ),
                    });
                }

                let nonce = Nonce::from_slice(&wrapped_key[..12]);
                let ciphertext_and_tag = &wrapped_key[12..];

                let cipher = Aes256Gcm::new_from_slice(&symmetric_key).map_err(|e| {
                    KasError::CryptoError {
                        operation: "AES-GCM-init".to_string(),
                        reason: format!("Failed to create cipher: {}", e),
                    }
                })?;

                let payload_key = cipher.decrypt(nonce, ciphertext_and_tag).map_err(|e| {
                    KasError::UnwrapError {
                        algorithm: "AES-GCM".to_string(),
                        reason: format!("AES-GCM decryption failed: {}", e),
                    }
                })?;

                Ok(payload_key)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ephemeral_key_pair_generation() {
        #[cfg(feature = "kas-client")]
        {
            // Test RSA key generation
            let key_pair_rsa =
                EphemeralKeyPair::new(KeyType::RSA).expect("Failed to generate RSA key pair");
            assert!(
                key_pair_rsa
                    .public_key_pem()
                    .starts_with("-----BEGIN PUBLIC KEY-----")
            );

            // Test EC key generation
            let key_pair_ec =
                EphemeralKeyPair::new(KeyType::EC).expect("Failed to generate EC key pair");
            assert!(
                key_pair_ec
                    .public_key_pem()
                    .starts_with("-----BEGIN PUBLIC KEY-----")
            );
        }
    }

    #[test]
    fn test_policy_serialization() {
        let policy = Policy {
            id: "test-policy".to_string(),
            body: BASE64.encode(b"test-policy-body"),
        };

        let json = serde_json::to_string(&policy).expect("Failed to serialize");
        assert!(json.contains("test-policy"));
    }
}
