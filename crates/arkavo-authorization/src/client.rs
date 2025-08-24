use crate::cache::DecisionCache;
use crate::config::AuthorizationConfig;
use crate::error::{AuthorizationError, Result};
use crate::types::{
    Action, CreateEntityChainsFromTokensRequest, CreateEntityChainsFromTokensResponse, Decision,
    DecisionRequest, EntityIdentifier, GetDecisionBulkRequest, GetDecisionBulkResponse,
    GetDecisionRequest, GetDecisionResponse, McpToolMapping, Resource,
};
use base64::{Engine as _, engine::general_purpose};
use reqwest::{Client, StatusCode};
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, warn};

pub struct AuthorizationClient {
    config: AuthorizationConfig,
    http_client: Client,
    cache: Arc<DecisionCache>,
}

impl AuthorizationClient {
    pub fn new(config: AuthorizationConfig) -> Result<Self> {
        let http_client = Client::builder()
            .timeout(config.timeout)
            .danger_accept_invalid_certs(false)
            .build()
            .map_err(|e| AuthorizationError::ConfigError(e.to_string()))?;

        let cache = Arc::new(DecisionCache::new(1000, config.cache_ttl));

        Ok(Self {
            config,
            http_client,
            cache,
        })
    }

    pub async fn authorize_mcp_tool(&self, token: &str, tool_name: &str) -> Result<Decision> {
        // Check if it's a safe diagnostic tool
        if McpToolMapping::is_safe_diagnostic(tool_name) {
            debug!("Tool {} is safe diagnostic, allowing", tool_name);
            return Ok(Decision::Permit);
        }

        // Get entity from token
        let entity = self.resolve_entity_from_token(token).await?;

        // Create resource for MCP tool
        let resource = McpToolMapping::tool_to_resource(tool_name);
        let action = Action::execute_tool();

        // Check cache first
        if let Some(cached) = self.cache.get(&entity, &action, &resource) {
            debug!("Cache hit for tool {}: {:?}", tool_name, cached);
            return Ok(cached);
        }

        // Make authorization request
        let decision = self.get_decision(&entity, &action, &resource).await?;

        // Cache the decision
        let ttl = Self::extract_token_ttl(token);
        self.cache
            .put(&entity, &action, &resource, decision.clone(), Some(ttl));

        Ok(decision)
    }

    pub async fn authorize_mcp_tools_bulk(
        &self,
        token: &str,
        tool_names: Vec<&str>,
    ) -> Result<Vec<(String, Decision)>> {
        // Filter out safe diagnostic tools
        let mut results = Vec::new();
        let mut tools_to_check = Vec::new();

        for tool_name in &tool_names {
            if McpToolMapping::is_safe_diagnostic(tool_name) {
                results.push(((*tool_name).to_string(), Decision::Permit));
            } else {
                tools_to_check.push(*tool_name);
            }
        }

        if tools_to_check.is_empty() {
            return Ok(results);
        }

        // Get entity from token
        let entity = self.resolve_entity_from_token(token).await?;

        // Build bulk request
        let decision_requests: Vec<DecisionRequest> = tools_to_check
            .iter()
            .map(|tool_name| DecisionRequest {
                entity_identifier: entity.clone(),
                action: Action::execute_tool(),
                resource: McpToolMapping::tool_to_resource(tool_name),
            })
            .collect();

        // Make bulk authorization request
        let response = self.get_decision_bulk(decision_requests).await?;

        // Process responses and cache
        let ttl = Self::extract_token_ttl(token);
        for (tool_name, decision_response) in tools_to_check.iter().zip(response.decision_responses)
        {
            let decision = decision_response.decision;

            // Cache each decision
            self.cache.put(
                &entity,
                &decision_response.action,
                &decision_response.resource,
                decision.clone(),
                Some(ttl),
            );

            results.push(((*tool_name).to_string(), decision));
        }

        Ok(results)
    }

    async fn resolve_entity_from_token(&self, token: &str) -> Result<EntityIdentifier> {
        let url = self
            .config
            .entity_resolution_v2_url("CreateEntityChainsFromTokens");

        let request = CreateEntityChainsFromTokensRequest {
            tokens: vec![token.to_string()],
        };

        let response = self
            .http_client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {token}"))
            .json(&request)
            .send()
            .await
            .map_err(|e| {
                error!("Entity resolution request failed: {}", e);
                AuthorizationError::EntityResolutionError(e.to_string())
            })?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            error!("Entity resolution failed with status {}: {}", status, body);
            return Err(AuthorizationError::EntityResolutionError(format!(
                "Status {status}: {body}"
            )));
        }

