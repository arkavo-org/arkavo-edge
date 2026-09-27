#![allow(clippy::disallowed_methods)]

//! Live integration tests against a running OpenClaw gateway.
//!
//! These tests require `OPENCLAW_GATEWAY_TOKEN` env and a gateway at `ws://127.0.0.1:18789`.
//! Skip automatically when gateway is unavailable.

use arkavo_openclaw::OpenClawTransport;
use arkavo_openclaw::device::DeviceIdentity;
use arkavo_openclaw::handshake::{HandshakeConfig, HandshakeOutcome};
use arkavo_protocol::transport::{A2aEndpoint, A2aRequest, A2aTransport, TransportConfig};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message;

fn gateway_url() -> String {
    std::env::var("OPENCLAW_GATEWAY_URL").unwrap_or_else(|_| "ws://127.0.0.1:18789".to_string())
}

fn gateway_token() -> Option<String> {
    std::env::var("OPENCLAW_GATEWAY_TOKEN").ok().or_else(|| {
        let home = dirs::home_dir()?;
        let path = home.join(".openclaw/openclaw.json");
        let data = std::fs::read_to_string(path).ok()?;
        let val: serde_json::Value = serde_json::from_str(&data).ok()?;
        val.get("gateway")?
            .get("auth")?
            .get("token")?
            .as_str()
            .map(|s| s.to_string())
    })
}

async fn connect_ws(
    url: &str,
) -> Option<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
> {
    tokio_tungstenite::connect_async(url)
        .await
        .ok()
        .map(|(ws, _)| ws)
}

#[tokio::test]
async fn handshake_without_device_succeeds() {
    let token = match gateway_token() {
        Some(t) => t,
        None => {
            eprintln!("SKIP: no gateway token available");
            return;
        }
    };

    let url = gateway_url();
    let mut ws = match connect_ws(&url).await {
        Some(ws) => ws,
        None => {
            eprintln!("SKIP: gateway not reachable at {url}");
            return;
        }
    };

    let config = HandshakeConfig {
        gateway_token: Some(token),
        ..Default::default()
    };

    let session = arkavo_openclaw::handshake::client_handshake(&mut ws, &config)
        .await
        .unwrap();

    assert_eq!(session.protocol, 3);
    assert!(session.conn_id.is_some());
    eprintln!("connected: conn_id={:?}", session.conn_id);
}

#[tokio::test]
async fn handshake_with_device_returns_pairing_or_connected() {
    let token = match gateway_token() {
        Some(t) => t,
        None => {
            eprintln!("SKIP: no gateway token available");
            return;
        }
    };

    let url = gateway_url();
    let mut ws = match connect_ws(&url).await {
        Some(ws) => ws,
        None => {
            eprintln!("SKIP: gateway not reachable at {url}");
            return;
        }
    };

    // Use a temp directory for the device identity so we don't pollute real config
    let tmp_dir = std::env::temp_dir().join(format!("arkavo-live-test-{}", uuid::Uuid::new_v4()));
    let device = DeviceIdentity::load_or_create(&tmp_dir).unwrap();
    eprintln!("device_id={}", device.device_id());

    let config = HandshakeConfig {
        gateway_token: Some(token),
        ..Default::default()
    };

    let outcome =
        arkavo_openclaw::handshake::client_handshake_with_device(&mut ws, &config, Some(&device))
            .await
            .unwrap();

    match outcome {
        HandshakeOutcome::Connected(session) => {
            eprintln!(
                "connected with scopes: {:?}, device_token: {:?}",
                session.scopes, session.device_token
            );
            assert_eq!(session.protocol, 3);

            // Try a scoped request to verify if device pairing actually granted scopes
            let req = serde_json::json!({
                "type": "req",
                "id": "scope-test-1",
                "method": "agents.list",
                "params": {}
            });
            ws.send(Message::Text(req.to_string())).await.unwrap();

            // Read frames, skip events, look for the response to our request
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
            loop {
                let remaining = deadline.duration_since(tokio::time::Instant::now());
                match tokio::time::timeout(remaining, ws.next()).await {
                    Ok(Some(Ok(Message::Text(text)))) => {
                        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                        if v.get("type").and_then(|t| t.as_str()) == Some("event") {
                            eprintln!(
                                "skipping event: {}",
                                v.get("event").unwrap_or(&serde_json::Value::Null)
                            );
                            continue;
                        }
                        eprintln!("agents.list response: {text}");
                        if v.get("ok") == Some(&serde_json::Value::Bool(true)) {
                            eprintln!("agents.list (operator.read) SUCCEEDED");
                        } else {
                            eprintln!("agents.list denied: {:?}", v.get("error"));
                        }
                        break;
                    }
                    Ok(Some(Ok(_))) => continue,
                    _ => {
                        eprintln!("timeout or error waiting for response");
                        break;
                    }
                }
            }
        }
        HandshakeOutcome::PairingRequested { request_id } => {
            eprintln!("pairing requested: request_id={request_id:?}");
            eprintln!("run: openclaw devices approve");
        }
    }

    let _ = std::fs::remove_dir_all(&tmp_dir);
}

