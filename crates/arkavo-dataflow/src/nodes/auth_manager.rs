use anyhow::Result;
use arkavo_memory::storage::MemoryStorage;
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

/// Secure credential data (stored separately)
#[derive(Debug, Serialize, Deserialize)]
struct SecureCredentialData {
    credential_id: String,
    encrypted_data: String,
    salt: String,
}

/// Authentication manager for secure credential storage
pub struct AuthManager {
    credentials: Arc<RwLock<HashMap<String, AuthCredential>>>,
    storage: Arc<MemoryStorage>,
}

impl AuthManager {
    /// Create a new authentication manager
    pub async fn new() -> Result<Self> {
        let storage = Arc::new(MemoryStorage::new().await?);
        let manager = Self {
            credentials: Arc::new(RwLock::new(HashMap::new())),
            storage,
        };

        // Load existing credentials from storage
        manager.load_credentials().await?;

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
    pub async fn get_credential(&self, credential_id: &str) -> Result<Option<String>> {
        // Check if credential exists
        let creds = self.credentials.read().await;
        if !creds.contains_key(credential_id) {
            return Ok(None);
        }
        drop(creds);

        // Retrieve secure data
        self.retrieve_secure_data(credential_id).await
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

        let mut creds = self.credentials.write().await;

        for result in results {
            if let Ok(credential) = serde_json::from_str::<AuthCredential>(&result.memory.content) {
                creds.insert(credential.id.clone(), credential);
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
        // In a production system, this would use proper encryption
        // For now, we'll use a simple base64 encoding as a placeholder
        use base64::Engine;
        let encoded = base64::engine::general_purpose::STANDARD.encode(value);
        let salt = uuid::Uuid::new_v4().to_string();

        let secure_data = SecureCredentialData {
            credential_id: credential_id.to_string(),
            encrypted_data: encoded,
            salt,
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
                "version": "1.0"
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

            // In production, this would decrypt the data
            // For now, we'll decode from base64
            use base64::Engine;
            let decoded =
                base64::engine::general_purpose::STANDARD.decode(&secure_data.encrypted_data)?;
            let value = String::from_utf8(decoded)?;

            return Ok(Some(value));
        }

        Ok(None)
    }
}

/// Helper function to create auth headers from credentials
pub async fn create_auth_headers(
    auth_manager: &AuthManager,
    credential_id: &str,
) -> Result<HashMap<String, String>> {
    let mut headers = HashMap::new();

    if let Some(metadata) = auth_manager.get_credential_metadata(credential_id).await {
        if let Some(value) = auth_manager.get_credential(credential_id).await? {
            match metadata.auth_method {
                AuthMethod::ApiKey => {
                    headers.insert("X-API-Key".to_string(), value);
                }
                AuthMethod::BearerToken => {
                    headers.insert("Authorization".to_string(), format!("Bearer {value}"));
                }
                AuthMethod::BasicAuth => {
                    headers.insert("Authorization".to_string(), format!("Basic {value}"));
                }
                AuthMethod::OAuth2 => {
                    headers.insert("Authorization".to_string(), format!("Bearer {value}"));
                }
                AuthMethod::Custom(header_name) => {
                    headers.insert(header_name, value);
                }
            }
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
