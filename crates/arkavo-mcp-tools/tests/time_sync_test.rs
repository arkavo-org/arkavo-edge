use arkavo_mcp_tools::{
    Tool, ToolRegistry,
    time_sync::{GetAgentTimeTool, GetTimeStatusTool, SyncAgentTimeTool},
};
use serde_json::json;

#[tokio::test]
async fn test_time_tools_registered() {
    let registry = ToolRegistry::new();

    assert!(registry.get("get_agent_time").is_some());
    assert!(registry.get("sync_agent_time").is_some());
    assert!(registry.get("get_time_status").is_some());
}

#[tokio::test]
async fn test_time_tools_categorized() {
    let registry = ToolRegistry::new();
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
    let registry = ToolRegistry::new();
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
    let registry = ToolRegistry::new();

    let get_time = registry.get("get_agent_time").unwrap();
    let sync_time = registry.get("sync_agent_time").unwrap();
    let get_status = registry.get("get_time_status").unwrap();

    let time1 = get_time.execute(json!({"format": "unix"})).await.unwrap();
    assert!(time1["unix_seconds"].is_number());

    let sync_result = sync_time
        .execute(json!({
            "server": "time.cloudflare.com"
        }))
        .await
        .unwrap();

    if sync_result["success"] == true {
        let status = get_status.execute(json!({})).await.unwrap();

        assert_eq!(status["synchronized"], true);
        assert!(status["last_sync"].is_string());
        assert!(status["offset_ms"].is_number());
        assert!(["healthy", "degraded"].contains(&status["health"].as_str().unwrap()));
    }

    let time2 = get_time
        .execute(json!({"format": "rfc3339"}))
        .await
        .unwrap();
    assert!(time2["timestamp"].is_string());
}

#[tokio::test]
async fn test_multiple_ntp_servers() {
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
    let registry = ToolRegistry::new();
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
