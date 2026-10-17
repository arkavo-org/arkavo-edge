use std::time::Duration;

use arkavo_protocol::agent_registry::AgentInfo;

/// How long discovery listens for agents' announcements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listen {
    /// The longest it listens.
    pub window: Duration,
    /// Stop this long after the last agent was heard, once one was; `None`
    /// listens for the whole window.
    pub quiet: Option<Duration>,
}

impl Listen {
    /// Enough to reach an agent by name: stop once announcements settle.
    pub const FIRST: Self = Self {
        window: Duration::from_secs(5),
        quiet: Some(Duration::from_millis(500)),
    };

    /// Every agent on the network, for `arkavo kit apply`. This host's own
    /// agent answers at once, so settling after it misses slower hosts.
    pub const ALL: Self = Self {
        window: Duration::from_secs(5),
        quiet: None,
    };

    fn done(&self, elapsed: Duration, since_last: Duration, found: usize) -> bool {
        elapsed >= self.window || (found > 0 && self.quiet.is_some_and(|q| since_last > q))
    }
}

/// Discover agents on the mesh network using mDNS, stopping once
/// announcements settle.
pub fn discover_mesh_agents() -> Result<Vec<AgentInfo>, Box<dyn std::error::Error>> {
    discover_mesh_agents_with(Listen::FIRST)
}

