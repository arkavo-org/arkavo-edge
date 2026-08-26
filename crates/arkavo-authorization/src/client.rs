use crate::cache::DecisionCache;
use crate::config::AuthorizationConfig;
use crate::cwt_subject::{sarc_tools_call, sarc_tools_list, token_map, trust_anchor_ok};
use crate::cwt_verify::CwtVerifier;
use crate::error::{AuthorizationError, Result};
use crate::pep::{is_hardcoded_mapped, is_pass_through, subject_cwt_from, tool_name_from_params};
use crate::types::{AuthzenEvaluationResponse, Decision, McpToolMapping};
use reqwest::{Client, StatusCode};
use serde_json::Value;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
use tracing::{error, info, warn};
use url::Url;

const TOKEN_REFRESH_MARGIN: Duration = Duration::from_secs(60);

struct CachedToken {
    token: String,
    expires_at: Instant,
}

#[derive(serde::Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
}

#[derive(serde::Deserialize)]
struct AuthzenDiscovery {
    policy_decision_point: Option<String>,
    access_evaluation_endpoint: Option<String>,
}

pub struct AuthorizationClient {
    config: AuthorizationConfig,
    http_client: Client,
    cache: Arc<DecisionCache>,
    verifier: CwtVerifier,
    token_cache: Mutex<Option<CachedToken>>,
    evaluation_url: Mutex<Option<Url>>,
}

impl AuthorizationClient {
    pub fn new(config: AuthorizationConfig) -> Result<Self> {
        let http_client = Client::builder()
            .timeout(config.timeout)
            .redirect(reqwest::redirect::Policy::none())
            .danger_accept_invalid_certs(false)
            .build()
            .map_err(|e| AuthorizationError::ConfigError(e.to_string()))?;

        let mut verifier = if let Some(url) = config.cose_keys_url() {
            CwtVerifier::new(url, config.oidc_issuer.clone())
        } else {
            CwtVerifier::with_static_keys(vec![])
        };
        if let Some(iss) = &config.oidc_issuer {
            verifier = verifier.with_expected_issuer(iss.clone());
        }
        verifier = verifier.with_audiences(config.expected_audiences());

        Ok(Self {
            cache: Arc::new(DecisionCache::new(1000, config.cache_ttl)),
            config,
            http_client,
            verifier,
            token_cache: Mutex::new(None),
            evaluation_url: Mutex::new(None),
        })
    }

    #[must_use]
    pub fn with_verifier(mut self, verifier: CwtVerifier) -> Self {
        self.verifier = verifier;
        self
    }

    pub async fn authorize_mcp_tool(&self, token: &str, tool_name: &str) -> Result<Decision> {
        match self
            .authorize_mcp_method(
                "tools/call",
                Some(&serde_json::json!({ "name": tool_name })),
                Some(token),
            )
            .await
        {
            Ok(d) => Ok(d),
            Err(AuthorizationError::Denied) => Ok(Decision::Deny),
            Err(e) => Err(e),
        }
    }

