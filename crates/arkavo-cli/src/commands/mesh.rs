use arkavo_protocol::agent_registry::AgentInfo;

/// Discover agents on the mesh network using mDNS
#[cfg(feature = "mdns")]
pub fn discover_mesh_agents() -> Result<Vec<AgentInfo>, Box<dyn std::error::Error>> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    use std::collections::HashMap;
    use std::time::Duration;
    use tracing::info;

    info!("Discovering mesh agents via mDNS...");

    let mdns = ServiceDaemon::new()?;
    let receiver = mdns.browse("_a2a._tcp.local.")?;

    let mut agents = Vec::new();
    let timeout = Duration::from_secs(5);
    let start = std::time::Instant::now();
    let mut last_discovery = std::time::Instant::now();

    while start.elapsed() < timeout {
        // Exit early once we've seen agents and had a quiet period
        if !agents.is_empty() && last_discovery.elapsed() > Duration::from_millis(500) {
            break;
        }
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                if let ServiceEvent::ServiceResolved(info) = event {
                    last_discovery = std::time::Instant::now();
                    let agent_id = info
                        .get_property_val_str("agent_id")
                        .unwrap_or("unknown")
                        .to_string();
                    let name = info.get_fullname().to_string();
                    let purpose = info
                        .get_property_val_str("purpose")
                        .unwrap_or("")
                        .to_string();

                    let capabilities_str = info
                        .get_property_val_str("capabilities")
                        .unwrap_or_default();
                    let mut capabilities: Vec<String> = if capabilities_str.is_empty() {
                        vec![]
                    } else {
                        capabilities_str.split(',').map(|s| s.to_string()).collect()
                    };

                    let mcp_tools_str = info.get_property_val_str("mcp_tools").unwrap_or_default();
                    if !mcp_tools_str.is_empty() {
                        let mcp_tools: Vec<String> =
                            mcp_tools_str.split(',').map(|s| s.to_string()).collect();
                        capabilities.extend(mcp_tools);
                    }

                    let address = info
                        .get_addresses()
                        .iter()
                        .next()
                        .map(|addr| format!("http://{}:{}", addr, info.get_port()));

                    let mut metadata = HashMap::new();
                    if let Some(model) = info.get_property_val_str("model") {
                        metadata.insert("model".to_string(), model.to_string());
                    }

                    // Extract public key for TDF encryption
                    let public_key = info
                        .get_property_val_str("public_key")
                        .map(|s| s.to_string());

                    agents.push(AgentInfo {
                        agent_id,
                        name: name.clone(),
                        purpose,
                        capabilities,
                        device_caps: None,
                        metadata,
                        last_seen: chrono::Utc::now(),
                        load: 0.0,
                        is_available: true,
                        address,
                        public_key,
                        capability_manifest: None,
                        capabilities_queried_at: None,
                        last_specialized_at: None,
                    });

                    tracing::debug!("Discovered agent: {}", name);
                }
            }
            Err(_) => {
                // Timeout on recv - continue until overall timeout
            }
        }
    }

    mdns.shutdown().ok();
    info!("Discovered {} agents via mDNS", agents.len());
    Ok(agents)
}

#[cfg(not(feature = "mdns"))]
pub fn discover_mesh_agents() -> Result<Vec<AgentInfo>, Box<dyn std::error::Error>> {
    tracing::warn!("mDNS feature not compiled in - cannot discover agents");
    Ok(Vec::new())
}

/// An agent's finished answer: the task it ran and every part of its result.
pub struct Answer {
    pub task_id: String,
    pub parts: Vec<arkavo_protocol::types::MessagePart>,
}

impl Answer {
    /// The text of the answer, one line per text part.
    pub fn text(&self) -> String {
        answer_text(&self.parts)
    }
}

/// Send a message to an agent via A2A and poll until the task completes,
/// for as long as the task keeps reporting progress (CLI-019).
///
/// `on_progress` hears each new progress message. The answer itself is left
/// to the caller to print, so nothing of it reaches the terminal for an
/// exchange that failed.
pub async fn send_and_poll_agent(
    transport: &arkavo_protocol::http::HttpTransport,
    text: &str,
    on_progress: &mut dyn FnMut(&str),
) -> Result<Answer, Box<dyn std::error::Error>> {
    use arkavo_protocol::{
        transport::{A2aRequest, A2aResponse, A2aTransport},
        types::{
            Message, MessagePart, MessageSendRequest, MessageSendResponse, TaskGetRequest,
            TaskGetResponse, TaskStatus,
        },
    };
    use std::time::Duration;

    let message = Message {
        parts: vec![MessagePart::Text {
            content: text.to_string(),
        }],
        metadata: Some(serde_json::json!({ "source": "arkavo_chat_cli" })),
    };
    let send_req = MessageSendRequest {
        message,
        task_id: None,
    };
    let rpc = A2aRequest::new("message/send", serde_json::json!([send_req]));

    let response = transport
        .send_request(rpc)
        .await
        .map_err(|e| format!("Failed to send: {e}"))?;

    let task_id = match response {
        A2aResponse::Success { result, .. } => {
            let resp: MessageSendResponse =
                serde_json::from_value(result).map_err(|e| format!("Parse error: {e}"))?;
            resp.task_id
        }
        A2aResponse::Error { error, .. } => {
            return Err(format!("RPC error: {} - {}", error.code, error.message).into());
        }
    };

    let mut watch = super::poll::Watch::default();
    loop {
        let get_req = TaskGetRequest {
            task_id: task_id.clone(),
        };
        let rpc = A2aRequest::new("tasks/get", serde_json::json!([get_req]));
        let response = transport
            .send_request(rpc)
            .await
            .map_err(|e| format!("Poll error: {e}"))?;

        match response {
            A2aResponse::Success { result, .. } => {
                let task_resp: TaskGetResponse =
                    serde_json::from_value(result).map_err(|e| format!("Parse error: {e}"))?;

                match task_resp.status {
                    TaskStatus::Completed => {
                        let parts = task_resp.result.map(|m| m.parts).unwrap_or_default();
                        return Ok(Answer { task_id, parts });
                    }
                    TaskStatus::Failed => {
                        let msg = task_resp
                            .error
                            .map(|e| format!("{}: {}", e.code, e.message))
                            .unwrap_or_else(|| "Unknown error".into());
                        return Err(format!("Agent error: {msg}").into());
                    }
                    TaskStatus::Canceled => {
                        return Err("Task was canceled by agent".into());
                    }
                    _ => {
                        if let Some(message) = watch.observe(&task_resp)? {
                            on_progress(&message);
                        }
                        tokio::time::sleep(Duration::from_secs(2)).await;
                    }
                }
            }
            A2aResponse::Error { error, .. } => {
                return Err(format!("RPC error: {} - {}", error.code, error.message).into());
            }
        }
    }
}

/// The text of an agent's answer, one line per text part.
fn answer_text(parts: &[arkavo_protocol::types::MessagePart]) -> String {
    parts
        .iter()
        .filter_map(|part| match part {
            arkavo_protocol::types::MessagePart::Text { content } => Some(content.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answer_text_joins_text_parts_in_order() {
        use arkavo_protocol::types::MessagePart;
        let parts = vec![
            MessagePart::Text {
                content: "first".to_string(),
            },
            MessagePart::Text {
                content: "second".to_string(),
            },
        ];
        assert_eq!(answer_text(&parts), "first\nsecond");
        assert_eq!(answer_text(&[]), "");
    }

    #[test]
    fn test_discover_returns_vec() {
        // Without real mDNS services, should return empty or error gracefully
        let result = discover_mesh_agents();
        assert!(result.is_ok());
    }
}
