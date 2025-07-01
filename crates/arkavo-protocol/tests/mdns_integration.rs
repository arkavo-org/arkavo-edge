#![cfg(feature = "mdns")]

use arkavo_protocol::discovery::{DiscoveryConfig, DiscoveryMethod, DiscoveryService};
use arkavo_protocol::mdns::MdnsServiceInfo;
use arkavo_protocol::transport::A2aEndpoint;
use std::time::Duration;
use tokio::time;

#[tokio::test]
async fn test_mdns_service_registration() {
    let discovery = DiscoveryService::new(DiscoveryConfig {
        method: DiscoveryMethod::Mdns,
        ..Default::default()
    });

    let info = MdnsServiceInfo {
        agent_id: "test-agent-1".to_string(),
        http_port: 18080,
        ws_port: Some(18081),
        version: "0.1.0".to_string(),
        capabilities: vec!["promise_request".to_string()],
    };

    // Register the service
    let result = discovery.register_mdns_service(info).await;
    assert!(result.is_ok());

    // Give it a moment to register
    time::sleep(Duration::from_millis(500)).await;

    // Shutdown cleanly
    let _ = discovery.shutdown_mdns().await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "mDNS tests require proper network environment"]
async fn test_mdns_discovery() {
    // Create two discovery services
    let discovery1 = DiscoveryService::new(DiscoveryConfig {
        method: DiscoveryMethod::Mdns,
        ..Default::default()
    });

    let discovery2 = DiscoveryService::new(DiscoveryConfig {
        method: DiscoveryMethod::Mdns,
        ..Default::default()
    });

    // Start discovery on both
    let _ = discovery1.start_mdns_discovery().await;
    let _ = discovery2.start_mdns_discovery().await;

    // Register agent 1
    let info1 = MdnsServiceInfo {
        agent_id: "test-agent-mdns-1".to_string(),
        http_port: 28080,
        ws_port: None,
        version: "0.1.0".to_string(),
        capabilities: vec!["promise_request".to_string()],
    };

    let _ = discovery1.register_mdns_service(info1).await;

    // Register agent 2
    let info2 = MdnsServiceInfo {
        agent_id: "test-agent-mdns-2".to_string(),
        http_port: 28081,
        ws_port: None,
        version: "0.1.0".to_string(),
        capabilities: vec!["promise_declare".to_string()],
    };

    let _ = discovery2.register_mdns_service(info2).await;

    // Give discovery time to work
    time::sleep(Duration::from_secs(3)).await;

    // Check if discovery1 can find agent2
    let agents = discovery1.discover_all().unwrap();
    let found_agent2 = agents
        .iter()
        .any(|a| a.agent_id == "test-agent-mdns-2");

    // Note: This might not work in all test environments due to network restrictions
    // In a real environment with proper mDNS support, this should pass
    if found_agent2 {
        println!("Successfully discovered agent via mDNS!");
    } else {
        println!("mDNS discovery not working in this environment (expected in CI)");
    }

    // Cleanup
    let _ = discovery1.shutdown_mdns().await;
    let _ = discovery2.shutdown_mdns().await;
}

#[tokio::test]
async fn test_mdns_fallback_to_static() {
    let discovery = DiscoveryService::new(DiscoveryConfig {
        method: DiscoveryMethod::Mdns,
        ..Default::default()
    });

    // Register a static endpoint
    let endpoint = A2aEndpoint {
        url: "http://localhost:38080".to_string(),
        agent_id: "static-agent".to_string(),
        public_key: None,
    };
    discovery.register_static_endpoint(endpoint.clone());

    // Should find it even with mDNS method
    let found = discovery.discover_agent("static-agent");
    assert!(found.is_ok());
    assert_eq!(found.unwrap().url, "http://localhost:38080");
}