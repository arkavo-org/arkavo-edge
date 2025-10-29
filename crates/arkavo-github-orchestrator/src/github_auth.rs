use crate::error::{Error, Result};
use chrono::Utc;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{debug, info};

const GITHUB_API_BASE: &str = "https://api.github.com";
const TOKEN_EXPIRY_BUFFER: i64 = 300; // 5 minutes

#[derive(Clone)]
pub struct GitHubApp {
    app_id: u64,
    private_key: EncodingKey,
    client: Client,
    installation_token: Arc<RwLock<Option<InstallationToken>>>,
}

#[derive(Debug, Serialize)]
struct Claims {
    iat: i64,
    exp: i64,
    iss: String,
}

#[derive(Debug, Clone, Deserialize)]
struct InstallationToken {
    token: String,
    expires_at: String,
    #[serde(skip)]
    cached_until: i64,
}

#[derive(Debug, Deserialize)]
struct Installation {
    id: u64,
    account: Account,
}

#[derive(Debug, Deserialize)]
struct Account {
    login: String,
    #[allow(dead_code)]
    #[serde(rename = "type")]
    account_type: String,
}

impl GitHubApp {
    pub fn new(app_id: u64, private_key_pem: &str) -> Result<Self> {
        let private_key = EncodingKey::from_rsa_pem(private_key_pem.as_bytes())
            .map_err(|e| Error::Other(anyhow::anyhow!("Invalid private key: {e}")))?;

        let client = Client::builder()
            .user_agent("arkavo-edge")
            .timeout(std::time::Duration::from_secs(30))
            .build()?;

        Ok(Self {
            app_id,
            private_key,
            client,
            installation_token: Arc::new(RwLock::new(None)),
        })
    }

    fn generate_jwt(&self) -> Result<String> {
        let now = Utc::now().timestamp();
        let claims = Claims {
            iat: now,
            exp: now + 600, // 10 minutes (GitHub maximum)
            iss: self.app_id.to_string(),
        };

        let header = Header::new(Algorithm::RS256);

        encode(&header, &claims, &self.private_key)
            .map_err(|e| Error::Other(anyhow::anyhow!("Failed to generate JWT: {e}")))
    }

    pub async fn get_installations(&self) -> Result<Vec<Installation>> {
        let jwt = self.generate_jwt()?;

        debug!("Fetching GitHub App installations");

        let response = self
            .client
            .get(format!("{GITHUB_API_BASE}/app/installations"))
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::GitHubApi(format!(
                "Failed to get installations: {status} - {error_text}"
            )));
        }

        let installations: Vec<Installation> = response.json().await?;
        info!(
            count = installations.len(),
            "Found GitHub App installations"
        );

        Ok(installations)
    }

    pub async fn get_installation_token(&self, installation_id: u64) -> Result<String> {
        let token_guard = self.installation_token.read().await;

        if let Some(token) = &*token_guard {
            let now = Utc::now().timestamp();
            if token.cached_until > now {
                debug!("Using cached installation token");
                return Ok(token.token.clone());
            }
        }

        drop(token_guard);

        debug!(installation_id, "Requesting new installation token");

        let jwt = self.generate_jwt()?;

        let response = self
            .client
            .post(format!(
                "{GITHUB_API_BASE}/app/installations/{installation_id}/access_tokens"
            ))
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(Error::GitHubApi(format!(
                "Failed to get installation token: {status} - {error_text}"
            )));
        }

        let mut token: InstallationToken = response.json().await?;

        let expires_at = chrono::DateTime::parse_from_rfc3339(&token.expires_at)
            .map_err(|e| Error::Other(anyhow::anyhow!("Invalid expiry date: {e}")))?
            .timestamp();

        token.cached_until = expires_at - TOKEN_EXPIRY_BUFFER;

        let result = token.token.clone();

        let mut token_guard = self.installation_token.write().await;
        *token_guard = Some(token);

        info!("Obtained new installation token");

        Ok(result)
    }

    pub async fn find_installation_by_owner(&self, owner: &str) -> Result<Option<u64>> {
        let installations = self.get_installations().await?;

        Ok(installations
            .iter()
            .find(|inst| inst.account.login.eq_ignore_ascii_case(owner))
            .map(|inst| inst.id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_jwt_generation() {
        let private_key_pem = r#"-----BEGIN RSA PRIVATE KEY-----
MIIEowIBAAKCAQEAy8Dbv8prpJ/0kKhlGeJYozo2t60EG8L0561g13R29LvMR5hy
vGZlGJpmn65+A4xHXInJYiPuKzrKUnApogDQ...
-----END RSA PRIVATE KEY-----"#;

        let result = GitHubApp::new(12345, private_key_pem);
        if result.is_ok() {
            let app = result.unwrap();
            let jwt_result = app.generate_jwt();
            assert!(jwt_result.is_ok());
        }
    }
}
