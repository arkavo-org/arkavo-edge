#![allow(clippy::disallowed_methods)]

use crate::types::McpToolMapping;
use crate::*;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn test_safe_diagnostic_tools() {
    let config = AuthorizationConfig::default();
    let client = AuthorizationClient::new(config).unwrap();

    // Safe diagnostic tools should always be permitted
    let decision = client
        .authorize_mcp_tool("dummy_token", "status")
        .await
        .unwrap();
    assert_eq!(decision, Decision::Permit);

    let decision = client
        .authorize_mcp_tool("dummy_token", "health")
        .await
        .unwrap();
    assert_eq!(decision, Decision::Permit);

    let decision = client
        .authorize_mcp_tool("dummy_token", "version")
        .await
        .unwrap();
    assert_eq!(decision, Decision::Permit);
}

#[tokio::test]
async fn test_get_decision_permit() {
    let mock_server = MockServer::start().await;

    // Mock Entity Resolution Service
    Mock::given(method("POST"))
        .and(path(
            "/entityresolution.v2.EntityResolutionService/CreateEntityChainsFromTokens",
        ))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "entity_chains": [{
                "id": "chain1",
                "entities": [{
                    "id": "user123",
                    "type": "subject"
                }]
            }]
        })))
        .mount(&mock_server)
        .await;

    // Mock Authorization Service - Permit
    Mock::given(method("POST"))
        .and(path("/authorization.v2.AuthorizationService/GetDecision"))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "decision": "permit"
        })))
        .mount(&mock_server)
        .await;

    let config = AuthorizationConfig::default()
        .with_base_url(&mock_server.uri())
        .unwrap();
    let client = AuthorizationClient::new(config).unwrap();

    let decision = client
        .authorize_mcp_tool("test_token", "git.commit")
        .await
        .unwrap();
    assert_eq!(decision, Decision::Permit);
}

#[tokio::test]
async fn test_get_decision_deny() {
    let mock_server = MockServer::start().await;

    // Mock Entity Resolution Service
    Mock::given(method("POST"))
        .and(path(
            "/entityresolution.v2.EntityResolutionService/CreateEntityChainsFromTokens",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "entity_chains": [{
                "id": "chain1",
                "entities": [{
                    "id": "user123",
                    "type": "subject"
                }]
            }]
        })))
        .mount(&mock_server)
        .await;

    // Mock Authorization Service - Deny
    Mock::given(method("POST"))
        .and(path("/authorization.v2.AuthorizationService/GetDecision"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "decision": "deny"
        })))
        .mount(&mock_server)
        .await;

    let config = AuthorizationConfig::default()
        .with_base_url(&mock_server.uri())
        .unwrap();
    let client = AuthorizationClient::new(config).unwrap();

    let decision = client
        .authorize_mcp_tool("test_token", "filesystem.write")
        .await
        .unwrap();
    assert_eq!(decision, Decision::Deny);
}

#[tokio::test]
async fn test_bulk_authorization() {
    let mock_server = MockServer::start().await;

    // Mock Entity Resolution Service
    Mock::given(method("POST"))
        .and(path(
            "/entityresolution.v2.EntityResolutionService/CreateEntityChainsFromTokens",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "entity_chains": [{
                "id": "chain1",
                "entities": [{
                    "id": "user123",
                    "type": "subject"
                }]
            }]
        })))
        .mount(&mock_server)
        .await;

    // Mock Bulk Authorization Service
    Mock::given(method("POST"))
        .and(path("/authorization.v2.AuthorizationService/GetDecisionBulk"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "decision_responses": [
                {
                    "entity_identifier": {"id": "user123", "type": "subject"},
                    "action": {"name": "execute_tool"},
                    "resource": {"attribute_value_fqns": ["https://arkavo.net/attr/mcp-tool/value/git.commit"]},
                    "decision": "permit"
                },
                {
                    "entity_identifier": {"id": "user123", "type": "subject"},
                    "action": {"name": "execute_tool"},
                    "resource": {"attribute_value_fqns": ["https://arkavo.net/attr/mcp-tool/value/filesystem.write"]},
                    "decision": "deny"
                }
            ]
        })))
        .mount(&mock_server)
        .await;

    let config = AuthorizationConfig::default()
        .with_base_url(&mock_server.uri())
        .unwrap();
    let client = AuthorizationClient::new(config).unwrap();

    let tools = vec!["git.commit", "filesystem.write", "status"];
    let results = client
        .authorize_mcp_tools_bulk("test_token", tools)
        .await
        .unwrap();

    assert_eq!(results.len(), 3);

    // Check individual results
    let status_result = results.iter().find(|(name, _)| name == "status").unwrap();
    assert_eq!(status_result.1, Decision::Permit);

    let git_result = results
        .iter()
        .find(|(name, _)| name == "git.commit")
        .unwrap();
    assert_eq!(git_result.1, Decision::Permit);

    let fs_result = results
        .iter()
        .find(|(name, _)| name == "filesystem.write")
        .unwrap();
    assert_eq!(fs_result.1, Decision::Deny);
}