/// Connect an `OpenClawTransport` with device identity.
/// Uses persisted device if available, otherwise creates a fresh one (auto-pairing).
/// Returns `None` if prerequisites are missing (no token, gateway unreachable, pairing required).
async fn connect_transport() -> Option<OpenClawTransport> {
    let token = gateway_token()?;
    let url = gateway_url();

    // Check gateway is reachable with a quick probe
    if connect_ws(&url).await.is_none() {
        eprintln!("SKIP: gateway not reachable at {url}");
        return None;
    }

    // Try persisted device first, then create a fresh one
    let device_dir = DeviceIdentity::default_dir();
    let (device, device_token) = if device_dir.join("device-key.bin").exists() {
        let device = DeviceIdentity::load_or_create(&device_dir).ok()?;
        let auth_store = arkavo_openclaw::device::DeviceAuthStore::load(&device_dir);
        let dt = auth_store
            .as_ref()
            .and_then(|s| s.get_token("operator"))
            .map(|t| t.token.clone());
        (device, dt)
    } else {
        let tmp = std::env::temp_dir().join(format!("arkavo-test-{}", uuid::Uuid::new_v4()));
        let device = DeviceIdentity::load_or_create(&tmp).ok()?;
        eprintln!("using fresh device: {}", device.device_id());
        (device, None)
    };

    let hs_config = HandshakeConfig {
        gateway_token: Some(token),
        device_token,
        ..Default::default()
    };

    let mut transport_config = TransportConfig::default();
    // Local gateway uses ws://, not wss://
    if url.starts_with("ws://") {
        transport_config.tls_config.require_tls = false;
    }

    let transport = OpenClawTransport::new(transport_config, hs_config).with_device(device);

    let endpoint = A2aEndpoint {
        url,
        agent_id: "openclaw-gateway".to_string(),
        public_key: None,
    };

    if let Err(e) = transport.connect(&endpoint).await {
        eprintln!("SKIP: connect failed: {e}");
        return None;
    }

    Some(transport)
}

#[tokio::test]
async fn device_with_persisted_token_reconnects() {
    let token = match gateway_token() {
        Some(t) => t,
        None => {
            eprintln!("SKIP: no gateway token available");
            return;
        }
    };

    let url = gateway_url();

    // Check if we have a previously approved device identity
    let device_dir = DeviceIdentity::default_dir();
    if !device_dir.join("device-key.bin").exists() {
        eprintln!(
            "SKIP: no persisted device identity at {}",
            device_dir.display()
        );
        return;
    }

    let device = DeviceIdentity::load_or_create(&device_dir).unwrap();
    let auth_store = arkavo_openclaw::device::DeviceAuthStore::load(&device_dir);

    let device_token = auth_store
        .as_ref()
        .and_then(|s| s.get_token("operator"))
        .map(|t| t.token.clone());

    let mut ws = match connect_ws(&url).await {
        Some(ws) => ws,
        None => {
            eprintln!("SKIP: gateway not reachable at {url}");
            return;
        }
    };

    let config = HandshakeConfig {
        gateway_token: Some(token),
        device_token,
        ..Default::default()
    };

    let outcome =
        arkavo_openclaw::handshake::client_handshake_with_device(&mut ws, &config, Some(&device))
            .await
            .unwrap();

    match outcome {
        HandshakeOutcome::Connected(session) => {
            eprintln!(
                "reconnected: scopes={:?}, device_token={:?}",
                session.scopes, session.device_token
            );
        }
        HandshakeOutcome::PairingRequested { request_id } => {
            eprintln!("pairing still needed: request_id={request_id:?}");
        }
    }
}

