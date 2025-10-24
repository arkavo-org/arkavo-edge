use crate::agent_connection::{AgentConnection, TelemetryEvent};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{RwLock, mpsc};

/// Run mDNS discovery for A2A agents
pub async fn run_mdns_discovery(
    agents: Arc<RwLock<Vec<serde_json::Value>>>,
    agent_connections: Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    telemetry_tx: mpsc::Sender<TelemetryEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(feature = "mdns")]
    {
        crate::mdns_impl::mdns::discover_agents(agents, agent_connections, telemetry_tx).await
    }

    #[cfg(not(feature = "mdns"))]
    {
        let _ = (agents, agent_connections, telemetry_tx);
        println!("mDNS discovery not compiled in");
        loop {
            tokio::time::sleep(tokio::time::Duration::from_secs(60)).await;
        }
    }
}
