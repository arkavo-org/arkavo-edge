#![allow(clippy::disallowed_methods)]

use crate::cwt_subject::Aud;
use crate::cwt_verify::test_support::{keypair, mint, pe_token};
use crate::error::jsonrpc_codes;
use crate::types::McpToolMapping;
use crate::*;
use arkavo_test_macros::spec;
use serde_json::json;
use std::time::Duration;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const ISS: &str = "https://identity.arkavo.net";
const KID: &[u8] = b"kid-1";
const SVC: &str = "service-cwt";

async fn mock_pdp_permit(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .and(header("Authorization", format!("Bearer {SVC}")))
        .and(header("Content-Type", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "decision": true,
            "context": { "obligations": { "required": [] } }
        })))
        .mount(server)
        .await;
}

fn test_client(server: &MockServer, vk: p256::ecdsa::VerifyingKey) -> AuthorizationClient {
    let eval = url::Url::parse(&format!("{}/access/v1/evaluation", server.uri())).unwrap();
    let config = AuthorizationConfig::default()
        .with_pdp_url(&server.uri())
        .unwrap()
        .with_evaluation_endpoint(eval)
        .with_service_token(SVC)
        .with_timeout(Duration::from_secs(2));
    let mut config = config;
    config.oidc_issuer = Some(ISS.into());
    config.mcp_resource_id = "https://mcp.arkavo.net".into();
    config.max_retries = 1;
    let verifier = crate::cwt_verify::CwtVerifier::with_static_keys(vec![(KID.to_vec(), vk)])
        .with_expected_issuer(ISS.into())
        .with_audiences(vec!["https://mcp.arkavo.net".into(), "arkavo".into()]);
    AuthorizationClient::new(config)
        .unwrap()
        .with_verifier(verifier)
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

#[test]
fn test_mcp_tool_mapping_underscores_not_dots() {
    let resource = McpToolMapping::tool_to_resource("git_commit");
    assert_eq!(
        resource.attribute_value_fqns[0],
        "https://arkavo.net/attr/mcp-tool/value/git_commit"
    );
    let resource = McpToolMapping::tool_to_resource("git.commit");
    assert_eq!(
        resource.attribute_value_fqns[0],
        "https://arkavo.net/attr/mcp-tool/value/git_commit"
    );
    assert!(!resource.attribute_value_fqns[0].contains("git.commit"));
}

#[test]
fn test_safe_diagnostic_excludes_list_tools() {
    assert!(McpToolMapping::is_safe_diagnostic("status"));
    assert!(McpToolMapping::is_safe_diagnostic("health"));
    assert!(McpToolMapping::is_safe_diagnostic("version"));
    assert!(!McpToolMapping::is_safe_diagnostic("list_tools"));
    assert!(!McpToolMapping::is_safe_diagnostic("git_commit"));
}

#[tokio::test]
async fn test_safe_diagnostic_tools() {
    let config = AuthorizationConfig::default();
    let client = AuthorizationClient::new(config).unwrap();
    assert_eq!(
        client.authorize_mcp_tool("dummy", "status").await.unwrap(),
        Decision::Permit
    );
    assert_eq!(
        client.authorize_mcp_tool("dummy", "health").await.unwrap(),
        Decision::Permit
    );
}

#[tokio::test]
async fn test_tools_call_permit_uses_service_bearer() {
    let mock_server = MockServer::start().await;
    mock_pdp_permit(&mock_server).await;
    let (sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let token = pe_token(&sk, KID, now());
    let decision = client
        .authorize_mcp_tool(&token, "git.commit")
        .await
        .unwrap();
    assert_eq!(decision, Decision::Permit);
}

#[tokio::test]
async fn test_tools_call_deny_and_jsonrpc_code() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "decision": false })))
        .mount(&mock_server)
        .await;
    let (sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let token = pe_token(&sk, KID, now());
    let err = client
        .authorize_mcp_method(
            "tools/call",
            Some(&json!({"name": "filesystem_write"})),
            Some(&token),
        )
        .await
        .unwrap_err();
    assert!(matches!(err, AuthorizationError::Denied));
    assert_eq!(err.jsonrpc_code(), jsonrpc_codes::DENIED);
}

#[tokio::test]
async fn test_tools_list_requires_resource_id_in_aud() {
    let mock_server = MockServer::start().await;
    let (sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let token = mint(
        &sk,
        KID,
        ISS,
        "arkavo:550e8400-e29b-41d4-a716-446655440000",
        Aud::One("arkavo".into()),
        now(),
        now() + 3600,
        &[],
        true,
        true,
        true,
    );
    let err = client
        .authorize_mcp_method("tools/list", Some(&json!({})), Some(&token))
        .await
        .unwrap_err();
    assert!(matches!(err, AuthorizationError::Mapping(_)));
    assert_eq!(err.jsonrpc_code(), jsonrpc_codes::MAPPING);
}

#[tokio::test]
async fn test_tools_list_permit() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .and(body_string_contains("mcp_server"))
        .and(body_string_contains("mcp_arkavo_net"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "decision": true })))
        .mount(&mock_server)
        .await;
    let (sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let token = pe_token(&sk, KID, now());
    let decision = client
        .authorize_mcp_method("tools/list", Some(&json!({})), Some(&token))
        .await
        .unwrap();
    assert_eq!(decision, Decision::Permit);
}