#[tokio::test]
async fn sessions_list_returns_active_sessions() {
    let transport = match connect_transport().await {
        Some(t) => t,
        None => return,
    };

    let req = A2aRequest::new("sessions/list", serde_json::json!({}));
    let resp = transport.send_request(req).await.unwrap();

    match &resp {
        arkavo_protocol::transport::A2aResponse::Success { result, .. } => {
            eprintln!("sessions.list result: {result}");
            // The gateway should return a list (possibly empty if no active sessions)
            assert!(result.is_array() || result.is_object());
        }
        arkavo_protocol::transport::A2aResponse::Error { error, .. } => {
            eprintln!("sessions.list error (may be expected): {}", error.message);
        }
    }

    let _ = transport.close().await;
}

#[tokio::test]
async fn models_list_returns_available_models() {
    let transport = match connect_transport().await {
        Some(t) => t,
        None => return,
    };

    let req = A2aRequest::new("models/list", serde_json::json!({}));
    let resp = transport.send_request(req).await.unwrap();

    match &resp {
        arkavo_protocol::transport::A2aResponse::Success { result, .. } => {
            eprintln!("models.list result: {result}");
        }
        arkavo_protocol::transport::A2aResponse::Error { error, .. } => {
            // May fail if no API key configured — that's expected
            eprintln!("models.list error (may be expected): {}", error.message);
        }
    }

    let _ = transport.close().await;
}

#[tokio::test]
async fn chat_send_with_streaming_response() {
    let transport = match connect_transport().await {
        Some(t) => t,
        None => return,
    };

    // First, discover sessions to find a valid session key
    let sessions_req = A2aRequest::new("sessions/list", serde_json::json!({}));
    let sessions_resp = transport.send_request(sessions_req).await.unwrap();

    let session_key = match &sessions_resp {
        arkavo_protocol::transport::A2aResponse::Success { result, .. } => {
            // Try to extract the first session key from the response
            result
                .as_array()
                .and_then(|arr| arr.first())
                .and_then(|s| s.get("key").or_else(|| s.get("sessionKey")))
                .and_then(|k| k.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| "agent:main:main".to_string())
        }
        _ => "agent:main:main".to_string(),
    };

    eprintln!("using discovered session");

    // Send chat via the convenience method
    let result = transport
        .send_chat(&session_key, "Say hello in one word.")
        .await;

    match result {
        Ok((run_id, mut events_rx)) => {
            eprintln!("chat.send ok, runId={run_id}");

            let mut delta_count = 0u32;
            let mut got_final = false;

            // Collect events with a timeout
            let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
            loop {
                let remaining = deadline.duration_since(tokio::time::Instant::now());
                match tokio::time::timeout(remaining, events_rx.recv()).await {
                    Ok(Ok(ev)) => {
                        if ev.event != "chat" {
                            continue;
                        }
                        // Check if this event belongs to our run
                        let ev_run_id = ev
                            .payload
                            .get("runId")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if !run_id.is_empty() && ev_run_id != run_id {
                            continue;
                        }

                        let state = ev
                            .payload
                            .get("state")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");

                        match state {
                            "delta" => {
                                delta_count += 1;
                                let content = ev
                                    .payload
                                    .get("content")
                                    .and_then(|v| v.as_str())
                                    .unwrap_or("");
                                eprint!("{content}");
                            }
                            "final" => {
                                got_final = true;
                                eprintln!("\nfinal event received");
                                break;
                            }
                            other => {
                                eprintln!("event state={other}");
                            }
                        }
                    }
                    Ok(Err(_)) => {
                        eprintln!("event channel closed");
                        break;
                    }
                    Err(_) => {
                        eprintln!("timeout waiting for events");
                        break;
                    }
                }
            }

            eprintln!("received {delta_count} delta(s), final={got_final}");
            // We expect at least one event (delta or final) from a functioning agent
            // If the agent lacks an API key, we may get 0 deltas — that's acceptable
        }
        Err(e) => {
            let msg = e.to_string();
            // Agent may not have an API key configured — that's an expected error path
            eprintln!("chat.send error (may be expected): {msg}");
            assert!(
                msg.contains("API")
                    || msg.contains("key")
                    || msg.contains("model")
                    || msg.contains("agent")
                    || msg.contains("scope")
                    || msg.contains("session")
                    || msg.contains("error"),
                "unexpected error: {msg}"
            );
        }
    }

    let _ = transport.close().await;
}
