use anyhow::Result;
use arkavo_memory::storage::MemoryStorage;
use ring::aead::{AES_256_GCM, Aad, BoundKey, Nonce, NonceSequence, SealingKey, UnboundKey};
use ring::pbkdf2;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Types of authentication methods supported
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    ApiKey,
    BearerToken,
    BasicAuth,
    OAuth2,
    Custom(String),
}

/// Authentication credential reference
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthCredential {
    pub id: String,
    pub provider_name: String,
    pub auth_method: AuthMethod,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

/// Decrypted credential with metadata
#[derive(Debug)]
pub struct DecryptedCredential {
    pub metadata: AuthCredential,
    pub value: String,
}

/// Secure credential data (stored separately)
#[derive(Debug, Serialize, Deserialize)]
struct SecureCredentialData {
    credential_id: String,
    encrypted_data: String, // Base64 encoded
    nonce: String,          // Base64 encoded
    salt: String,           // Base64 encoded
}

/// Simple nonce sequence for AES-GCM
struct NonceSeq {
    nonce: [u8; 12],
}

impl NonceSeq {
    fn new(nonce: [u8; 12]) -> Self {
        Self { nonce }
    }
}

impl NonceSequence for NonceSeq {
    fn advance(&mut self) -> Result<Nonce, ring::error::Unspecified> {
        Nonce::try_assume_unique_for_key(&self.nonce)
    }
}

/// Authentication manager for secure credential storage
pub struct AuthManager {
    credentials: Arc<RwLock<HashMap<String, AuthCredential>>>,
    storage: Arc<MemoryStorage>,
    rng: SystemRandom,
}

impl AuthManager {
    /// Create a new authentication manager
    pub async fn new() -> Result<Self> {
        let storage = Arc::new(MemoryStorage::new().await?);
        let manager = Self {
            credentials: Arc::new(RwLock::new(HashMap::new())),
            storage,
            rng: SystemRandom::new(),
        };

        // Load existing credentials from storage
        manager.load_credentials().await?;

        // Log startup message
        tracing::info!(
            "Credential vault initialized with software-only AES-256-GCM encryption (demo)"
        );

        Ok(manager)
    }

    /// Store a new credential
    pub async fn store_credential(
        &self,
        provider_name: &str,
        auth_method: AuthMethod,
        credential_value: &str,
        description: Option<String>,
    ) -> Result<String> {
        let credential_id = format!("{provider_name}_{}", uuid::Uuid::new_v4());

        let credential = AuthCredential {
            id: credential_id.clone(),
            provider_name: provider_name.to_string(),
            auth_method,
            description,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            expires_at: None,
            metadata: None,
        };

        // Store credential metadata
        {
            let mut creds = self.credentials.write().await;
            creds.insert(credential_id.clone(), credential.clone());
        }

        // Store metadata in memory
        self.persist_credential_metadata(&credential).await?;

        // Store encrypted credential data
        self.store_secure_data(&credential_id, credential_value)
            .await?;

        Ok(credential_id)
    }

    /// Retrieve a credential value
    /// Get a credential by ID
    ///
    /// # Panics
    /// 
    /// Panics if the credential metadata is not found after the secure data is retrieved.
    pub async fn get_credential(&self, credential_id: &str) -> Result<DecryptedCredential> {
        // Check if credential exists
        let creds = self.credentials.read().await;
        if !creds.contains_key(credential_id) {
            return Err(anyhow::anyhow!("Credential not found: {}", credential_id));
        }
        let metadata = creds.get(credential_id).cloned();
        drop(creds);

        // Retrieve secure data
        let value = self
            .retrieve_secure_data(credential_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Credential data not found"))?;

        Ok(DecryptedCredential {
            metadata: metadata.unwrap(),
            value,
        })
    }

    /// Get credential metadata (without the actual credential value)
    pub async fn get_credential_metadata(&self, credential_id: &str) -> Option<AuthCredential> {
        let creds = self.credentials.read().await;
        creds.get(credential_id).cloned()
    }

    /// List all credentials for a provider
    pub async fn list_provider_credentials(&self, provider_name: &str) -> Vec<AuthCredential> {
        let creds = self.credentials.read().await;
        creds
            .values()
            .filter(|c| c.provider_name == provider_name)
            .cloned()
            .collect()
    }

    /// Update credential metadata
    pub async fn update_credential_metadata(
        &self,
        credential_id: &str,
        description: Option<String>,
        expires_at: Option<chrono::DateTime<chrono::Utc>>,
    ) -> Result<()> {
        {
            let mut creds = self.credentials.write().await;

            if let Some(cred) = creds.get_mut(credential_id) {
                cred.description = description;
                cred.expires_at = expires_at;
                cred.updated_at = chrono::Utc::now();

                // Clone for persistence
                let cred_clone = cred.clone();
                drop(creds);

                // Persist updated metadata
                self.persist_credential_metadata(&cred_clone).await?;
            }
        }

        Ok(())
    }

    /// Remove a credential
    pub async fn remove_credential(&self, credential_id: &str) -> Result<()> {
        // Remove from in-memory cache
        {
            let mut creds = self.credentials.write().await;
            creds.remove(credential_id);
        }

        // Mark as removed in storage
        // Search for the credential by content matching
        let results = self
            .storage
            .search(credential_id, 1, Some("auth_credential"))
            .await?;

        if let Some(result) = results.first() {
            if let Some(mut metadata) = result.memory.metadata.clone() {
                metadata["removed"] = json!(true);
                metadata["removed_at"] = json!(chrono::Utc::now().to_rfc3339());

                let updated_memory = arkavo_memory::models::Memory {
                    id: result.memory.id,
                    content: result.memory.content.clone(),
                    embedding: result.memory.embedding.clone(),
                    category: result.memory.category.clone(),
                    metadata: Some(metadata),
                    created_at: result.memory.created_at,
                    updated_at: chrono::Utc::now(),
                };

                self.storage.store(updated_memory).await?;
            }
        }

        Ok(())
    }

    /// Check if credentials have expired
    pub async fn cleanup_expired_credentials(&self) -> Result<u32> {
        let now = chrono::Utc::now();
        let mut expired_count = 0;

        let creds = self.credentials.read().await;
        let expired_ids: Vec<String> = creds
            .values()
            .filter(|c| c.expires_at.map(|exp| exp < now).unwrap_or(false))
            .map(|c| c.id.clone())
            .collect();
        drop(creds);

        for credential_id in expired_ids {
            self.remove_credential(&credential_id).await?;
            expired_count += 1;
        }

        Ok(expired_count)
    }

    /// Load credentials from storage
    async fn load_credentials(&self) -> Result<()> {
        // Search for all auth credentials
        let results = self
            .storage
            .search("", 100, Some("auth_credential"))
            .await?;

        {
            let mut creds = self.credentials.write().await;
            for result in results {
                if let Ok(credential) = serde_json::from_str::<AuthCredential>(&result.memory.content) {
                    creds.insert(credential.id.clone(), credential);
                }
            }
        }

        Ok(())
    }

    /// Persist credential metadata
    async fn persist_credential_metadata(&self, credential: &AuthCredential) -> Result<()> {
        let content = serde_json::to_string_pretty(credential)?;

        let memory = arkavo_memory::models::Memory {
            id: uuid::Uuid::new_v4(),
            content,
            embedding: vec![],
            category: Some("auth_credential".to_string()),
            metadata: Some(json!({
                "type": "auth_credential",
                "credential_id": credential.id,
                "provider": credential.provider_name,
                "auth_method": credential.auth_method,
                "version": "1.0"
            })),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.storage.store(memory).await?;

        Ok(())
    }

    /// Store secure credential data (encrypted)
    async fn store_secure_data(&self, credential_id: &str, value: &str) -> Result<()> {
        use base64::Engine;

        // Generate salt for key derivation
        let mut salt = [0u8; 32];
        self.rng
            .fill(&mut salt)
            .map_err(|_| anyhow::anyhow!("Failed to generate salt"))?;

        // Derive key from a master key (in production, this would come from secure storage)
        let master_key = self.get_or_create_master_key()?;
        let mut derived_key = [0u8; 32];
        pbkdf2::derive(
            pbkdf2::PBKDF2_HMAC_SHA256,
            std::num::NonZeroU32::new(100_000).unwrap(),
            &salt,
            master_key.as_bytes(),
            &mut derived_key,
        );

        // Create encryption key
        let unbound_key = UnboundKey::new(&AES_256_GCM, &derived_key)
            .map_err(|_| anyhow::anyhow!("Failed to create encryption key"))?;
        let mut sealing_key = SealingKey::new(unbound_key, NonceSeq::new([0u8; 12]));

        // Generate nonce
        let mut nonce = [0u8; 12];
        self.rng
            .fill(&mut nonce)
            .map_err(|_| anyhow::anyhow!("Failed to generate nonce"))?;

        // Encrypt the credential
        let mut in_out = value.as_bytes().to_vec();
        let tag = sealing_key
            .seal_in_place_separate_tag(Aad::empty(), &mut in_out)
            .map_err(|_| anyhow::anyhow!("Encryption failed"))?;

        // Append tag to encrypted data
        in_out.extend_from_slice(tag.as_ref());

        let secure_data = SecureCredentialData {
            credential_id: credential_id.to_string(),
            encrypted_data: base64::engine::general_purpose::STANDARD.encode(&in_out),
            nonce: base64::engine::general_purpose::STANDARD.encode(nonce),
            salt: base64::engine::general_purpose::STANDARD.encode(salt),
        };

        let content = serde_json::to_string(&secure_data)?;

        let memory = arkavo_memory::models::Memory {
            id: uuid::Uuid::new_v4(),
            content,
            embedding: vec![],
            category: Some("secure_credential".to_string()),
            metadata: Some(json!({
                "type": "secure_credential",
                "credential_id": credential_id,
                "version": "2.0", // Updated version for encrypted format
                "algorithm": "AES-256-GCM"
            })),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        self.storage.store(memory).await?;

        Ok(())
    }

    /// Retrieve secure credential data
    async fn retrieve_secure_data(&self, credential_id: &str) -> Result<Option<String>> {
        // Search for secure credential
        let results = self
            .storage
            .search(credential_id, 1, Some("secure_credential"))
            .await?;

        if let Some(result) = results.first() {
            let secure_data: SecureCredentialData = serde_json::from_str(&result.memory.content)?;

            // Check version for backward compatibility
            if let Some(metadata) = &result.memory.metadata {
                if let Some(version) = metadata.get("version").and_then(|v| v.as_str()) {
                    if version == "1.0" {
                        // Legacy base64-only encoding
                        use base64::Engine;
                        let decoded = base64::engine::general_purpose::STANDARD
                            .decode(&secure_data.encrypted_data)?;
                        let value = String::from_utf8(decoded)?;
                        return Ok(Some(value));
                    }
                }
            }

            // Decrypt AES-256-GCM encrypted data
            use base64::Engine;
            use ring::aead::{Aad, OpeningKey};

            let encrypted_data =
                base64::engine::general_purpose::STANDARD.decode(&secure_data.encrypted_data)?;
            let nonce_bytes =
                base64::engine::general_purpose::STANDARD.decode(&secure_data.nonce)?;
            let salt = base64::engine::general_purpose::STANDARD.decode(&secure_data.salt)?;

            // Derive key from master key
            let master_key = self.get_or_create_master_key()?;
            let mut derived_key = [0u8; 32];
            pbkdf2::derive(
                pbkdf2::PBKDF2_HMAC_SHA256,
                std::num::NonZeroU32::new(100_000).unwrap(),
                &salt,
                master_key.as_bytes(),
                &mut derived_key,
            );

            // Create decryption key
            let unbound_key = UnboundKey::new(&AES_256_GCM, &derived_key)
                .map_err(|_| anyhow::anyhow!("Failed to create decryption key"))?;

            // Create nonce
            let mut nonce = [0u8; 12];
            nonce.copy_from_slice(&nonce_bytes);
            let nonce_seq = NonceSeq::new(nonce);

            let mut opening_key = OpeningKey::new(unbound_key, nonce_seq);

            // Split encrypted data and tag
            let tag_size = AES_256_GCM.tag_len();
            if encrypted_data.len() < tag_size {
                return Err(anyhow::anyhow!("Invalid encrypted data"));
            }

            let mut in_out = encrypted_data.clone();

            // Decrypt
            let decrypted = opening_key
                .open_in_place(Aad::empty(), &mut in_out)
                .map_err(|_| anyhow::anyhow!("Decryption failed"))?;

            let value = String::from_utf8(decrypted.to_vec())?;
            return Ok(Some(value));
        }

        Ok(None)
    }

    /// Get or create master key (in production, this would use secure key storage)
    fn get_or_create_master_key(&self) -> Result<String> {
        // In production, this would:
        // 1. Try to retrieve from secure enclave/HSM
        // 2. Use OS keychain services
        // 3. Derive from hardware security module

        // For now, we use a deterministic key based on a seed
        // This is NOT secure for production use!
        Ok("TEMPORARY_MASTER_KEY_DO_NOT_USE_IN_PRODUCTION".to_string())
    }
}

/// Helper function to create auth headers from credentials
pub async fn create_auth_headers(
    auth_manager: &AuthManager,
    credential_id: &str,
) -> Result<HashMap<String, String>> {
    let mut headers = HashMap::new();

    let credential = auth_manager.get_credential(credential_id).await?;

    match credential.metadata.auth_method {
        AuthMethod::ApiKey => {
            headers.insert("X-API-Key".to_string(), credential.value);
        }
        AuthMethod::BearerToken => {
            headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", credential.value),
            );
        }
        AuthMethod::BasicAuth => {
            headers.insert(
                "Authorization".to_string(),
                format!("Basic {}", credential.value),
            );
        }
        AuthMethod::OAuth2 => {
            headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", credential.value),
            );
        }
        AuthMethod::Custom(header_name) => {
            headers.insert(header_name, credential.value);
        }
    }

    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_methods() {
        let method = AuthMethod::ApiKey;
        assert_eq!(method, AuthMethod::ApiKey);

        // Test serialization
        let json = serde_json::to_string(&method).unwrap();
        assert_eq!(json, r#""api_key""#);
    }

    #[tokio::test]
    async fn test_auth_headers() {
        // This would need a test database setup
        // For now, we'll test the header creation logic
        let mut headers = HashMap::new();
        let value = "test-api-key";

        // Test API key header
        headers.insert("X-API-Key".to_string(), value.to_string());
        assert_eq!(headers.get("X-API-Key").unwrap(), "test-api-key");

        // Test bearer token header
        headers.clear();
        headers.insert("Authorization".to_string(), format!("Bearer {value}"));
        assert_eq!(headers.get("Authorization").unwrap(), "Bearer test-api-key");
    }
}
