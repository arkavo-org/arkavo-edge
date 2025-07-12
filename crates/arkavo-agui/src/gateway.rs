use crate::agent_connection::{AgentConnection, TelemetryEvent};
use crate::types::*;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};
use warp::Filter;

/// Connection information for active WebSocket clients
struct ConnectionInfo {
    _ws_tx: mpsc::Sender<AgUiEvent>,
    _agent_id: Option<String>,
    subscriptions: Vec<SubscriptionHandle>,
}

pub struct AgUiGateway {
    port: u16,
    discovered_agents: Arc<RwLock<Vec<serde_json::Value>>>,
    connections: Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    telemetry_tx: mpsc::Sender<TelemetryEvent>,
    telemetry_rx: Option<mpsc::Receiver<TelemetryEvent>>,
}

impl AgUiGateway {
    pub fn new(port: u16) -> Self {
        let (telemetry_tx, telemetry_rx) = mpsc::channel(1000);
        Self {
            port,
            discovered_agents: Arc::new(RwLock::new(Vec::new())),
            connections: Arc::new(RwLock::new(HashMap::new())),
            agent_connections: Arc::new(RwLock::new(HashMap::new())),
            telemetry_tx,
            telemetry_rx: Some(telemetry_rx),
        }
    }

    pub async fn start(mut self) -> Result<(), Box<dyn std::error::Error>> {
        let discovered_agents = self.discovered_agents.clone();
        let agents_clone = discovered_agents.clone();

        // Start mDNS discovery in background
        let agent_connections_for_mdns = self.agent_connections.clone();
        let telemetry_tx_for_mdns = self.telemetry_tx.clone();
        tokio::spawn(async move {
            println!("AG-UI: Starting mDNS discovery...");
            match run_mdns_discovery(
                agents_clone,
                agent_connections_for_mdns,
                telemetry_tx_for_mdns,
            )
            .await
            {
                Ok(_) => println!("AG-UI: mDNS discovery completed"),
                Err(e) => eprintln!("AG-UI: mDNS discovery error: {e}"),
            }
        });

        // Serve static files
        let index_file = warp::get()
            .and(warp::path::end())
            .map(|| warp::reply::html(include_str!("../static/dashboard.html")));

        let chat_ui = warp::path("chat")
            .and(warp::get())
            .map(|| warp::reply::html(include_str!("../static/index-agui.html")));

        // WebSocket endpoint for AG-UI protocol
        let connections = self.connections.clone();
        let agents_for_ws = discovered_agents.clone();
        let agent_connections_for_ws = self.agent_connections.clone();

        let websocket = warp::path("ws")
            .and(warp::ws())
            .and(warp::any().map(move || connections.clone()))
            .and(warp::any().map(move || agents_for_ws.clone()))
            .and(warp::any().map(move || agent_connections_for_ws.clone()))
            .map(|ws: warp::ws::Ws, connections, agents, agent_connections| {
                ws.on_upgrade(move |socket| {
                    handle_websocket(socket, connections, agents, agent_connections)
                })
            });

        // SSE endpoint for legacy agent discovery (to be deprecated)
        let agents_for_sse = discovered_agents.clone();
        let agent_events = warp::path("events")
            .and(warp::get())
            .and(warp::any().map(move || agents_for_sse.clone()))
            .map(|agents: Arc<RwLock<Vec<serde_json::Value>>>| {
                use futures::stream;
                use warp::sse::Event;

                let event_stream = stream::unfold(agents, |agents| async move {
                    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

                    let agents_list = agents.read().await.clone();
                    let response = serde_json::json!({
                        "agents": agents_list
                    });

                    let event = Event::default()
                        .event("discovery")
                        .data(response.to_string());

                    Some((Ok::<_, warp::Error>(event), agents))
                });

                warp::sse::reply(event_stream)
            });

        // Agent proxy endpoint for JSON-RPC calls
        let agents_for_proxy = discovered_agents.clone();
        let agent_proxy = warp::path!("agent" / String)
            .and(warp::post())
            .and(warp::body::json())
            .and(warp::any().map(move || agents_for_proxy.clone()))
            .and_then(handle_agent_proxy);

        // Telemetry WebSocket endpoint
        let telemetry_rx = self
            .telemetry_rx
            .take()
            .expect("telemetry_rx already taken");
        let telemetry_broadcast = Arc::new(RwLock::new(telemetry_rx));
        let telemetry_for_ws = telemetry_broadcast.clone();

        let telemetry_ws = warp::path("telemetry")
            .and(warp::ws())
            .and(warp::any().map(move || telemetry_for_ws.clone()))
            .map(|ws: warp::ws::Ws, telemetry| {
                ws.on_upgrade(move |socket| handle_telemetry_websocket(socket, telemetry))
            });

        let routes = index_file
            .or(chat_ui)
            .or(websocket)
            .or(agent_events)
            .or(agent_proxy)
            .or(telemetry_ws);

        let addr: SocketAddr = ([0, 0, 0, 0], self.port).into();
        println!("Starting AG-UI Gateway on http://127.0.0.1:{}", self.port);
        println!("WebSocket endpoint: ws://127.0.0.1:{}/ws", self.port);
        println!("Open http://127.0.0.1:{} in your web browser", self.port);

        warp::serve(routes).run(addr).await;

        Ok(())
    }
}

