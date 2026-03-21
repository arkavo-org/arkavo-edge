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
    tracing::info!(
        "Starting anti-entropy loop for agent {} (interval: {:?})",
        learning_bus.agent_id(),
        interval
    );
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;

        let peer_count = learning_bus.peer_count().await;
        tracing::debug!("Anti-entropy tick: {} peers connected", peer_count);

        if let Err(e) = learning_bus.run_anti_entropy().await {
            tracing::debug!("Anti-entropy cycle error: {}", e);
        }
    }
}

/// Start periodic lesson synthesis and propagation
pub async fn start_lesson_propagation_loop(learning_bus: Arc<LearningBus>, interval: Duration) {
    tracing::info!(
        "Starting lesson propagation loop for agent {} (interval: {:?})",
        learning_bus.agent_id(),
        interval
    );
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;

        tracing::debug!("Lesson propagation tick");

        if let Err(e) = learning_bus.synthesize_and_propagate_lessons().await {
            tracing::debug!("Lesson propagation error: {}", e);
        }
    }
}

/// Start periodic broadcast of proven advisor adjustments to mesh peers
pub async fn start_advisor_broadcast_loop(learning_bus: Arc<LearningBus>, interval: Duration) {
    tracing::info!(
        "Starting advisor broadcast loop for agent {} (interval: {:?})",
        learning_bus.agent_id(),
        interval
    );
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;

        tracing::debug!("Advisor broadcast tick");

        learning_bus.broadcast_advisor_adjustments().await;
    }
}

/// Start periodic cleanup of expired gossip entries
///
/// Cleans up patches, lessons, and seen_* maps older than max_message_age.
pub async fn start_cleanup_loop(learning_bus: Arc<LearningBus>, interval: Duration) {
    tracing::info!(
        "Starting gossip cleanup loop for agent {} (interval: {:?})",
        learning_bus.agent_id(),
        interval
    );
    let mut ticker = tokio::time::interval(interval);

    loop {
        ticker.tick().await;

        tracing::debug!("Gossip cleanup tick");

        let gossip = learning_bus.gossip().read().await;
        gossip.cleanup_expired().await;
        let stats = gossip.stats().await;
        drop(gossip);

        tracing::debug!(
            "Gossip stats after cleanup: {} patches, {} lessons, {} seen_messages",
            stats.patch_count,
            stats.lesson_count,
            stats.seen_messages_count
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;
    use arkavo_gossip::GossipConfig;
    use arkavo_test_macros::spec;

    #[spec("SRV-006")]
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
