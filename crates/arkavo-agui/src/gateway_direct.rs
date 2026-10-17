//! Explicit peers complement LAN discovery when multicast is unavailable.

use crate::agent_connection::{AgentConnection, TelemetryEvent};
use crate::gateway::AppState;
use crate::types::AgUiEvent;
use serde_json::{Value, json};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

pub fn parse_endpoint(value: &str) -> Result<SocketAddr, String> {
    let address: SocketAddr = value
        .parse()
        .map_err(|_| "Agent endpoint must be IP:PORT (IPv6: [IP]:PORT)".to_string())?;
    let ip = address.ip().to_canonical();
    if address.port() == 0
        || ip.is_unspecified()
        || ip.is_multicast()
        || matches!(ip, std::net::IpAddr::V4(ip) if ip.is_broadcast())
    {
        return Err("Agent endpoint must name a unicast address and nonzero port".into());
    }
    Ok(address)
}

fn agent_record(card: &Value, endpoint: SocketAddr) -> Result<Value, String> {
    let name = card["name"]
        .as_str()
        .filter(|name| !name.trim().is_empty() && name.len() <= 256)
        .ok_or("Agent card must include a nonempty name of at most 256 bytes")?;
    // A peer's card describes it; it cannot redirect the operator's connection.
    Ok(json!({
        "id": name,
        "name": name,
        "purpose": card["description"].as_str().unwrap_or_default(),
        "endpoint": endpoint.to_string(),
        "discovery": "direct",
    }))
}

async fn read_card(
    endpoint: SocketAddr,
) -> Result<Value, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .build()?;
    let mut response = client
        .get(format!("http://{endpoint}/.well-known/agent.json"))
        .send()
        .await?
        .error_for_status()?;
    if !response.status().is_success() {
        return Err("Agent card redirects are not followed".into());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if body.len() + chunk.len() > 65536 {
            return Err("Agent card exceeds 64 KiB".into());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&body)?)
}

pub async fn connect(
    state: &AppState,
    telemetry_tx: mpsc::Sender<TelemetryEvent>,
    endpoint: SocketAddr,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let record = agent_record(&read_card(endpoint).await?, endpoint)?;
    let id = record["id"]
        .as_str()
        .ok_or("Missing agent identity")?
        .to_owned();
    let connection = Arc::new(AgentConnection::new(
        id.clone(),
        endpoint.to_string(),
        telemetry_tx,
    ));
    connection.connect().await?;

    // Reserve the name before subscribing; never replace a different peer.
    let mut agents = state.agents.write().await;
    if let Some(existing) = agents.iter().find(|agent| agent["id"] == id) {
        if existing["endpoint"] != endpoint.to_string() {
            return Err(
                format!("Agent name {id:?} already belongs to a different endpoint").into(),
            );
        }
    } else {
        agents.push(record);
    }
    state
        .agent_connections
        .write()
        .await
        .insert(id.clone(), connection.clone());
    drop(agents);

    let (tx, mut rx) = mpsc::channel::<AgUiEvent>(32);
    match connection
        .subscribe_metrics(tx, state.security_handler.clone())
        .await
    {
        Ok(()) => {
            let browsers = state.connections.clone();
            let topology = state.context_topology_cache.clone();
            tokio::spawn(async move {
                while let Some(event) = rx.recv().await {
                    if let AgUiEvent::TelemetryEvent {
                        event_type,
                        details,
                        ..
                    } = &event
                        && event_type == "context_topology"
                    {
                        topology.write().await.insert(id.clone(), details.clone());
                    }
                    for browser in browsers.read().await.values() {
                        let _ = browser._ws_tx.send(event.clone()).await;
                    }
                }
            });
        }
        Err(error) => {
            tracing::warn!(%endpoint, %error, "Agent metrics subscription unavailable; polling remains active")
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_endpoint_accepts_ipv4_and_ipv6_without_wildcards_or_urls() {
        for value in ["127.0.0.1:8342", "10.0.0.2:8342", "[::1]:8342"] {
            assert!(parse_endpoint(value).is_ok(), "{value}");
        }
        for value in [
            "0.0.0.0:8342",
            "[::]:8342",
            "224.0.0.251:5353",
            "[ff02::fb]:5353",
            "127.0.0.1:0",
            "http://127.0.0.1:8342",
            "example.com:8342",
            "[::ffff:0.0.0.0]:8342",
            "255.255.255.255:8342",
            "[::ffff:255.255.255.255]:8342",
        ] {
            assert!(parse_endpoint(value).is_err(), "{value}");
        }
    }

    #[test]
    fn card_cannot_redirect_a_direct_peer_or_invent_model_metadata() {
        let endpoint = "127.0.0.1:8342".parse().unwrap();
        let record = agent_record(&json!({"name": "triage", "description": "Read-only triage", "url": "http://untrusted.invalid"}), endpoint).unwrap();
        assert_eq!(record["id"], "triage");
        assert_eq!(record["endpoint"], "127.0.0.1:8342");
        assert_eq!(record["discovery"], "direct");
        assert!(record.get("model").is_none());
    }

    #[test]
    fn malformed_card_cannot_register_an_unnamed_peer() {
        let endpoint = "127.0.0.1:8342".parse().unwrap();
        for card in [
            json!({}),
            json!({"name": " "}),
            json!({"name": 5}),
            json!({"name": "x".repeat(257)}),
        ] {
            assert!(agent_record(&card, endpoint).is_err());
        }
    }
}
