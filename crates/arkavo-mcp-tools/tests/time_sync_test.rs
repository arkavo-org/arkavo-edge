#![allow(clippy::disallowed_methods, clippy::uninlined_format_args)]

use arkavo_mcp_tools::{
    Tool, ToolRegistry,
    time_sync::{GetAgentTimeTool, SyncAgentTimeTool},
};
use arkavo_memory::MemoryStorage;
use serde_json::json;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::net::UdpSocket;

async fn create_test_registry() -> ToolRegistry {
    let storage = Arc::new(MemoryStorage::new_test().await.expect("Storage init"));
    ToolRegistry::new(storage)
}

/// Seconds between the NTP epoch (1900) and the Unix epoch (1970).
const NTP_EPOCH_OFFSET: u64 = 2_208_988_800;

/// Start an SNTP responder on an ephemeral loopback port and return that
/// port.
///
/// The sync tests used to query public NTP servers, so whether their success
/// branch ran at all depended on the runner's network, and a slow or dropped
/// reply cost them a two-second timeout per server. A responder on 127.0.0.1
/// always answers, which lets them require success. The crate's own
/// `NtpServer` sits behind the `ntp-server` feature that CI does not build,
/// and cannot report the port it bound, so this is the few lines of RFC 4330
/// the client needs.
///
/// It runs as a task on the calling test's runtime, which `#[tokio::test]`
/// makes single-threaded, so a sync tool that blocked that thread while
/// waiting for the reply would starve the responder and fail the test.
async fn start_local_sntp_server(stratum: u8) -> u16 {
    let socket = UdpSocket::bind("127.0.0.1:0")
        .await
        .expect("bind local SNTP responder");
    let port = socket.local_addr().expect("responder address").port();
    tokio::spawn(async move {
        let mut request = [0u8; 48];
        while let Ok((len, peer)) = socket.recv_from(&mut request).await {
            if len < request.len() {
                continue;
            }
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after the Unix epoch");
            let seconds = (now.as_secs() + NTP_EPOCH_OFFSET) as u32;
            let fraction = ((u64::from(now.subsec_nanos()) << 32) / 1_000_000_000) as u32;
            let mut timestamp = [0u8; 8];
            timestamp[..4].copy_from_slice(&seconds.to_be_bytes());
            timestamp[4..].copy_from_slice(&fraction.to_be_bytes());

            let mut response = [0u8; 48];
            // The client rejects a reply whose version differs from its
            // request's, so echo it and set mode 4 (server).
            response[0] = (request[0] & 0x38) | 4;
            response[1] = stratum;
            // Origin is the client's transmit timestamp, which is how the
            // client matches the reply to its request.
            response[24..32].copy_from_slice(&request[40..48]);
            response[32..40].copy_from_slice(&timestamp);
            response[40..48].copy_from_slice(&timestamp);
            let _ = socket.send_to(&response, peer).await;
        }
    });
    port
}

#[tokio::test]
async fn test_time_tools_registered() {
    let registry = create_test_registry().await;

    assert!(registry.get("get_agent_time").is_some());
    assert!(registry.get("sync_agent_time").is_some());
    assert!(registry.get("get_time_status").is_some());
}

#[tokio::test]
async fn test_time_tools_categorized() {
    let registry = create_test_registry().await;
    let categories = registry.list_by_category();

    assert!(categories.contains_key("System"));

    let system_tools = &categories["System"];
    let time_tool_names: Vec<String> = system_tools
        .iter()
        .map(|t| t.name.clone())
        .filter(|n| n.contains("time") || n.contains("sync"))
        .collect();

    assert!(time_tool_names.contains(&"get_agent_time".to_string()));
    assert!(time_tool_names.contains(&"sync_agent_time".to_string()));
    assert!(time_tool_names.contains(&"get_time_status".to_string()));
}

#[tokio::test]
async fn test_get_agent_time_execution() {
    let registry = create_test_registry().await;
    let tool = registry.get("get_agent_time").unwrap();

    let result = tool
        .execute(json!({
            "format": "rfc3339"
        }))
        .await;

    assert!(result.is_ok());
    let response = result.unwrap();

    assert!(response["timestamp"].as_str().is_some());
    assert_eq!(response["format"], "rfc3339");
    assert!(response["unix_seconds"].as_i64().is_some());
    assert_eq!(response["source"], "system_clock");
}

#[tokio::test]
async fn test_get_agent_time_all_formats() {
    let tool = GetAgentTimeTool::new();

    let formats = vec!["rfc3339", "unix", "iso8601"];

    for format in formats {
        let result = tool
            .execute(json!({
                "format": format
            }))
            .await;

        assert!(result.is_ok(), "Format {} failed", format);
        let response = result.unwrap();
        assert_eq!(response["format"], format);
    }
}

#[tokio::test]
async fn test_sync_agent_time_with_local_server() {
    let port = start_local_sntp_server(2).await;
    let tool = SyncAgentTimeTool::new();

    let response = tool
        .execute(json!({
            "server": "127.0.0.1",
            "port": port,
            "protocol": "sntp"
        }))
        .await
        .expect("sync tool call");

    assert_eq!(response["success"], true, "local sync failed: {response}");
    assert_eq!(response["server"], "127.0.0.1");
    assert_eq!(response["port"], port);
    assert!(response["offset_ms"].is_number());
    assert!(response["rtt_ms"].is_number());
    assert_eq!(response["stratum"], 2);
    assert_eq!(response["applied"], false);
    assert!(response["recommendation"].as_str().is_some());

    // The responder reads the same clock as the client.
    let offset_ms = response["offset_ms"].as_f64().unwrap();
    assert!(offset_ms.abs() < 10000.0);
}

