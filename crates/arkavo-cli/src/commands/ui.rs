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
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    use arkavo_protocol::mdns::MdnsManager;

    println!("Starting mDNS discovery service...");
    let mdns = MdnsManager::new();

    // Start mDNS discovery
    mdns.start_discovery().await?;
    println!("mDNS discovery started, looking for _a2a._tcp services");

    // Poll for discovered services
    let mut poll_count = 0;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        poll_count += 1;

        if poll_count % 5 == 0 {
            // Log every 10 seconds
            println!(
                "UI: mDNS discovery poll #{}, checking for services...",
                poll_count
            );
        }

        let discovered = mdns.get_discovered_services().await;

        let mut agents_list = agents.write().await;
        agents_list.clear();

        // If mDNS found agents, use them
        if !discovered.is_empty() {
            println!("UI: Found {} agents via mDNS!", discovered.len());
            for endpoint in discovered {
                println!(
                    "UI: Discovered agent: {} at {}",
                    endpoint.agent_id, endpoint.url
                );

                let url = endpoint.url.replace("http://", "");
                let agent_info = serde_json::json!({
                    "id": endpoint.agent_id.clone(),
                    "name": endpoint.agent_id.clone(),
                    "purpose": "Agent discovered via mDNS",
                    "model": "Unknown",
                    "endpoint": url
                });
                agents_list.push(agent_info);
            }
        } else if poll_count % 5 == 0 {
            println!("UI: No agents discovered via mDNS (poll #{})", poll_count);
        }
    }
}
