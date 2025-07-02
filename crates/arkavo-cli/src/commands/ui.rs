use std::net::SocketAddr;
use std::sync::Arc;
use tokio::sync::RwLock;

pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.len() > 0 && matches!(args[0].as_str(), "help" | "-h" | "--help") {
        print_usage();
        return Ok(());
    }

    // Parse optional port argument
    let port = if args.len() > 0 {
        args[0].parse::<u16>().unwrap_or(7700)
    } else {
        7700
    };

    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(async { start_ui_server(port).await })
}

fn print_usage() {
    println!("Arkavo UI - Web interface for agent orchestration");
    println!();
    println!("USAGE:");
    println!("    arkavo ui [PORT]");
    println!();
    println!("OPTIONS:");
    println!("    PORT    Port to run the UI server on (default: 7700)");
    println!();
    println!("EXAMPLES:");
    println!("    arkavo ui          # Start UI on default port 7700");
    println!("    arkavo ui 8080     # Start UI on port 8080");
}

async fn start_ui_server(port: u16) -> Result<(), Box<dyn std::error::Error>> {
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use warp::Filter;

    // Shared state for discovered agents
    let discovered_agents = Arc::new(RwLock::new(Vec::new()));
    let agents_clone = discovered_agents.clone();

    // Start mDNS discovery in background
    tokio::spawn(async move {
        println!("UI: Spawning mDNS discovery task...");
        match run_mdns_discovery(agents_clone).await {
            Ok(_) => println!("UI: mDNS discovery task completed"),
            Err(e) => eprintln!("UI: mDNS discovery error: {}", e),
        }
    });

    // Serve static files
    let static_files = warp::get()
        .and(warp::path::end())
        .map(|| warp::reply::html(include_str!("../../static/index.html")));

    // SSE endpoint for agent discovery
    let agent_events = warp::path("events")
        .and(warp::get())
        .and(warp::any().map(move || discovered_agents.clone()))
        .map(|agents: Arc<RwLock<Vec<serde_json::Value>>>| {
            use futures::stream;
            use warp::sse::Event;

            let event_stream = stream::unfold(agents, |agents| async move {
                // Poll for discovered agents
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

    let routes = static_files.or(agent_events);

    let addr: SocketAddr = ([0, 0, 0, 0], port).into();
    println!("Starting Arkavo UI server on http://127.0.0.1:{}", port);
    println!("Open this URL in your web browser");
    println!("Press Ctrl+C to stop");

    warp::serve(routes).run(addr).await;

    Ok(())
}

async fn run_mdns_discovery(
    _agents: Arc<RwLock<Vec<serde_json::Value>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "mdns")]
    let agents = _agents;
    #[cfg(feature = "mdns")]
    {
        use std::sync::mpsc;
        use std::time::Duration;

        println!("Starting mDNS discovery service...");

        // Create a channel to communicate between threads
        let (tx, rx) = mpsc::channel::<serde_json::Value>();

        // Run zeroconf in a separate thread since it's not Send
        std::thread::spawn(move || {
            use zeroconf::{MdnsBrowser, ServiceType, prelude::*};

            let service_type = ServiceType::new("a2a", "tcp").unwrap();
            let mut browser = MdnsBrowser::new(service_type);

            let tx_clone = tx.clone();
            browser.set_service_discovered_callback(Box::new(move |result, _| {
                match result {
                    Ok(service) => {
                        let host = service.address().to_string();

                        println!(
                            "UI: Discovered service: {} at {}:{}",
                            service.name(),
                            host,
                            service.port()
                        );

                        // Extract agent info from TXT record
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
                            "id": agent_id.clone(),
                            "name": agent_id.clone(),
                            "purpose": purpose,
                            "model": model,
                            "endpoint": format!("{}:{}", host, service.port())
                        });

                        // Send to the channel
                        let _ = tx_clone.send(agent_info);
                    }
                    Err(e) => {
                        eprintln!("UI: Error discovering service: {}", e);
                    }
                }
            }));

            match browser.browse_services() {
                Ok(event_loop) => {
                    println!("mDNS browser started, looking for _a2a._tcp services");
                    let _ = event_loop.poll(Duration::from_secs(3600)); // Poll for 1 hour
                }
                Err(e) => {
                    eprintln!("Failed to start mDNS browser: {}", e);
                }
            }
        });

        // Process discovered services
        let mut discovered_agents = std::collections::HashMap::new();

        loop {
            // Check for new discoveries
            while let Ok(agent_info) = rx.try_recv() {
                if let Some(id) = agent_info.get("id").and_then(|v| v.as_str()) {
                    discovered_agents.insert(id.to_string(), agent_info);
                }
            }

            // Update the shared agents list
            let mut agents_list = agents.write().await;
            agents_list.clear();
            agents_list.extend(discovered_agents.values().cloned());

            if !agents_list.is_empty() {
                println!("UI: {} agents currently discovered", agents_list.len());
            }

            drop(agents_list); // Release the lock

            tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        }
    }

    #[cfg(not(feature = "mdns"))]
    {
        println!("mDNS discovery not compiled in (disabled for musl builds)");
        println!("UI will not discover agents via mDNS");
        // Just sleep forever
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }
    }
}