#[tokio::test]
async fn test_unknown_method_denied_ping_passthrough() {
    let mock_server = MockServer::start().await;
    let (_sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    assert_eq!(
        client
            .authorize_mcp_method("ping", None, None)
            .await
            .unwrap(),
        Decision::Permit
    );
    assert_eq!(
        client
            .authorize_mcp_method("notifications/initialized", None, None)
            .await
            .unwrap(),
        Decision::Permit
    );
    let err = client
        .authorize_mcp_method("resources/list", None, Some("x"))
        .await
        .unwrap_err();
    assert_eq!(err.jsonrpc_code(), jsonrpc_codes::DENIED);
}

#[tokio::test]
async fn test_pdp_unavailable_and_obligations_fail_closed() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock_server)
        .await;
    let (sk, vk) = keypair();
    let mut client_cfg_unavailable = test_client(&mock_server, vk);
    let token = pe_token(&sk, KID, now());
    let err = client_cfg_unavailable
        .authorize_mcp_method(
            "tools/call",
            Some(&json!({"name": "git_commit"})),
            Some(&token),
        )
        .await
        .unwrap_err();
    assert_eq!(err.jsonrpc_code(), jsonrpc_codes::PDP);

    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "decision": true,
            "context": { "obligations": { "required": ["https://example/obl"] } }
        })))
        .mount(&mock_server)
        .await;
    client_cfg_unavailable = test_client(&mock_server, vk);
    let err = client_cfg_unavailable
        .authorize_mcp_method(
            "tools/call",
            Some(&json!({"name": "git_commit"})),
            Some(&token),
        )
        .await
        .unwrap_err();
    assert_eq!(err.jsonrpc_code(), jsonrpc_codes::DENIED);
}

#[tokio::test]
async fn test_cache_hit_skips_second_pdp_call() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "decision": true })))
        .expect(1)
        .mount(&mock_server)
        .await;
    let (sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let token = pe_token(&sk, KID, now());
    assert_eq!(
        client
            .authorize_mcp_tool(&token, "git_commit")
            .await
            .unwrap(),
        Decision::Permit
    );
    assert_eq!(
        client
            .authorize_mcp_tool(&token, "git_commit")
            .await
            .unwrap(),
        Decision::Permit
    );
}

#[tokio::test]
async fn test_ttl_from_token_exp_not_jwt_split() {
    let ttl = crate::cache::DecisionCache::calculate_ttl_from_token(Some(now() + 30));
    assert!(ttl <= Duration::from_secs(30));
    let expired = crate::cache::DecisionCache::calculate_ttl_from_token(Some(now() - 10));
    assert_eq!(expired, Duration::from_mins(1));
}

#[tokio::test]
async fn test_missing_token_is_mapping_error() {
    let mock_server = MockServer::start().await;
    let (_sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let err = client
        .authorize_mcp_method("tools/call", Some(&json!({"name": "x"})), None)
        .await
        .unwrap_err();
    assert_eq!(err.jsonrpc_code(), jsonrpc_codes::MAPPING);
}

#[tokio::test]
async fn test_service_cwt_from_client_credentials() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "minted-service-cwt",
            "expires_in": 3600
        })))
        .expect(1)
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .and(header("Authorization", "Bearer minted-service-cwt"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "decision": true })))
        .mount(&mock_server)
        .await;

    let (sk, vk) = keypair();
    let eval = url::Url::parse(&format!("{}/access/v1/evaluation", mock_server.uri())).unwrap();
    let mut config = AuthorizationConfig::default()
        .with_pdp_url(&mock_server.uri())
        .unwrap()
        .with_evaluation_endpoint(eval);
    config.token_url = Some(format!("{}/oauth/token", mock_server.uri()));
    config.client_id = Some("mcp-edge".into());
    config.client_secret = Some("secret".into());
    config.oidc_issuer = Some(ISS.into());
    config.mcp_resource_id = "https://mcp.arkavo.net".into();
    config.service_token = None;
    let verifier = crate::cwt_verify::CwtVerifier::with_static_keys(vec![(KID.to_vec(), vk)])
        .with_expected_issuer(ISS.into())
        .with_audiences(vec!["https://mcp.arkavo.net".into()]);
    let client = AuthorizationClient::new(config)
        .unwrap()
        .with_verifier(verifier);
    let token = pe_token(&sk, KID, now());
    assert_eq!(
        client
            .authorize_mcp_tool(&token, "git_commit")
            .await
            .unwrap(),
        Decision::Permit
    );
}