    pub async fn authorize_mcp_method(
        &self,
        method: &str,
        params: Option<&Value>,
        subject_cwt: Option<&str>,
    ) -> Result<Decision> {
        if is_pass_through(method) {
            return Ok(Decision::Permit);
        }
        if !is_hardcoded_mapped(method) {
            info!(
                event = "auth_decision",
                action = "deny",
                method,
                "Unknown MCP method denied"
            );
            return Err(AuthorizationError::Denied);
        }

        let token = subject_cwt_from(subject_cwt).ok_or_else(|| {
            AuthorizationError::Mapping(
                "missing subject CWT (Authorization Bearer or CLAUDE_CODE_SESSION_ACCESS_TOKEN)"
                    .into(),
            )
        })?;

        if method == "tools/call" {
            let tool_name = tool_name_from_params(params)?;
            if McpToolMapping::is_safe_diagnostic(tool_name)
                || self
                    .config
                    .safe_diagnostic_tools
                    .iter()
                    .any(|t| t == tool_name)
            {
                info!(
                    event = "auth_decision",
                    action = "permit",
                    resource = tool_name,
                    "Safe diagnostic tool allowed"
                );
                return Ok(Decision::Permit);
            }
        }

        let claims = self.verifier.verify(&token).await?;
        let token_json = token_map(&claims);
        let platform = self.config.platform_audience.as_deref();
        let (action_name, resource_type, resource_id, sarc) = if method == "tools/list" {
            let slug = self.config.mcp_server_slug();
            let sarc = sarc_tools_list(&claims, &self.config.mcp_resource_id, &slug, platform)?;
            ("tools/list", "mcp_server", slug, sarc)
        } else {
            let tool_name = tool_name_from_params(params)?;
            let slug = crate::cwt_subject::tool_value_slug(tool_name);
            let sarc = sarc_tools_call(&claims, tool_name, platform);
            ("tools/call", "tool", slug, sarc)
        };

        if !trust_anchor_ok(&sarc, &token_json) {
            return Err(AuthorizationError::Mapping(
                "subject.id does not equal $token.sub".into(),
            ));
        }

        let pdp = self.config.pdp_origin();
        if let Some(cached) =
            self.cache
                .get(&pdp, &claims.sub, action_name, resource_type, &resource_id)
        {
            info!(event = "auth_decision", action = ?cached, method, "Authorization cache hit");
            return match cached {
                Decision::Permit => Ok(Decision::Permit),
                Decision::Deny => Err(AuthorizationError::Denied),
            };
        }

        let decision = self.evaluate(&sarc).await?;
        let ttl =
            DecisionCache::calculate_ttl_from_token(Some(i64::try_from(claims.exp).unwrap_or(0)));
        self.cache.put(
            &pdp,
            &claims.sub,
            action_name,
            resource_type,
            &resource_id,
            decision.clone(),
            Some(ttl),
        );
        match decision {
            Decision::Permit => Ok(Decision::Permit),
            Decision::Deny => Err(AuthorizationError::Denied),
        }
    }

    pub async fn authorize_mcp_tools_bulk(
        &self,
        token: &str,
        tool_names: Vec<&str>,
    ) -> Result<Vec<(String, Decision)>> {
        let mut results = Vec::new();
        for tool_name in tool_names {
            match self.authorize_mcp_tool(token, tool_name).await {
                Ok(d) => results.push((tool_name.to_string(), d)),
                Err(AuthorizationError::Denied) => {
                    results.push((tool_name.to_string(), Decision::Deny));
                }
                Err(e) => return Err(e),
            }
        }
        Ok(results)
    }