#[tokio::test]
async fn test_cache_hit() {
    let mock_server = MockServer::start().await;

    // Mock Entity Resolution Service - called twice (once per authorize_mcp_tool call)
    Mock::given(method("POST"))
        .and(path(
            "/entityresolution.v2.EntityResolutionService/CreateEntityChainsFromTokens",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "entity_chains": [{
                "id": "chain1",
                "entities": [{
                    "id": "user123",
                    "type": "subject"
                }]
            }]
        })))
        .expect(2)
        .mount(&mock_server)
        .await;

    // Mock Authorization Service - should only be called once
    Mock::given(method("POST"))
        .and(path("/authorization.v2.AuthorizationService/GetDecision"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "decision": "permit"
        })))
        .expect(1)
        .mount(&mock_server)
        .await;

    let config = AuthorizationConfig::default()
        .with_base_url(&mock_server.uri())
        .unwrap();
    let client = AuthorizationClient::new(config).unwrap();

    // First call - should hit the server
    let decision1 = client
        .authorize_mcp_tool("test_token", "git.commit")
        .await
        .unwrap();
    assert_eq!(decision1, Decision::Permit);

    // Second call - should hit the cache
    let decision2 = client
        .authorize_mcp_tool("test_token", "git.commit")
        .await
        .unwrap();
    assert_eq!(decision2, Decision::Permit);
}

#[tokio::test]
async fn test_server_error_retry() {
    let mock_server = MockServer::start().await;

    // Mock Entity Resolution Service
    Mock::given(method("POST"))
        .and(path(
            "/entityresolution.v2.EntityResolutionService/CreateEntityChainsFromTokens",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "entity_chains": [{
                "id": "chain1",
                "entities": [{
                    "id": "user123",
                    "type": "subject"
                }]
            }]
        })))
        .mount(&mock_server)
        .await;

    // Mock Authorization Service - use up_to(2) to handle both initial failure and retry
    Mock::given(method("POST"))
        .and(path("/authorization.v2.AuthorizationService/GetDecision"))
        .respond_with(ResponseTemplate::new(503).append_header("content-type", "application/json"))
        .up_to_n_times(1) // First request fails
        .mount(&mock_server)
        .await;

    Mock::given(method("POST"))
        .and(path("/authorization.v2.AuthorizationService/GetDecision"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "decision": "permit"
        })))
        .expect(1) // Second request succeeds
        .mount(&mock_server)
        .await;

    let config = AuthorizationConfig::default()
        .with_base_url(&mock_server.uri())
        .unwrap()
        .with_timeout(std::time::Duration::from_secs(1));
    let client = AuthorizationClient::new(config).unwrap();

    // Should retry and succeed
    let decision = client
        .authorize_mcp_tool("test_token", "git.commit")
        .await
        .unwrap();
    assert_eq!(decision, Decision::Permit);
}

#[test]
fn test_mcp_tool_mapping() {
    let resource = McpToolMapping::tool_to_resource("git_commit");
    assert_eq!(
        resource.attribute_value_fqns[0],
        "https://arkavo.net/attr/mcp-tool/value/git.commit"
    );

    let resource = McpToolMapping::tool_to_resource("filesystem_read");
    assert_eq!(
        resource.attribute_value_fqns[0],
        "https://arkavo.net/attr/mcp-tool/value/filesystem.read"
    );

    let resource = McpToolMapping::tool_to_resource("custom_tool");
    assert_eq!(
        resource.attribute_value_fqns[0],
        "https://arkavo.net/attr/mcp-tool/value/custom_tool"
    );
}

#[test]
fn test_safe_diagnostic_detection() {
    assert!(McpToolMapping::is_safe_diagnostic("status"));
    assert!(McpToolMapping::is_safe_diagnostic("health"));
    assert!(McpToolMapping::is_safe_diagnostic("version"));
    assert!(McpToolMapping::is_safe_diagnostic("list_tools"));

    assert!(!McpToolMapping::is_safe_diagnostic("git_commit"));
    assert!(!McpToolMapping::is_safe_diagnostic("filesystem_write"));
}
