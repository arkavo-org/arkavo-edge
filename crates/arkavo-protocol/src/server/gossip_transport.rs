//! Gossip transport functions for message routing
//!
//! Routes gossip messages over the A2A peer network.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::broadcast;

use super::learning_bus::LearningBus;

/// Start gossip message transport over peer connections
///
/// This function listens for outgoing gossip messages from the LearningBus
/// and routes them to the appropriate peers via the message sender function.
pub async fn start_gossip_transport<F, Fut>(learning_bus: Arc<LearningBus>, send_to_peer: F)
where
    F: Fn(String, serde_json::Value) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Result<(), String>> + Send,
{
    let mut gossip_rx = learning_bus.subscribe_gossip_out();

    loop {
        match gossip_rx.recv().await {
            Ok((peer_id, message)) => {
                let payload = match serde_json::to_value(&message) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!("Failed to serialize gossip message: {}", e);
                        continue;
                    }
                };

                if let Err(e) = send_to_peer(peer_id.clone(), payload).await {
                    tracing::warn!("Failed to send gossip to {}: {}", peer_id, e);
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                tracing::warn!("Gossip transport lagged {} messages", n);
            }
            Err(broadcast::error::RecvError::Closed) => {
                tracing::info!("Gossip transport channel closed");
                break;
            }
        }
    }
}

/// Start periodic anti-entropy synchronization with peers
pub async fn start_anti_entropy_loop(learning_bus: Arc<LearningBus>, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;

        if let Err(e) = learning_bus.run_anti_entropy().await {
            tracing::debug!("Anti-entropy cycle error: {}", e);
        }
    }
}

/// Start periodic lesson synthesis and propagation
pub async fn start_lesson_propagation_loop(learning_bus: Arc<LearningBus>, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;

        if let Err(e) = learning_bus.synthesize_and_propagate_lessons().await {
            tracing::debug!("Lesson propagation error: {}", e);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;
    use arkavo_gossip::GossipConfig;

    #[tokio::test]
    async fn test_anti_entropy_loop_starts() {
        let keypair = Arc::new(AgentKeypair::generate());
        let bus = Arc::new(LearningBus::new(
            "test-agent".to_string(),
            "test-swarm".to_string(),
            keypair,
            GossipConfig::default(),
        ));

        // Run for a short time and cancel
        let handle = tokio::spawn({
            let bus = bus.clone();
            async move {
                start_anti_entropy_loop(bus, Duration::from_millis(100)).await;
            }
        });

        tokio::time::sleep(Duration::from_millis(250)).await;
        handle.abort();
    }
}