#[tokio::test]
#[ignore = "live: queries time.cloudflare.com over the public internet"]
async fn test_sync_agent_time_with_public_server() {
    let tool = SyncAgentTimeTool::new();

    let result = tool
        .execute(json!({
            "server": "time.cloudflare.com",
            "protocol": "sntp"
        }))
        .await;

    assert!(result.is_ok());
    let response = result.unwrap();

    if response["success"] == true {
        assert!(response["offset_ms"].is_number());
        assert!(response["rtt_ms"].is_number());
        assert!(response["stratum"].is_number());
        assert_eq!(response["applied"], false);
        assert!(response["recommendation"].as_str().is_some());

        let offset_ms = response["offset_ms"].as_f64().unwrap();
        assert!(offset_ms.abs() < 10000.0);
    } else {
        println!("NTP sync failed (network issue): {}", response["error"]);
    }
}

#[tokio::test]
async fn test_sync_agent_time_invalid_protocol() {
    let tool = SyncAgentTimeTool::new();

    let result = tool
        .execute(json!({
            "server": "time.cloudflare.com",
            "protocol": "ntp"
        }))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_get_time_status_initial() {
    use arkavo_mcp_tools::time_sync::GetTimeStatusTool;
    use std::sync::{Arc, Mutex};

    let state = Arc::new(Mutex::new(arkavo_mcp_tools::time_sync::LastSyncState {
        timestamp: None,
        server: None,
        offset_ms: None,
        rtt_ms: None,
        error: None,
    }));

    let tool = GetTimeStatusTool::new(state);
    let result = tool.execute(json!({})).await;

    assert!(result.is_ok());
    let response = result.unwrap();

    assert_eq!(response["synchronized"], false);
    assert_eq!(response["health"], "unknown");
}

#[tokio::test]
async fn test_end_to_end_workflow() {
    let port = start_local_sntp_server(2).await;
    let registry = create_test_registry().await;

    let get_time = registry.get("get_agent_time").unwrap();
    let sync_time = registry.get("sync_agent_time").unwrap();
    let get_status = registry.get("get_time_status").unwrap();

    let time1 = get_time.execute(json!({"format": "unix"})).await.unwrap();
    assert!(time1["unix_seconds"].is_number());

    let sync_result = sync_time
        .execute(json!({
            "server": "127.0.0.1",
            "port": port
        }))
        .await
        .unwrap();
    assert_eq!(
        sync_result["success"], true,
        "local sync failed: {sync_result}"
    );

    let status = get_status.execute(json!({})).await.unwrap();

    assert_eq!(status["synchronized"], true);
    assert!(status["last_sync"].is_string());
    assert!(status["offset_ms"].is_number());
    assert!(["healthy", "degraded"].contains(&status["health"].as_str().unwrap()));

    let time2 = get_time
        .execute(json!({"format": "rfc3339"}))
        .await
        .unwrap();
    assert!(time2["timestamp"].is_string());
}

#[tokio::test]
async fn test_multiple_ntp_servers() {
    let tool = SyncAgentTimeTool::new();

    // Each responder reports its own stratum, so a reply cannot be credited
    // to the wrong server.
    for stratum in [1u8, 2, 3] {
        let port = start_local_sntp_server(stratum).await;
        let response = tool
            .execute(json!({
                "server": "127.0.0.1",
                "port": port
            }))
            .await
            .unwrap_or_else(|e| panic!("Server on port {} failed: {}", port, e));

        assert_eq!(response["success"], true, "local sync failed: {response}");
        assert_eq!(response["port"], port);
        assert_eq!(response["stratum"], stratum);
    }
}

#[tokio::test]
#[ignore = "live: queries public NTP pools over the internet"]
async fn test_multiple_public_ntp_servers() {
    let tool = SyncAgentTimeTool::new();

    let servers = vec!["time.cloudflare.com", "time.google.com", "pool.ntp.org"];

    for server in servers {
        let result = tool
            .execute(json!({
                "server": server
            }))
            .await;

        assert!(result.is_ok(), "Server {} failed", server);
        let response = result.unwrap();

        if response["success"] == true {
            assert_eq!(response["server"], server);
            println!(
                "Server {} - offset: {}ms, RTT: {}ms",
                server,
                response["offset_ms"].as_f64().unwrap(),
                response["rtt_ms"].as_f64().unwrap()
            );
        }
    }
}

#[tokio::test]
async fn test_timezone_support() {
    let tool = GetAgentTimeTool::new();

    let timezones = vec!["UTC", "America/New_York", "Europe/London", "Asia/Tokyo"];

    for tz in timezones {
        let result = tool
            .execute(json!({
                "format": "rfc3339",
                "timezone": tz
            }))
            .await;

        assert!(result.is_ok(), "Timezone {} failed", tz);
        let response = result.unwrap();
        assert_eq!(response["timezone"], tz);
    }
}

#[tokio::test]
async fn test_tool_schemas() {
    let registry = create_test_registry().await;
    let tools = registry.list_tools();

    let time_tools: Vec<_> = tools
        .iter()
        .filter(|t| t.name.contains("time") || t.name.contains("sync"))
        .collect();

    assert_eq!(time_tools.len(), 3);

    for tool_info in time_tools {
        assert!(!tool_info.description.is_empty());
        assert_eq!(tool_info.category, "System");
    }
}
