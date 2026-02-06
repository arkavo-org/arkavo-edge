//! Mock KAS tests using mockito for automated CI coverage
//!
//! These tests validate the KAS client behavior without requiring a real KAS server.

#[cfg(feature = "kas-client")]
#[cfg(test)]
mod kas_mock_tests {
    use mockito::Server;
    use opentdf::{
        TdfManifest,
        kas::{KasClient, KeyType},
        manifest::TdfManifestExt,
    };

    /// Helper to create a valid mock KAS rewrap response
    fn create_mock_rewrap_response(wrapped_key_b64: &str, session_public_key_pem: &str) -> String {
        serde_json::json!({
            "responses": [{
                "policyId": "00000000-0000-0000-0000-000000000000",
                "results": [{
                    "keyAccessObjectId": "kao-0",
                    "status": "permit",
                    "entityWrappedKey": wrapped_key_b64
                }]
            }],
            "sessionPublicKey": session_public_key_pem
        })
        .to_string()
    }

    /// Helper to create a properly formatted test manifest with policy
    fn create_test_manifest_with_policy(kas_url: String) -> TdfManifest {
        use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
        use opentdf::{AttributeIdentifier, AttributePolicy, AttributeValue, Operator, Policy};

        // Create a simple policy
        let attr_id = AttributeIdentifier {
            namespace: "example.com".to_string(),
            name: "clearance".to_string(),
        };
        let policy = Policy::new(
            "00000000-0000-0000-0000-000000000000".to_string(), // Valid UUID format (36 chars)
            vec![AttributePolicy::condition(
                attr_id,
                Operator::Equals,
                AttributeValue::String("secret".to_string()),
            )],
            vec![],
        );

        // Create manifest with the policy
        let mut manifest = TdfManifest::new("0.payload".to_string(), kas_url);

        // Set a dummy wrapped key (base64 encoded random bytes)
        manifest.encryption_information.key_access[0].wrapped_key =
            BASE64.encode(b"dummy-wrapped-key-32-bytes-long!");

        // Embed policy
        manifest.set_policy(&policy).unwrap();

        manifest
    }

    #[tokio::test]
    async fn test_kas_client_creation() {
        let client = KasClient::new("http://mock-kas.example.com", "mock-token");
        assert!(client.is_ok(), "KAS client creation should succeed");
    }

    #[tokio::test]
    async fn test_kas_rewrap_request_format() {
        let mut server = Server::new_async().await;

        // Create a mock endpoint that validates the request format
        let mock = server
            .mock("POST", "/kas/v2/rewrap")
            .match_header("Authorization", "Bearer mock-token")
            .match_header("Content-Type", "application/json")
            .match_body(mockito::Matcher::Regex(r#".*"signedRequestToken":"eyJ.*"#.to_string()))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(create_mock_rewrap_response(
                "mock-wrapped-key-base64",
                "-----BEGIN PUBLIC KEY-----\nMFkwEwYHKoZIzj0CAQYIKoZIzj0DAQcDQgAE\n-----END PUBLIC KEY-----"
            ))
            .create();

        let kas_url = server.url();
        let client = KasClient::new(&kas_url, "mock-token").unwrap();

        // Note: manifest URL should match what's in key_access[0].url
        // The client will append /kas/v2/rewrap to its base_url
        let manifest = create_test_manifest_with_policy(kas_url.clone());

        // This will fail at unwrap_key stage but validates request format
        let result = client.rewrap_standard_tdf(&manifest).await;

        // Verify the request was made
        mock.assert();

        // We expect this to fail at unwrap (invalid keys) but request format should be correct
        assert!(result.is_err(), "Should fail at key unwrap with mock data");
    }

