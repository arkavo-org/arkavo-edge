use crate::server::Tool;
use arkavo_mcp::ToolSchema;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sntpc::NtpContext;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tracing::{debug, warn};

const DEFAULT_NTP_PORT: u16 = 123;
const DEFAULT_TIMEOUT_SECS: u64 = 2;
const DEFAULT_NTP_SERVER: &str = "pool.ntp.org";

#[derive(Clone)]
pub struct LastSyncState {
    pub timestamp: Option<DateTime<Utc>>,
    pub server: Option<String>,
    pub offset_ms: Option<f64>,
    pub rtt_ms: Option<f64>,
    pub error: Option<String>,
}

pub struct GetAgentTimeTool {
    schema: ToolSchema,
}

impl GetAgentTimeTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "get_agent_time".to_string(),
                aliases: Some(vec![
                    "time".to_string(),
                    "current_time".to_string(),
                    "now".to_string(),
                    "clock".to_string(),
                    "date".to_string(),
                    "datetime".to_string(),
                    "what_time".to_string(),
                ]),
                description: "Get the current time. Use this tool when asked 'what time is it?' or for current date/time.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "format": {
                            "type": "string",
                            "enum": ["rfc3339", "unix", "iso8601"],
                            "description": "Output format for timestamp (default: rfc3339)"
                        },
                        "timezone": {
                            "type": "string",
                            "description": "Timezone for output (default: UTC)"
                        }
                    },
                    "required": []
                }),
            },
        }
    }

    fn format_time(now: DateTime<Utc>, format: &str, timezone_str: &str) -> crate::Result<Value> {
        let tz = if timezone_str == "UTC" || timezone_str.is_empty() {
            chrono_tz::UTC
        } else {
            timezone_str
                .parse::<chrono_tz::Tz>()
                .map_err(|e| crate::ToolError::InvalidParams(format!("Invalid timezone: {}", e)))?
        };

        let local_time = now.with_timezone(&tz);

        match format {
            "rfc3339" | "" => Ok(json!({
                "timestamp": local_time.to_rfc3339(),
                "format": "rfc3339",
                "timezone": timezone_str,
                "unix_seconds": now.timestamp(),
                "unix_nanos": now.timestamp_subsec_nanos(),
                "source": "system_clock"
            })),
            "unix" => Ok(json!({
                "unix_seconds": now.timestamp(),
                "unix_nanos": now.timestamp_subsec_nanos(),
                "format": "unix",
                "source": "system_clock"
            })),
            "iso8601" => Ok(json!({
                "timestamp": local_time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
                "format": "iso8601",
                "timezone": timezone_str,
                "unix_seconds": now.timestamp(),
                "source": "system_clock"
            })),
            _ => Err(crate::ToolError::InvalidParams(format!(
                "Unknown format: {}",
                format
            ))),
        }
    }
}

impl Default for GetAgentTimeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for GetAgentTimeTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let format = args
            .get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("rfc3339");

        let timezone = args
            .get("timezone")
            .and_then(|v| v.as_str())
            .unwrap_or("UTC");

        let now = Utc::now();
        Self::format_time(now, format, timezone)
    }
}

pub struct SyncAgentTimeTool {
    schema: ToolSchema,
    last_sync: Arc<Mutex<LastSyncState>>,
}

impl SyncAgentTimeTool {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "sync_agent_time".to_string(),
                aliases: None,
                description: "Synchronize agent time with NTP/SNTP server. Returns offset and round-trip time. Does not modify system clock.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "server": {
                            "type": "string",
                            "description": "NTP server address (default: pool.ntp.org)"
                        },
                        "port": {
                            "type": "integer",
                            "description": "NTP server port (default: 123)"
                        },
                        "protocol": {
                            "type": "string",
                            "enum": ["sntp"],
                            "description": "Time protocol to use (default: sntp)"
                        }
                    },
                    "required": []
                }),
            },
            last_sync: Arc::new(Mutex::new(LastSyncState {
                timestamp: None,
                server: None,
                offset_ms: None,
                rtt_ms: None,
                error: None,
            })),
        }
    }

    pub fn last_sync_state(&self) -> Arc<Mutex<LastSyncState>> {
        Arc::clone(&self.last_sync)
    }

    async fn query_ntp_server(server: &str, port: u16) -> crate::Result<(f64, f64, u8)> {
        let server_addr = format!("{}:{}", server, port);

        let socket_addr: SocketAddr = server_addr
            .to_socket_addrs()
            .map_err(|e| crate::ToolError::Other(format!("Failed to resolve server: {}", e)))?
            .next()
            .ok_or_else(|| crate::ToolError::Other("No address resolved".to_string()))?;

        debug!("Querying NTP server: {}", socket_addr);

        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| crate::ToolError::Other(format!("Failed to bind UDP socket: {}", e)))?;

        socket
            .set_read_timeout(Some(Duration::from_secs(DEFAULT_TIMEOUT_SECS)))
            .map_err(|e| crate::ToolError::Other(format!("Failed to set timeout: {}", e)))?;

        let ntp_context = NtpContext::new(sntpc::StdTimestampGen::default());

        let result = sntpc::get_time(socket_addr, &socket, ntp_context)
            .await
            .map_err(|e| crate::ToolError::Other(format!("NTP query failed: {:?}", e)))?;

        let offset_secs = result.offset() as f64 / 1_000_000_000.0;
        let rtt_ms = result.roundtrip() as f64 / 1_000_000.0;
        let stratum = result.stratum();

        let offset_ms = offset_secs * 1000.0;

        debug!(
            "NTP result - offset: {}ms, RTT: {}ms, stratum: {}",
            offset_ms, rtt_ms, stratum
        );

        Ok((offset_ms, rtt_ms, stratum))
    }
}