async fn handle_websocket(
    ws: warp::ws::WebSocket,
    connections: Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
) {
    use futures::StreamExt;

    let (ws_sink, mut ws_stream) = ws.split();
    let session_id = uuid::Uuid::new_v4().to_string();

    // Create bounded channel for back-pressure handling
    let (tx, mut rx) = mpsc::channel::<AgUiEvent>(32);

    // Spawn task to forward messages from channel to WebSocket
    let forward_task = tokio::spawn(async move {
        use futures::SinkExt;
        let mut ws_tx = ws_sink;

        while let Some(event) = rx.recv().await {
            if let Ok(json) = serde_json::to_string(&event) {
                if ws_tx.send(warp::ws::Message::text(json)).await.is_err() {
                    break; // WebSocket closed
                }
            }
        }
    });

    println!("AG-UI: New WebSocket connection: {session_id}");

    // Handle incoming messages
    while let Some(result) = ws_stream.next().await {
        match result {
            Ok(msg) => {
                if let Ok(text) = msg.to_str() {
                    match serde_json::from_str::<AgUiEvent>(text) {
                        Ok(event) => {
                            if let Err(e) = handle_event(
                                event,
                                &session_id,
                                &connections,
                                &agents,
                                &agent_connections,
                                &tx,
                            )
                            .await
                            {
                                eprintln!("AG-UI: Error handling event: {e}");
                            }
                        }
                        Err(e) => {
                            eprintln!("AG-UI: Failed to parse event: {e}");
                            let error = AgUiEvent::Error {
                                code: "INVALID_EVENT".to_string(),
                                message: format!("Failed to parse event: {e}"),
                            };
                            let _ = tx.send(error).await;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("AG-UI: WebSocket error: {e}");
                break;
            }
        }
    }

    // Clean up connection and subscriptions
    if let Some(mut conn_info) = connections.write().await.remove(&session_id) {
        // Cancel all subscriptions
        for mut sub in conn_info.subscriptions.drain(..) {
            sub.cancel();
        }
    }

    // Cancel the forward task
    forward_task.abort();

    println!("AG-UI: WebSocket connection closed: {session_id}");
}

async fn handle_event(
    event: AgUiEvent,
    session_id: &str,
    connections: &Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    agents: &Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    tx: &mpsc::Sender<AgUiEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    match event {
        AgUiEvent::Connect {
            agent_id,
            agui_version,
            since_event_id: _,
        } => {
            println!("AG-UI: Connect request for agent {agent_id} (version {agui_version})");

            // Find the agent
            let agents_list = agents.read().await;
            let agent = agents_list
                .iter()
                .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(&agent_id))
                .cloned();

            if let Some(_agent_info) = agent {
                // Store connection info
                let conn_info = ConnectionInfo {
                    _ws_tx: tx.clone(),
                    _agent_id: Some(agent_id.clone()),
                    subscriptions: Vec::new(),
                };

                connections
                    .write()
                    .await
                    .insert(session_id.to_string(), conn_info);

                // Send initial snapshots
                let state_snapshot = AgUiEvent::StateSnapshot {
                    state: serde_json::json!({}),
                    event_id: uuid::Uuid::new_v4().to_string(),
                };

                let messages_snapshot = AgUiEvent::MessagesSnapshot {
                    messages: vec![],
                    event_id: uuid::Uuid::new_v4().to_string(),
                };

                tx.send(state_snapshot).await?;
                tx.send(messages_snapshot).await?;

                // Send lifecycle start
                let lifecycle_start = AgUiEvent::LifecycleStart {
                    session_id: session_id.to_string(),
                };

                tx.send(lifecycle_start).await?;
            } else {
                let error = AgUiEvent::Error {
                    code: "AGENT_NOT_FOUND".to_string(),
                    message: format!("Agent {agent_id} not found"),
                };
                tx.send(error).await?;
            }
        }

        AgUiEvent::ChatOpen { agent_id } => {
            // Get the agent connection
            let agent_conns = agent_connections.read().await;
            if let Some(agent_conn) = agent_conns.get(&agent_id) {
                // Subscribe to chat for this agent
                match agent_conn
                    .subscribe_to_chat(agent_id.clone(), tx.clone())
                    .await
                {
                    Ok(sub_handle) => {
                        // Store subscription handle
                        let mut conn_guard = connections.write().await;
                        if let Some(conn_info) = conn_guard.get_mut(session_id) {
                            conn_info.subscriptions.push(sub_handle);
                        }
                    }
                    Err(e) => {
                        let error = AgUiEvent::Error {
                            code: "SUBSCRIPTION_FAILED".to_string(),
                            message: format!("Failed to subscribe to chat: {e}"),
                        };
                        tx.send(error).await?;
                    }
                }
            } else {
                let error = AgUiEvent::Error {
                    code: "AGENT_NOT_CONNECTED".to_string(),
                    message: format!("Agent {agent_id} is not connected"),
                };
                tx.send(error).await?;
            }
        }

        AgUiEvent::ChatClose { agent_id } => {
            // Get the agent connection
            let agent_conns = agent_connections.read().await;
            if let Some(agent_conn) = agent_conns.get(&agent_id) {
                // Unsubscribe from chat
                if let Err(e) = agent_conn.unsubscribe_chat(&agent_id).await {
                    eprintln!("Failed to unsubscribe from chat: {e}");
                }
            }
        }

        AgUiEvent::UserMessage {
            agent_id,
            content,
            attachments: _,
        } => {
            // Get the agent connection
            let agent_conns = agent_connections.read().await;
            if let Some(agent_conn) = agent_conns.get(&agent_id) {
                // Send the user message
                match agent_conn.send_user_message(&agent_id, content).await {
                    Err(e) => {
                        let error = AgUiEvent::Error {
                            code: "MESSAGE_SEND_FAILED".to_string(),
                            message: format!("Failed to send message: {e}"),
                        };
                        tx.send(error).await?;
                    }
                    Ok(_) => {
                        // Message sent successfully
                    }
                }
            } else {
                let error = AgUiEvent::Error {
                    code: "AGENT_NOT_CONNECTED".to_string(),
                    message: format!("Agent {agent_id} is not connected"),
                };
                tx.send(error).await?;
            }
        }

        _ => {
            println!("AG-UI: Received event: {event:?}");
        }
    }

    Ok(())
}

async fn handle_agent_proxy(
    agent_id: String,
    body: serde_json::Value,
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
) -> Result<impl warp::Reply, warp::Rejection> {
    // Find the agent
    let agents_list = agents.read().await;
    let agent = agents_list
        .iter()
        .find(|a| a.get("id").and_then(|v| v.as_str()) == Some(&agent_id));

    if let Some(agent_info) = agent {
        if let Some(endpoint) = agent_info.get("endpoint").and_then(|v| v.as_str()) {
            // Forward the request to the agent
            let request_id = body.get("id").cloned();
            match forward_to_agent(endpoint, body).await {
                Ok(response) => Ok(warp::reply::json(&response)),
                Err(e) => {
                    let error_response = serde_json::json!({
                        "jsonrpc": "2.0",
                        "error": {
                            "code": -32603,
                            "message": format!("Failed to forward request: {}", e)
                        },
                        "id": request_id
                    });
                    Ok(warp::reply::json(&error_response))
                }
            }
        } else {
            let error_response = serde_json::json!({
                "jsonrpc": "2.0",
                "error": {
                    "code": -32602,
                    "message": "Agent endpoint not found"
                },
                "id": body.get("id").cloned()
            });
            Ok(warp::reply::json(&error_response))
        }
    } else {
        let error_response = serde_json::json!({
            "jsonrpc": "2.0",
            "error": {
                "code": -32602,
                "message": format!("Agent {agent_id} not found")
            },
            "id": body.get("id").cloned()
        });
        Ok(warp::reply::json(&error_response))
    }
}

async fn forward_to_agent(
    endpoint: &str,
    body: serde_json::Value,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let client = reqwest::Client::new();
    let url = format!("http://{endpoint}");

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    let json_response = response.json::<serde_json::Value>().await?;
    Ok(json_response)
}

async fn handle_telemetry_websocket(
    ws: warp::ws::WebSocket,
    telemetry_rx: Arc<RwLock<mpsc::Receiver<TelemetryEvent>>>,
) {
    use futures::{SinkExt, StreamExt};

    let (mut ws_tx, _ws_rx) = ws.split();

    println!("New telemetry WebSocket connection");

    // Take ownership of the receiver
    let mut rx = telemetry_rx.write().await;

    // Forward telemetry events to WebSocket
    while let Some(event) = rx.recv().await {
        if let Ok(json) = serde_json::to_string(&event) {
            if ws_tx.send(warp::ws::Message::text(json)).await.is_err() {
                break;
            }
        }
    }

    println!("Telemetry WebSocket connection closed");
}

async fn run_mdns_discovery(
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    telemetry_tx: mpsc::Sender<TelemetryEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    // This is the same mDNS discovery code from ui.rs
    // We'll reuse it here for now
    #[cfg(feature = "mdns")]
    {
        use std::sync::mpsc;
        use std::time::Duration;

        println!("Starting mDNS discovery service...");

        let (tx, rx) = mpsc::channel::<serde_json::Value>();

        std::thread::spawn(move || {
            use zeroconf::{MdnsBrowser, ServiceType, prelude::*};

            let service_type = ServiceType::new("a2a", "tcp").unwrap();
            let mut browser = MdnsBrowser::new(service_type);

            let tx_clone = tx;
            browser.set_service_discovered_callback(Box::new(move |result, _| match result {
                Ok(service) => {
                    let host = service.address().to_string();

                    println!(
                        "AG-UI: Discovered service: {} at {}:{}",
                        service.name(),
                        host,
                        service.port()
                    );

                    let mut agent_id = service.name().to_string();
                    if agent_id.starts_with("arkavo-agent-") {
                        agent_id = agent_id.trim_start_matches("arkavo-agent-").to_string();
                    }

                    let mut purpose = "Agent discovered via mDNS".to_string();
                    let mut model = "Unknown".to_string();

                    if let Some(txt) = service.txt() {
                        for (key, value) in txt.iter() {
                            match key.as_str() {
                                "agent_id" => agent_id = value.clone(),
                                "purpose" => purpose = value.clone(),
                                "model" => model = value.clone(),
                                _ => {}
                            }
                        }
                    }

                    let agent_info = serde_json::json!({
                        "id": agent_id,
                        "name": agent_id,
                        "purpose": purpose,
                        "model": model,
                        "endpoint": format!("{}:{}", host, service.port())
                    });

                    let _ = tx_clone.send(agent_info);
                }
                Err(e) => {
                    eprintln!("AG-UI: Error discovering service: {e}");
                }
            }));

            match browser.browse_services() {
                Ok(event_loop) => {
                    println!("mDNS browser started, looking for _a2a._tcp services");
                    let _ = event_loop.poll(Duration::from_secs(3600));
                }
                Err(e) => {
                    eprintln!("Failed to start mDNS browser: {e}");
                }
            }
        });

        let mut discovered_agents = std::collections::HashMap::new();

        loop {
            while let Ok(agent_info) = rx.try_recv() {
                if let Some(id) = agent_info.get("id").and_then(|v| v.as_str()) {
                    let id_string = id.to_string();

                    // Check if this is a new agent
                    if !discovered_agents.contains_key(&id_string) {
                        // Create persistent connection to the agent
                        if let Some(endpoint) = agent_info.get("endpoint").and_then(|v| v.as_str())
                        {
                            let agent_conn = Arc::new(AgentConnection::new(
                                id_string.clone(),
                                endpoint.to_string(),
                                telemetry_tx.clone(),
                            ));

                            // Start the connection in a background task
                            let agent_conn_clone = agent_conn.clone();
                            tokio::spawn(async move {
                                agent_conn_clone.start().await;
                            });

                            // Store the connection
                            agent_connections
                                .write()
                                .await
                                .insert(id_string.clone(), agent_conn);

                            println!("AG-UI: Created persistent connection to agent {id}");
                        }
                    }

                    discovered_agents.insert(id_string, agent_info);
                }
            }

            let mut agents_list = agents.write().await;
            agents_list.clear();
            agents_list.extend(discovered_agents.values().cloned());

            drop(agents_list);

            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }
    }

    #[cfg(not(feature = "mdns"))]
    {
        println!("mDNS discovery not compiled in");
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }
    }
}