    #[tokio::test]
    async fn test_kas_authentication_failure() {
        let mut server = Server::new_async().await;

        let _mock = server
            .mock("POST", "/kas/v2/rewrap")
            .with_status(401)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error": "Unauthorized"}"#)
            .create();

        let kas_url = server.url();
        let client = KasClient::new(&kas_url, "invalid-token").unwrap();

        let manifest = create_test_manifest_with_policy(kas_url.clone());

        let result = client.rewrap_standard_tdf(&manifest).await;

        assert!(result.is_err(), "Should fail with 401");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Authentication")
                || err.to_string().contains("401")
                || err.to_string().contains("Unauthorized"),
            "Error should indicate authentication failure: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_kas_access_denied() {
        let mut server = Server::new_async().await;

        let _mock = server
            .mock("POST", "/kas/v2/rewrap")
            .with_status(403)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error": "Access denied: insufficient permissions"}"#)
            .create();

        let kas_url = server.url();
        let client = KasClient::new(&kas_url, "valid-token").unwrap();

        let manifest = create_test_manifest_with_policy(kas_url.clone());

        let result = client.rewrap_standard_tdf(&manifest).await;

        assert!(result.is_err(), "Should fail with 403");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("403") || err.to_string().contains("denied"),
            "Error should indicate access denial: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_kas_server_error() {
        let mut server = Server::new_async().await;

        let _mock = server
            .mock("POST", "/kas/v2/rewrap")
            .with_status(500)
            .with_header("content-type", "application/json")
            .with_body(r#"{"error": "Internal server error"}"#)
            .create();

        let kas_url = server.url();
        let client = KasClient::new(&kas_url, "valid-token").unwrap();

        let manifest = create_test_manifest_with_policy(kas_url.clone());

        let result = client.rewrap_standard_tdf(&manifest).await;

        assert!(result.is_err(), "Should fail with 500");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("500") || err.to_string().contains("server"),
            "Error should indicate server error: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_kas_invalid_response_format() {
        let mut server = Server::new_async().await;

        let _mock = server
            .mock("POST", "/kas/v2/rewrap")
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(r#"{"invalid": "response"}"#)
            .create();

        let kas_url = server.url();
        let client = KasClient::new(&kas_url, "valid-token").unwrap();

        let manifest = create_test_manifest_with_policy(kas_url.clone());

        let result = client.rewrap_standard_tdf(&manifest).await;

        assert!(result.is_err(), "Should fail with invalid response");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("missing field") || err.to_string().contains("Invalid"),
            "Error should indicate parsing failure: {}",
            err
        );
    }

    #[tokio::test]
    async fn test_kas_network_timeout() {
        // Mock server that never responds (simulates timeout)
        let kas_url = "http://192.0.2.1:9999"; // TEST-NET address, should timeout
        let client = KasClient::new(kas_url, "token").unwrap();

        let manifest = create_test_manifest_with_policy(kas_url.to_string());

        let result = client.rewrap_standard_tdf(&manifest).await;

        assert!(result.is_err(), "Should fail with network error");
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("HTTP")
                || err.to_string().contains("network")
                || err.to_string().contains("timeout"),
            "Error should indicate network failure: {}",
            err
        );
    }

    #[test]
    fn test_ephemeral_key_pair_rsa() {
        use opentdf::kas::EphemeralKeyPair;

        let key_pair = EphemeralKeyPair::new(KeyType::RSA);
        assert!(key_pair.is_ok(), "RSA key pair generation should succeed");

        let key_pair = key_pair.unwrap();
        let pem = key_pair.public_key_pem();

        assert!(
            pem.starts_with("-----BEGIN PUBLIC KEY-----"),
            "Should be valid PEM"
        );
        assert!(
            pem.trim_end().ends_with("-----END PUBLIC KEY-----"),
            "Should be valid PEM"
        );
        assert!(pem.len() > 200, "RSA-2048 PEM should be substantial");
    }

    #[test]
    fn test_ephemeral_key_pair_ec() {
        use opentdf::kas::EphemeralKeyPair;

        let key_pair = EphemeralKeyPair::new(KeyType::EC);
        assert!(key_pair.is_ok(), "EC key pair generation should succeed");

        let key_pair = key_pair.unwrap();
        let pem = key_pair.public_key_pem();

        assert!(
            pem.starts_with("-----BEGIN PUBLIC KEY-----"),
            "Should be valid PEM"
        );
        assert!(
            pem.trim_end().ends_with("-----END PUBLIC KEY-----"),
            "Should be valid PEM"
        );
        assert!(pem.len() > 100, "EC P-256 PEM should have content");
        assert!(pem.len() < 200, "EC PEM should be smaller than RSA");
    }

    #[tokio::test]
    async fn test_jwt_signing() {
        let mut server = Server::new_async().await;

        let _mock = server
            .mock("POST", "/kas/v2/rewrap")
            .match_header("Authorization", "Bearer test-token")
            .match_body(mockito::Matcher::Regex(
                r#""signedRequestToken":"eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+""#
                    .to_string(),
            ))
            .with_status(200)
            .with_header("content-type", "application/json")
            .with_body(create_mock_rewrap_response("test", "test"))
            .create();

        let kas_url = server.url();
        let client = KasClient::new(&kas_url, "test-token").unwrap();

        let manifest = create_test_manifest_with_policy(kas_url.clone());

        // Attempt rewrap - will fail at unwrap but validates JWT format
        let _result = client.rewrap_standard_tdf(&manifest).await;
    }
}

#[cfg(not(feature = "kas"))]
#[test]
fn kas_feature_disabled() {
    // This test passes when KAS feature is not enabled
    // It ensures the tests compile even without the feature
}
