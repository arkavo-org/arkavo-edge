#![allow(clippy::disallowed_methods)]
#![allow(clippy::field_reassign_with_default)]

#[cfg(test)]
mod discover_features_tests {
    use arkavo_protocol::{A2aServer, config::ServerConfig};
    use jsonrpsee::{core::client::ClientT, rpc_params, ws_client::WsClientBuilder};
    use serde_json::json;

    #[tokio::test]
    async fn test_discover_features_disclose() {
        // Start test server
        let mut config = ServerConfig::default();
        config.bind_address = "127.0.0.1".to_string();
        config.port = 18901; // Use specific test port
        config.task_store_path = None; // Use in-memory database for tests

        let server = A2aServer::new(config.clone());
        let handle = server.start().await.unwrap();

        // Create client
        let url = format!("ws://{}:{}", config.bind_address, config.port);
        let client = WsClientBuilder::default().build(&url).await.unwrap();

        // Test proactive disclosure
        let response: serde_json::Value = client
            .request("discover_features_disclose", rpc_params![])
            .await
            .unwrap();

        // Verify response structure
        assert!(response.get("disclosures").is_some());
        let disclosures = response["disclosures"].as_array().unwrap();
        assert!(!disclosures.is_empty());

        // Check for expected protocols
        let has_discover_features = disclosures.iter().any(|d| {
            d["feature-type"] == "protocol"
                && d["id"] == "https://didcomm.org/discover-features/2.0"
        });
        assert!(
            has_discover_features,
            "Should have discover-features protocol"
        );

        let has_a2a = disclosures
            .iter()
            .any(|d| d["feature-type"] == "protocol" && d["id"] == "https://arkavo.org/a2a/1.0");
        assert!(has_a2a, "Should have A2A protocol");

        handle.stop().unwrap();
    }

    #[tokio::test]
    async fn test_discover_features_query_with_filter() {
        // Start test server
        let mut config = ServerConfig::default();
        config.bind_address = "127.0.0.1".to_string();
        config.port = 18902; // Use different test port
        config.task_store_path = None; // Use in-memory database for tests

        let server = A2aServer::new(config.clone());
        let handle = server.start().await.unwrap();

        // Create client
        let url = format!("ws://{}:{}", config.bind_address, config.port);
        let client = WsClientBuilder::default().build(&url).await.unwrap();

        // Test query with protocol filter
        let query = json!({
            "queries": [
                {
                    "feature-type": "protocol",
                    "match": "https://didcomm.org/*"
                }
            ]
        });

        let response: serde_json::Value = client
            .request("discover_features_query", rpc_params![query])
            .await
            .unwrap();

        // Verify filtered response
        let disclosures = response["disclosures"].as_array().unwrap();

        // Should only have DIDComm protocols
        for disclosure in disclosures {
            assert_eq!(disclosure["feature-type"], "protocol");
            assert!(
                disclosure["id"]
                    .as_str()
                    .unwrap()
                    .starts_with("https://didcomm.org/")
            );
        }

        handle.stop().unwrap();
    }

    #[tokio::test]
    async fn test_discover_features_query_mcp_tools() {
        // Start test server with MCP
        let mut config = ServerConfig::default();
        config.bind_address = "127.0.0.1".to_string();
        config.port = 18903; // Use another different test port
        config.task_store_path = None; // Use in-memory database for tests

        let server = A2aServer::new(config.clone());

        // Set agent metadata to simulate real agent
        server
            .set_agent_metadata(
                "test-agent".to_string(),
                "Test agent for discover features".to_string(),
                "test-model".to_string(),
                5, // action_interval
            )
            .await;

        let handle = server.start().await.unwrap();

        // Create client
        let url = format!("ws://{}:{}", config.bind_address, config.port);
        let client = WsClientBuilder::default().build(&url).await.unwrap();

        // Query only MCP tools
        let query = json!({
            "queries": [
                {
                    "feature-type": "mcp-tool"
                }
            ]
        });

        let response: serde_json::Value = client
            .request("discover_features_query", rpc_params![query])
            .await
            .unwrap();

        // Verify response contains only MCP tools
        let disclosures = response["disclosures"].as_array().unwrap();
        for disclosure in disclosures {
            assert_eq!(disclosure["feature-type"], "mcp-tool");
        }

        handle.stop().unwrap();
    }
}