    async fn evaluate(&self, sarc: &Value) -> Result<Decision> {
        let url = self.evaluation_endpoint().await?;
        let service = self.service_bearer().await?;
        let mut retries = 0;
        loop {
            let response = self
                .http_client
                .post(url.clone())
                .header("Content-Type", "application/json")
                .header("Authorization", format!("Bearer {service}"))
                .json(sarc)
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => {
                    let parsed: AuthzenEvaluationResponse = resp.json().await?;
                    if required_obligations_nonempty(parsed.context.as_ref()) {
                        info!(
                            event = "auth_decision",
                            action = "deny",
                            reason = "obligations",
                            "Fail closed on required obligations"
                        );
                        return Err(AuthorizationError::Denied);
                    }
                    return if parsed.decision {
                        Ok(Decision::Permit)
                    } else {
                        Ok(Decision::Deny)
                    };
                }
                Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED => {
                    return Err(AuthorizationError::PdpUnavailable(
                        "PEP service CWT rejected by PDP (401)".into(),
                    ));
                }
                Ok(resp)
                    if resp.status().is_server_error() && retries < self.config.max_retries =>
                {
                    warn!(
                        "PDP server error, retrying... (attempt {}/{})",
                        retries + 1,
                        self.config.max_retries
                    );
                    retries += 1;
                    tokio::time::sleep(Duration::from_millis(100 * (1 << retries))).await;
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    error!("AuthZEN evaluation failed: {status} {body}");
                    return Err(AuthorizationError::PdpUnavailable(format!(
                        "Status {status}"
                    )));
                }
                Err(e) if retries < self.config.max_retries => {
                    warn!("AuthZEN request error, retrying: {e}");
                    retries += 1;
                    tokio::time::sleep(Duration::from_millis(100 * (1 << retries))).await;
                }
                Err(e) => return Err(AuthorizationError::PdpUnavailable(e.to_string())),
            }
        }
    }

    async fn evaluation_endpoint(&self) -> Result<Url> {
        if let Some(u) = &self.config.evaluation_endpoint {
            return Ok(u.clone());
        }
        {
            let cached = self.evaluation_url.lock().await;
            if let Some(u) = cached.as_ref() {
                return Ok(u.clone());
            }
        }
        let well_known = format!(
            "{}/.well-known/authzen-configuration",
            self.config.pdp_origin()
        );
        let resp = self
            .http_client
            .get(&well_known)
            .send()
            .await
            .map_err(|e| AuthorizationError::PdpUnavailable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(AuthorizationError::PdpUnavailable(format!(
                "discovery HTTP {}",
                resp.status()
            )));
        }
        let disc: AuthzenDiscovery = resp
            .json()
            .await
            .map_err(|e| AuthorizationError::PdpUnavailable(format!("discovery JSON: {e}")))?;
        if let Some(pdp) = &disc.policy_decision_point {
            let want = self.config.pdp_origin();
            if pdp.trim_end_matches('/') != want {
                return Err(AuthorizationError::PdpUnavailable(
                    "policy_decision_point mismatch".into(),
                ));
            }
        }
        let endpoint = disc.access_evaluation_endpoint.ok_or_else(|| {
            AuthorizationError::PdpUnavailable("missing access_evaluation_endpoint".into())
        })?;
        let url =
            Url::parse(&endpoint).map_err(|e| AuthorizationError::PdpUnavailable(e.to_string()))?;
        *self.evaluation_url.lock().await = Some(url.clone());
        Ok(url)
    }

    async fn service_bearer(&self) -> Result<String> {
        if let Some(t) = &self.config.service_token {
            return Ok(t.clone());
        }
        let (token_url, client_id, client_secret) = match (
            &self.config.token_url,
            &self.config.client_id,
            &self.config.client_secret,
        ) {
            (Some(u), Some(id), Some(sec)) => (u, id, sec),
            _ => {
                return Err(AuthorizationError::PdpUnavailable(
                    "AUTHZEN_TOKEN_URL / AUTHZEN_CLIENT_ID / AUTHZEN_CLIENT_SECRET required".into(),
                ));
            }
        };
        let mut cache = self.token_cache.lock().await;
        if let Some(cached) = cache.as_ref()
            && Instant::now() < cached.expires_at
        {
            return Ok(cached.token.clone());
        }
        let resp = self
            .http_client
            .post(token_url)
            .form(&[
                ("grant_type", "client_credentials"),
                ("client_id", client_id.as_str()),
                ("client_secret", client_secret.as_str()),
            ])
            .send()
            .await
            .map_err(|e| AuthorizationError::PdpUnavailable(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(AuthorizationError::PdpUnavailable(format!(
                "token endpoint HTTP {}",
                resp.status()
            )));
        }
        let parsed: TokenResponse = resp
            .json()
            .await
            .map_err(|e| AuthorizationError::PdpUnavailable(e.to_string()))?;
        let ttl = Duration::from_secs(parsed.expires_in.max(120));
        let expires_at = Instant::now() + ttl.saturating_sub(TOKEN_REFRESH_MARGIN);
        let token = parsed.access_token.clone();
        *cache = Some(CachedToken {
            token: parsed.access_token,
            expires_at,
        });
        drop(cache);
        Ok(token)
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    pub fn evict_expired_cache(&self) {
        self.cache.evict_expired();
    }
}

fn required_obligations_nonempty(context: Option<&Value>) -> bool {
    context
        .and_then(|c| c.get("obligations"))
        .and_then(|o| o.get("required"))
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty())
}
