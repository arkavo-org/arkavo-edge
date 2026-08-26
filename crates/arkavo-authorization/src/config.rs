use std::env;
use std::time::Duration;
use url::Url;

#[derive(Debug, Clone)]
pub struct AuthorizationConfig {
    pub pdp_url: Url,
    pub token_url: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub mcp_resource_id: String,
    pub mcp_server_slug: Option<String>,
    pub oidc_issuer: Option<String>,
    pub audience: Option<String>,
    pub platform_audience: Option<String>,
    pub cose_keys_url: Option<String>,
    pub timeout: Duration,
    pub max_retries: u32,
    pub cache_ttl: Duration,
    pub fail_closed: bool,
    pub safe_diagnostic_tools: Vec<String>,
    /// Test/static override; skips well-known discovery when set.
    pub evaluation_endpoint: Option<Url>,
    /// Test/static service CWT; skips client_credentials when set.
    pub service_token: Option<String>,
}

impl Default for AuthorizationConfig {
    fn default() -> Self {
        let pdp = env::var("AUTHZEN_PDP_URL")
            .or_else(|_| env::var("OPENTDF_BASE_URL"))
            .unwrap_or_else(|_| "https://kas.arkavo.net".to_string());
        let pdp_url =
            Url::parse(&pdp).unwrap_or_else(|_| Url::parse("https://kas.arkavo.net").unwrap());

        Self {
            pdp_url,
            token_url: env::var("AUTHZEN_TOKEN_URL").ok(),
            client_id: env::var("AUTHZEN_CLIENT_ID").ok(),
            client_secret: env::var("AUTHZEN_CLIENT_SECRET").ok(),
            mcp_resource_id: env::var("AUTHZEN_MCP_RESOURCE_ID")
                .unwrap_or_else(|_| "https://mcp.arkavo.net".to_string()),
            mcp_server_slug: env::var("AUTHZEN_MCP_SERVER_SLUG").ok(),
            oidc_issuer: env::var("OIDC_ISSUER").ok(),
            audience: env::var("AUD").ok(),
            platform_audience: env::var("OIDC_PLATFORM_AUDIENCE").ok(),
            cose_keys_url: env::var("AUTHZEN_COSE_KEYS_URL").ok(),
            timeout: Duration::from_secs(5),
            max_retries: 3,
            cache_ttl: Duration::from_mins(1),
            fail_closed: true,
            safe_diagnostic_tools: vec![
                "status".to_string(),
                "health".to_string(),
                "version".to_string(),
            ],
            evaluation_endpoint: None,
            service_token: None,
        }
    }
}

impl AuthorizationConfig {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_pdp_url(mut self, url: &str) -> Result<Self, url::ParseError> {
        self.pdp_url = Url::parse(url)?;
        Ok(self)
    }

    pub fn with_base_url(self, url: &str) -> Result<Self, url::ParseError> {
        self.with_pdp_url(url)
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    pub fn with_cache_ttl(mut self, ttl: Duration) -> Self {
        self.cache_ttl = ttl;
        self
    }

    pub fn with_evaluation_endpoint(mut self, url: Url) -> Self {
        self.evaluation_endpoint = Some(url);
        self
    }

    pub fn with_service_token(mut self, token: impl Into<String>) -> Self {
        self.service_token = Some(token.into());
        self
    }

    pub fn with_safe_tools(mut self, tools: Vec<String>) -> Self {
        self.safe_diagnostic_tools = tools;
        self
    }

    pub fn mcp_server_slug(&self) -> String {
        crate::cwt_subject::mcp_server_slug(&self.mcp_resource_id, self.mcp_server_slug.as_deref())
    }

    pub fn expected_audiences(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(a) = &self.audience {
            out.push(a.clone());
        }
        out.push(self.mcp_resource_id.clone());
        out.push("arkavo".to_string());
        if let Some(p) = &self.platform_audience {
            out.push(p.clone());
        }
        out
    }

    pub fn cose_keys_url(&self) -> Option<String> {
        if let Some(u) = &self.cose_keys_url {
            return Some(u.clone());
        }
        self.oidc_issuer
            .as_ref()
            .map(|iss| format!("{}/.well-known/cose-keys", iss.trim_end_matches('/')))
    }

    pub fn pdp_origin(&self) -> String {
        self.pdp_url.as_str().trim_end_matches('/').to_string()
    }
}
