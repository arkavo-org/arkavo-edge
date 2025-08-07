use std::env;
use std::time::Duration;
use url::Url;

#[derive(Debug, Clone)]
pub struct AuthorizationConfig {
    pub base_url: Url,
    pub oidc_issuer: Option<String>,
    pub audience: Option<String>,
    pub timeout: Duration,
    pub max_retries: u32,
    pub cache_ttl: Duration,
    pub fail_closed: bool,
    pub safe_diagnostic_tools: Vec<String>,
}

impl Default for AuthorizationConfig {
    fn default() -> Self {
        let base_url = env::var("OPENTDF_BASE_URL")
            .unwrap_or_else(|_| "https://platform.opentdf.io".to_string());

        let base_url = Url::parse(&base_url)
            .unwrap_or_else(|_| Url::parse("https://platform.opentdf.io").unwrap());

        Self {
            base_url,
            oidc_issuer: env::var("OIDC_ISSUER").ok(),
            audience: env::var("AUD").ok(),
            timeout: Duration::from_secs(5),
            max_retries: 3,
            cache_ttl: Duration::from_secs(60),
            fail_closed: true,
            safe_diagnostic_tools: vec![
                "status".to_string(),
                "health".to_string(),
                "version".to_string(),
            ],
        }
    }
}

impl AuthorizationConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_base_url(mut self, url: &str) -> Result<Self, url::ParseError> {
        self.base_url = Url::parse(url)?;
        Ok(self)
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    pub fn with_fail_open(mut self) -> Self {
        self.fail_closed = false;
        self
    }

    pub fn with_safe_tools(mut self, tools: Vec<String>) -> Self {
        self.safe_diagnostic_tools = tools;
        self
    }

    pub fn authorization_v2_url(&self, method: &str) -> Url {
        self.base_url
            .join(&format!("/authorization.v2.AuthorizationService/{method}"))
            .unwrap_or_else(|_| self.base_url.clone())
    }

    pub fn entity_resolution_v2_url(&self, method: &str) -> Url {
        self.base_url
            .join(&format!(
                "/entityresolution.v2.EntityResolutionService/{method}"
            ))
            .unwrap_or_else(|_| self.base_url.clone())
    }
}