        let ers_response: CreateEntityChainsFromTokensResponse = response.json().await?;

        let entity_chain = ers_response
            .entity_chains
            .into_iter()
            .next()
            .ok_or_else(|| {
                AuthorizationError::EntityResolutionError("No entity chains returned".to_string())
            })?;

        let entity = entity_chain.entities.into_iter().next().ok_or_else(|| {
            AuthorizationError::EntityResolutionError("No entities in chain".to_string())
        })?;

        Ok(entity)
    }

    pub async fn get_decision(
        &self,
        entity: &EntityIdentifier,
        action: &Action,
        resource: &Resource,
    ) -> Result<Decision> {
        let url = self.config.authorization_v2_url("GetDecision");

        let request = GetDecisionRequest {
            entity_identifier: entity.clone(),
            action: action.clone(),
            resource: resource.clone(),
        };

        let response = self.make_connect_request(&url, &request).await?;

        let decision_response: GetDecisionResponse = response.json().await?;
        Ok(decision_response.decision)
    }

    async fn get_decision_bulk(
        &self,
        decision_requests: Vec<DecisionRequest>,
    ) -> Result<GetDecisionBulkResponse> {
        let url = self.config.authorization_v2_url("GetDecisionBulk");

        let request = GetDecisionBulkRequest { decision_requests };

        let response = self.make_connect_request(&url, &request).await?;

        let bulk_response: GetDecisionBulkResponse = response.json().await?;
        Ok(bulk_response)
    }

    async fn make_connect_request<T: serde::Serialize + Sync>(
        &self,
        url: &url::Url,
        body: &T,
    ) -> Result<reqwest::Response> {
        let mut retries = 0;

        loop {
            let response = self
                .http_client
                .post(url.clone())
                .header("Content-Type", "application/json")
                .header("Connect-Protocol-Version", "1")
                .json(body)
                .send()
                .await;

            match response {
                Ok(resp) if resp.status().is_success() => return Ok(resp),
                Ok(resp) if resp.status() == StatusCode::UNAUTHORIZED => {
                    return Err(AuthorizationError::InvalidToken("Unauthorized".to_string()));
                }
                Ok(resp) if resp.status() == StatusCode::FORBIDDEN => {
                    return Err(AuthorizationError::Denied);
                }
                Ok(resp)
                    if resp.status().is_server_error() && retries < self.config.max_retries =>
                {
                    warn!(
                        "Server error, retrying... (attempt {}/{})",
                        retries + 1,
                        self.config.max_retries
                    );
                    retries += 1;
                    tokio::time::sleep(Duration::from_millis(100 * (1 << retries))).await;
                }
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    error!("Request failed with status {}: {}", status, body);
                    return Err(AuthorizationError::InvalidResponse(format!(
                        "Status {status}: {body}"
                    )));
                }
                Err(e) if retries < self.config.max_retries => {
                    warn!(
                        "Request error, retrying... (attempt {}/{}): {}",
                        retries + 1,
                        self.config.max_retries,
                        e
                    );
                    retries += 1;
                    tokio::time::sleep(Duration::from_millis(100 * (1 << retries))).await;
                }
                Err(e) => return Err(AuthorizationError::HttpError(e)),
            }
        }
    }

    fn extract_token_ttl(token: &str) -> Duration {
        // Parse JWT to get expiration
        let parts: Vec<&str> = token.split('.').collect();
        if parts.len() != 3 {
            return Duration::from_secs(60);
        }

        if let Ok(decoded) = general_purpose::URL_SAFE_NO_PAD.decode(parts[1])
            && let Ok(claims) = serde_json::from_slice::<serde_json::Value>(&decoded)
            && let Some(exp) = claims.get("exp").and_then(|v| v.as_i64())
        {
            return DecisionCache::calculate_ttl_from_token(Some(exp));
        }

        Duration::from_secs(60)
    }

    pub fn clear_cache(&self) {
        self.cache.clear();
    }

    pub fn evict_expired_cache(&self) {
        self.cache.evict_expired();
    }
}
