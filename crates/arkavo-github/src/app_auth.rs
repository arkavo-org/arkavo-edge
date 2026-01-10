use crate::error::{GitHubError, Result};
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
    private_key: Option<EncodingKey>,
    client: Client,
    installation_token: Arc<RwLock<Option<InstallationToken>>>,
    personal_token: Option<String>,
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
            .map_err(|e| GitHubError::GitHubApi(format!("Invalid private key: {e}")))?;

        let client = Client::builder()
            .user_agent("arkavo-edge")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(GitHubError::Http)?;

        Ok(Self {
            app_id,
            private_key: Some(private_key),
            client,
            installation_token: Arc::new(RwLock::new(None)),
            personal_token: None,
        })
    }

    pub fn new_from_token(token: String) -> Result<Self> {
        let client = Client::builder()
            .user_agent("arkavo-edge")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(GitHubError::Http)?;

        Ok(Self {
            app_id: 0,
            private_key: None,
            client,
            installation_token: Arc::new(RwLock::new(None)),
            personal_token: Some(token),
        })
    }

    fn generate_jwt(&self) -> Result<String> {
        let private_key = self
            .private_key
            .as_ref()
            .ok_or_else(|| GitHubError::GitHubApi("GitHub App private key not available".into()))?;

        let now = Utc::now().timestamp();
        let claims = Claims {
            iat: now,
            exp: now + 600, // 10 minutes (GitHub maximum)
            iss: self.app_id.to_string(),
        };

        let header = Header::new(Algorithm::RS256);

        encode(&header, &claims, private_key)
            .map_err(|e| GitHubError::GitHubApi(format!("Failed to generate JWT: {e}")))
    }

    #[allow(dead_code)]
    async fn get_installations(&self) -> Result<Vec<Installation>> {
        let jwt = self.generate_jwt()?;

        debug!("Fetching GitHub App installations");

        let response = self
            .client
            .get(format!("{GITHUB_API_BASE}/app/installations"))
            .header("Authorization", format!("Bearer {jwt}"))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .map_err(GitHubError::Http)?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Failed to get installations: {status} - {error_text}"
            )));
        }

        let installations: Vec<Installation> = response.json().await.map_err(GitHubError::Http)?;
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
            .await
            .map_err(GitHubError::Http)?;

        if !response.status().is_success() {
            let status = response.status();
            let error_text = response.text().await.unwrap_or_default();
            return Err(GitHubError::GitHubApi(format!(
                "Failed to get installation token: {status} - {error_text}"
            )));
        }

        let mut token: InstallationToken = response.json().await.map_err(GitHubError::Http)?;

        let expires_at = chrono::DateTime::parse_from_rfc3339(&token.expires_at)
            .map_err(|e| GitHubError::GitHubApi(format!("Invalid expiry date: {e}")))?
            .timestamp();

        token.cached_until = expires_at - TOKEN_EXPIRY_BUFFER;

        let result = token.token.clone();

        {
            let mut token_guard = self.installation_token.write().await;
            *token_guard = Some(token);
        }

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

    pub async fn get_token(&self, owner: &str, repo: &str) -> Result<String> {
        if let Some(token) = &self.personal_token {
            return Ok(token.clone());
        }

        let installation_id = self
            .find_installation_by_owner(owner)
            .await?
            .ok_or_else(|| {
                GitHubError::GitHubApi(format!(
                    "No GitHub App installation found for owner: {owner}/{repo}"
                ))
            })?;

        self.get_installation_token(installation_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_from_token() {
        let result = GitHubApp::new_from_token("test_token".to_string());
        assert!(result.is_ok());
    }
}