#[tokio::test]
async fn test_discovery_then_evaluate() {
    let mock_server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/.well-known/authzen-configuration"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "policy_decision_point": mock_server.uri(),
            "access_evaluation_endpoint": format!("{}/access/v1/evaluation", mock_server.uri())
        })))
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "decision": true })))
        .mount(&mock_server)
        .await;
    let (sk, vk) = keypair();
    let mut config = AuthorizationConfig::default()
        .with_pdp_url(&mock_server.uri())
        .unwrap()
        .with_service_token(SVC);
    config.evaluation_endpoint = None;
    config.oidc_issuer = Some(ISS.into());
    config.mcp_resource_id = "https://mcp.arkavo.net".into();
    let verifier = crate::cwt_verify::CwtVerifier::with_static_keys(vec![(KID.to_vec(), vk)])
        .with_expected_issuer(ISS.into())
        .with_audiences(vec!["https://mcp.arkavo.net".into()]);
    let client = AuthorizationClient::new(config)
        .unwrap()
        .with_verifier(verifier);
    let token = pe_token(&sk, KID, now());
    assert_eq!(
        client.authorize_mcp_tool(&token, "echo").await.unwrap(),
        Decision::Permit
    );
}

#[spec("AUTHZ-001")]
#[tokio::test]
async fn test_authz_001_get_decision_permit() {
    let mock_server = MockServer::start().await;
    mock_pdp_permit(&mock_server).await;
    let (sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let token = pe_token(&sk, KID, now());
    let decision = client
        .authorize_mcp_tool(&token, "git_commit")
        .await
        .unwrap();
    assert_eq!(decision, Decision::Permit);
}

#[spec("AUTHZ-001")]
#[tokio::test]
async fn test_authz_001_get_decision_deny() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "decision": false })))
        .mount(&mock_server)
        .await;
    let (sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let token = pe_token(&sk, KID, now());
    let decision = client
        .authorize_mcp_tool(&token, "filesystem.write")
        .await
        .unwrap();
    assert_eq!(decision, Decision::Deny);
}

#[spec("AUTHZ-001")]
#[tokio::test]
async fn test_authz_001_get_decision_service_unavailable() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&mock_server)
        .await;
    let (sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let token = pe_token(&sk, KID, now());
    let result = client.authorize_mcp_tool(&token, "git_commit").await;
    assert!(result.is_err());
    assert_eq!(result.unwrap_err().jsonrpc_code(), jsonrpc_codes::PDP);
}

#[spec("AUTHZ-001")]
#[tokio::test]
async fn test_authz_001_get_decision_caches_with_ttl() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "decision": true })))
        .expect(1)
        .mount(&mock_server)
        .await;
    let (sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let token = pe_token(&sk, KID, now());
    assert_eq!(
        client
            .authorize_mcp_tool(&token, "git_commit")
            .await
            .unwrap(),
        Decision::Permit
    );
    assert_eq!(
        client
            .authorize_mcp_tool(&token, "git_commit")
            .await
            .unwrap(),
        Decision::Permit
    );
}

#[tokio::test]
async fn test_bulk_authorization() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .and(body_string_contains("git_commit"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "decision": true })))
        .mount(&mock_server)
        .await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .and(body_string_contains("filesystem_write"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "decision": false })))
        .mount(&mock_server)
        .await;
    let (sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let token = pe_token(&sk, KID, now());
    let results = client
        .authorize_mcp_tools_bulk(&token, vec!["git.commit", "filesystem.write", "status"])
        .await
        .unwrap();
    assert_eq!(results.len(), 3);
    assert_eq!(
        results.iter().find(|(n, _)| n == "status").unwrap().1,
        Decision::Permit
    );
    assert_eq!(
        results.iter().find(|(n, _)| n == "git.commit").unwrap().1,
        Decision::Permit
    );
    assert_eq!(
        results
            .iter()
            .find(|(n, _)| n == "filesystem.write")
            .unwrap()
            .1,
        Decision::Deny
    );
}

#[tokio::test]
async fn test_user_cwt_not_sent_as_authzen_bearer() {
    let mock_server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .and(header("Authorization", format!("Bearer {SVC}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "decision": true })))
        .expect(1)
        .mount(&mock_server)
        .await;
    let (sk, vk) = keypair();
    let client = test_client(&mock_server, vk);
    let token = pe_token(&sk, KID, now());
    assert_ne!(token, SVC);
    client.authorize_mcp_tool(&token, "echo").await.unwrap();
}