impl Default for SyncAgentTimeTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SyncAgentTimeTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> crate::Result<Value> {
        let server = args
            .get("server")
            .and_then(|v| v.as_str())
            .unwrap_or(DEFAULT_NTP_SERVER);

        let port = args
            .get("port")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_NTP_PORT as u64) as u16;

        let protocol = args
            .get("protocol")
            .and_then(|v| v.as_str())
            .unwrap_or("sntp");

        if protocol != "sntp" {
            return Err(crate::ToolError::InvalidParams(format!(
                "Unsupported protocol: {}. Only 'sntp' is supported.",
                protocol
            )));
        }

        match Self::query_ntp_server(server, port).await {
            Ok((offset_ms, rtt_ms, stratum)) => {
                let recommendation = if offset_ms.abs() > 1000.0 {
                    format!(
                        "Clock is {:.1}s {}, significant drift detected",
                        offset_ms.abs() / 1000.0,
                        if offset_ms > 0.0 { "fast" } else { "slow" }
                    )
                } else if offset_ms.abs() > 100.0 {
                    format!(
                        "Clock is {:.1}ms {}, consider adjusting",
                        offset_ms.abs(),
                        if offset_ms > 0.0 { "fast" } else { "slow" }
                    )
                } else {
                    "Clock is well synchronized".to_string()
                };

                {
                    let mut state = self.last_sync.lock().unwrap();
                    *state = LastSyncState {
                        timestamp: Some(Utc::now()),
                        server: Some(server.to_string()),
                        offset_ms: Some(offset_ms),
                        rtt_ms: Some(rtt_ms),
                        error: None,
                    };
                }

                Ok(json!({
                    "success": true,
                    "server": server,
                    "port": port,
                    "protocol": protocol,
                    "offset_ms": offset_ms,
                    "rtt_ms": rtt_ms,
                    "stratum": stratum,
                    "precision_ms": rtt_ms / 2.0,
                    "applied": false,
                    "recommendation": recommendation
                }))
            }
            Err(e) => {
                warn!("NTP sync failed: {}", e);

                {
                    let mut state = self.last_sync.lock().unwrap();
                    *state = LastSyncState {
                        timestamp: Some(Utc::now()),
                        server: Some(server.to_string()),
                        offset_ms: None,
                        rtt_ms: None,
                        error: Some(e.to_string()),
                    };
                }

                Ok(json!({
                    "success": false,
                    "server": server,
                    "port": port,
                    "protocol": protocol,
                    "error": e.to_string()
                }))
            }
        }
    }
}

pub struct GetTimeStatusTool {
    schema: ToolSchema,
    sync_tool_state: Arc<Mutex<LastSyncState>>,
}

impl GetTimeStatusTool {
    pub fn new(sync_tool_state: Arc<Mutex<LastSyncState>>) -> Self {
        Self {
            schema: ToolSchema {
                name: "get_time_status".to_string(),
                aliases: Some(vec!["ntp_status".to_string(), "sync_status".to_string()]),
                description: "Check NTP time synchronization health. For current time, use get_agent_time instead.".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {},
                    "required": []
                }),
            },
            sync_tool_state,
        }
    }

    pub fn get_sync_state(&self) -> Arc<Mutex<LastSyncState>> {
        Arc::clone(&self.sync_tool_state)
    }
}

#[async_trait]
impl Tool for GetTimeStatusTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, _args: Value) -> crate::Result<Value> {
        let state = self.sync_tool_state.lock().unwrap();

        let synchronized = state.timestamp.is_some() && state.error.is_none();

        let health = if let Some(ref _error) = state.error {
            "unhealthy"
        } else if let Some(offset_ms) = state.offset_ms {
            if offset_ms.abs() > 1000.0 {
                "degraded"
            } else {
                "healthy"
            }
        } else {
            "unknown"
        };

        // Always include current system time so the model can answer time queries
        let now = Utc::now();

        Ok(json!({
            "current_time": now.to_rfc3339(),
            "current_time_unix": now.timestamp(),
            "synchronized": synchronized,
            "last_sync": state.timestamp.map(|t| t.to_rfc3339()),
            "server": state.server,
            "offset_ms": state.offset_ms,
            "rtt_ms": state.rtt_ms,
            "health": health,
            "last_error": state.error
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_agent_time_rfc3339() {
        let tool = GetAgentTimeTool::new();
        let result = tool.execute(json!({"format": "rfc3339"})).await.unwrap();

        assert!(result["timestamp"].as_str().is_some());
        assert_eq!(result["format"], "rfc3339");
        assert!(result["unix_seconds"].as_i64().is_some());
    }

    #[tokio::test]
    async fn test_get_agent_time_unix() {
        let tool = GetAgentTimeTool::new();
        let result = tool.execute(json!({"format": "unix"})).await.unwrap();

        assert_eq!(result["format"], "unix");
        assert!(result["unix_seconds"].as_i64().is_some());
        assert!(result["unix_nanos"].as_u64().is_some());
    }

    #[tokio::test]
    async fn test_sync_agent_time_schema() {
        let tool = SyncAgentTimeTool::new();
        let schema = tool.schema();

        assert_eq!(schema.name, "sync_agent_time");
        assert!(schema.description.contains("NTP"));
    }

    #[tokio::test]
    async fn test_get_time_status_initial() {
        let state = Arc::new(Mutex::new(LastSyncState {
            timestamp: None,
            server: None,
            offset_ms: None,
            rtt_ms: None,
            error: None,
        }));

        let tool = GetTimeStatusTool::new(state);
        let result = tool.execute(json!({})).await.unwrap();

        assert_eq!(result["synchronized"], false);
        assert_eq!(result["health"], "unknown");
    }
}
