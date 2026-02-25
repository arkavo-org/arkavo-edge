//! Live integration tests against a running OpenClaw gateway.
//!
//! These tests require `OPENCLAW_GATEWAY_TOKEN` env and a gateway at `ws://127.0.0.1:18789`.
//! Skip automatically when gateway is unavailable.

use arkavo_openclaw::device::DeviceIdentity;
use arkavo_openclaw::handshake::{HandshakeConfig, HandshakeOutcome};
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
