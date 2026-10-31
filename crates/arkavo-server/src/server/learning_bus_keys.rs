//! Gossip keys: the agent's own, vouched for by its workload key, and its
//! peers', bound through the statements they show (INGRESS-017).

use std::sync::Arc;

use arkavo_crypto::AgentPublicKey;
use arkavo_device_identity::subordinate_key::Voucher;
use arkavo_gossip::GossipResult;

use super::learning_bus::LearningBus;

impl LearningBus {
    /// Show peers the statement `voucher` signs for this bus's gossip key.
    /// Without it the agent shows none, as agents from before key statements
    /// do. A voucher for any other key is refused: its statement would vouch
    /// for a key this bus does not sign with.
    pub fn set_gossip_voucher(&self, voucher: Arc<Voucher>) {
        if voucher.key() != self.keypair.public_key().to_did_key() {
            tracing::error!(
                "Not showing peers a statement for {}, which is not this agent's gossip key",
                voucher.key()
            );
            return;
        }
        let _ = self.gossip_voucher.set(voucher);
    }

    /// The statement to show a peer alongside this agent's gossip key, or
    /// none if no workload key vouches for it or signing failed.
    pub async fn gossip_statement(&self) -> Option<String> {
        let voucher = self.gossip_voucher.get()?.clone();
        match tokio::task::spawn_blocking(move || voucher.statement()).await {
            Ok(Ok(statement)) => Some(statement),
            Ok(Err(e)) => {
                tracing::warn!("Could not sign the gossip key statement: {e}");
                None
            }
            Err(e) => {
                tracing::warn!("Signing the gossip key statement stopped: {e}");
                None
            }
        }
    }

    /// Bind `peer_id` to a gossip key it has just proven it holds, through
    /// the key statement it showed, if any. Returns the workload `did:key`
    /// that vouched for the key.
    pub async fn bind_peer_key(
        &self,
        peer_id: String,
        key: AgentPublicKey,
        statement: Option<&str>,
    ) -> GossipResult<Option<String>> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |elapsed| elapsed.as_secs());
        let vouched = arkavo_gossip::key_exchange::vouched_by(statement, &key, now)?;
        let gossip = self.gossip.read().await;
        match vouched {
            Some(statement) => {
                gossip
                    .register_vouched_key(peer_id, key, &statement)
                    .await?;
                Ok(Some(statement.issuer))
            }
            None => {
                gossip.register_key(peer_id, key).await?;
                Ok(None)
            }
        }
    }
}