/// Discover agents on the mesh network using mDNS, listening as `listen`
/// says. Each agent is listed once.
#[cfg(feature = "mdns")]
pub fn discover_mesh_agents_with(
    listen: Listen,
) -> Result<Vec<AgentInfo>, Box<dyn std::error::Error>> {
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    use std::collections::HashMap;
    use tracing::info;

    info!("Discovering mesh agents via mDNS...");

    let mdns = ServiceDaemon::new()?;
    let receiver = mdns.browse("_a2a._tcp.local.")?;

    let mut agents: Vec<AgentInfo> = Vec::new();
    let start = std::time::Instant::now();
    let mut last_discovery = std::time::Instant::now();

    while !listen.done(start.elapsed(), last_discovery.elapsed(), agents.len()) {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(event) => {
                if let ServiceEvent::ServiceResolved(info) = event {
                    last_discovery = std::time::Instant::now();
                    // An agent resolves again on each interface it announces on.
                    if agents.iter().any(|a| a.name == info.get_fullname()) {
                        continue;
                    }
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
pub fn discover_mesh_agents_with(
    _listen: Listen,
) -> Result<Vec<AgentInfo>, Box<dyn std::error::Error>> {
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
    on_progress: &mut (dyn FnMut(&str) + Send),
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

    let mut watch = arkavo_protocol::task_watch::Watch::default();
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
                poll_refused(&watch, &task_id, &error)?;
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    }
}

/// What an error answer to a `tasks/get` poll means: `Ok` to poll again,
/// `Err` with what to tell the user. An agent's rate limiter is shared by
/// every caller, so a burst from elsewhere can turn a poll away while the
/// task runs on; that poll is no change, and polling stops at the CLI-019
/// limit as it would for a task that reports nothing new.
pub(super) fn poll_refused(
    watch: &arkavo_protocol::task_watch::Watch,
    task_id: &str,
    error: &arkavo_protocol::transport::JsonRpcError,
) -> Result<(), String> {
    if arkavo_protocol::rate_limit::is_rate_limited(error.code, &error.message) {
        return watch.unanswered(task_id);
    }
    Err(format!("RPC error: {} - {}", error.code, error.message))
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

    /// SKD-012 regression: `kit apply` listed this Mac's own agent and
    /// stopped before a MacBook's announcement arrived.
    #[arkavo_test_macros::spec("SKD-012")]
    #[test]
    fn finding_every_agent_listens_for_the_whole_window() {
        let ms = Duration::from_millis;
        assert!(!Listen::FIRST.done(ms(100), ms(10), 0));
        assert!(
            Listen::FIRST.done(ms(700), ms(600), 1),
            "settled after the first agent"
        );
        assert!(!Listen::ALL.done(ms(700), ms(600), 1));
        assert!(!Listen::ALL.done(ms(4_900), ms(4_000), 1));
        assert!(Listen::ALL.done(ms(5_000), ms(0), 3));
    }

    /// An agent whose rate limiter turns away its first `refusals` polls
    /// of `tasks/get` before the task completes.
    async fn busy_agent(refusals: usize) -> arkavo_protocol::http::HttpTransport {
        use arkavo_protocol::rate_limit::{RATE_LIMITED, RATE_LIMITED_MESSAGE};
        use arkavo_protocol::transport::{A2aEndpoint, A2aTransport, TlsConfig, TransportConfig};
        use axum::{Json, Router, routing::post};
        use serde_json::{Value, json};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let polls = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route(
            "/rpc",
            post(move |Json(request): Json<Value>| {
                let polls = polls.clone();
                async move {
                    let mut answer = match request["method"].as_str() {
                        Some("message/send") => {
                            json!({"result": {"task_id": "t1", "status": "submitted"}})
                        }
                        _ if polls.fetch_add(1, Ordering::SeqCst) < refusals => json!({
                            "error": {"code": RATE_LIMITED, "message": RATE_LIMITED_MESSAGE}
                        }),
                        _ => json!({"result": {
                            "task_id": "t1",
                            "status": "completed",
                            "result": {"parts": [{"type": "text", "content": "Done."}]}
                        }}),
                    };
                    answer["jsonrpc"] = json!("2.0");
                    answer["id"] = request["id"].clone();
                    Json(answer)
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback");
        let url = format!("http://{}", listener.local_addr().expect("bound"));
        tokio::spawn(async move { axum::serve(listener, app).await });

        let transport = arkavo_protocol::http::HttpTransport::new(TransportConfig {
            tls_config: TlsConfig {
                require_tls: false,
                ..Default::default()
            },
            ..Default::default()
        })
        .expect("transport");
        transport
            .connect(&A2aEndpoint {
                url,
                agent_id: "writer".into(),
                public_key: None,
            })
            .await
            .expect("connects");
        transport
    }

    /// Regression (CLI-019): the agent's rate limiter is shared by every
    /// caller, and one refused poll ended `arkavo chat` while the task ran
    /// on. A refused poll is polled again.
    #[arkavo_test_macros::spec("CLI-019")]
    #[tokio::test]
    async fn a_poll_the_rate_limiter_refuses_is_made_again() {
        let transport = busy_agent(2).await;

        let answer = send_and_poll_agent(&transport, "Make a coin", &mut |_| {})
            .await
            .expect("answered after two refused polls");

        assert_eq!(answer_text(&answer.parts), "Done.");
    }

    /// CLI-019: only a refusal by the rate limiter is polled again; another
    /// error with the same code still ends the poll, and refusals end it
    /// once the task has been quiet for the limit.
    #[arkavo_test_macros::spec("CLI-019")]
    #[test]
    fn only_a_rate_limit_refusal_is_polled_again() {
        use arkavo_protocol::task_watch::Watch;
        use arkavo_protocol::transport::JsonRpcError;
        let error = |code, message: &str| JsonRpcError {
            code,
            message: message.into(),
            data: None,
        };
        let refused = error(-32001, "Rate limit exceeded");

        assert_eq!(poll_refused(&Watch::default(), "t1", &refused), Ok(()));
        let other = poll_refused(&Watch::default(), "t1", &error(-32001, "Session not found"));
        assert_eq!(other, Err("RPC error: -32001 - Session not found".into()));
        let quiet = poll_refused(&Watch::with_limit(Duration::ZERO), "t1", &refused);
        assert!(quiet.unwrap_err().contains("still running"));
    }

    #[test]
    fn test_discover_returns_vec() {
        // Without real mDNS services, should return empty or error gracefully
        let result = discover_mesh_agents();
        assert!(result.is_ok());
    }
}
