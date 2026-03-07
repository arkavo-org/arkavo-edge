//! Bridge wiring arkavo-autolearn into the agent runtime
//!
//! Constructs a minimal AutoLearner, wires its gossip transport through
//! the existing LearningBus, and spawns it as a background task.

use std::sync::Arc;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use arkavo_autolearn::{
    AutoLearnerBuilder, GossipNetworkBridge, InvariantLayer, NetworkConfig, PainSignal,
    PolicyEnsemble, PolicyLayer, PolicySynthesizer,
};
use arkavo_crypto::AgentKeypair;
use arkavo_ensemble::ConstantCost;
use torg_core::{Builder, Token};

use super::learning_bus::LearningBus;

pub struct AutoLearnBridge {
    pub handle: JoinHandle<()>,
}

impl AutoLearnBridge {
    /// Build and spawn an AutoLearner as a background task.
    ///
    /// # Panics
    ///
    /// Panics if the minimal bootstrap policy graph cannot be constructed (programming error).
    pub fn new(
        agent_name: String,
        learning_bus: Arc<LearningBus>,
        pain_rx: mpsc::Receiver<PainSignal>,
    ) -> Result<Self, arkavo_autolearn::AutoLearnError> {
        let graph = {
            let mut b = Builder::new();
            b.push(Token::InputDecl).unwrap();
            b.push(Token::Id(0)).unwrap();
            b.push(Token::NodeStart).unwrap();
            b.push(Token::Id(1)).unwrap();
            b.push(Token::Or).unwrap();
            b.push(Token::Id(0)).unwrap();
            b.push(Token::Id(0)).unwrap();
            b.push(Token::NodeEnd).unwrap();
            b.push(Token::OutputDecl).unwrap();
            b.push(Token::Id(1)).unwrap();
            b.finish().unwrap()
        };
        let invariant_layer = Arc::new(InvariantLayer::new());
        let ensemble = PolicyEnsemble::new(
            PolicyLayer::new(graph),
            invariant_layer.clone(),
            ConstantCost::zero(),
        );
        let synthesizer = Arc::new(PolicySynthesizer::new()?);

        let keypair = AgentKeypair::generate();
        let mut network_bridge =
            GossipNetworkBridge::new(agent_name.clone(), keypair, NetworkConfig::default());
        let outbox_rx = network_bridge.take_outbox().unwrap();
        let patch_rx = network_bridge.take_patch_receiver().unwrap();
        let network_bridge = Arc::new(network_bridge);

        learning_bus.set_autolearn_bridge(network_bridge.clone());

        let mut learner = AutoLearnerBuilder::new()
            .synthesizer(synthesizer)
            .invariant_layer(invariant_layer)
            .network(network_bridge)
            .ensemble(ensemble)
            .model_id(agent_name)
            .with_pain_receiver(pain_rx)
            .patch_receiver(patch_rx)
            .build()?;

        // Forward patchlet outbox to all peers via LearningBus transport
        let lb = learning_bus;
        tokio::spawn(async move {
            let mut outbox = outbox_rx;
            while let Some(msg) = outbox.recv().await {
                let peers = lb.get_all_peer_addresses().await;
                for (peer_id, _) in peers {
                    let _ = lb.gossip_out_tx().send((peer_id, msg.clone()));
                }
            }
        });

        let handle = tokio::spawn(async move {
            let cancel = tokio_util::sync::CancellationToken::new();
            if let Err(e) = learner.run(cancel).await {
                tracing::error!("AutoLearner terminated: {e}");
            }
        });

        Ok(Self { handle })
    }
}
