//! Auto-learning loop for self-healing agents
//!
//! This crate implements Phase 4 of Symbolic Boundary Evolution (SBE):
//! the auto-learning loop with swarm propagation.
//!
//! # Four-Step Learning Cycle
//!
//! 1. **Pain Signal**: Anomalies from runtime monitoring or proactive boundary probing
//! 2. **Synthesis**: Ministral-3B generates TØRG graph within Adaptive Layer constraints
//! 3. **Immune Response**: Agent verifies via InvariantLayer (distrusts its own LLM)
//! 4. **Swarm Propagation**: Verified patches broadcast via gossip; receivers verify independently
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────┐
//! │                       AutoLearner                                │
//! │  ┌─────────────┐  ┌──────────────┐  ┌────────────────────────┐ │
//! │  │ PainAggregator│→│MinistralSynth │→│   ImmuneVerifier       │ │
//! │  │  (signals)    │  │  (LLM)       │  │ (InvariantLayer+SAT) │ │
//! │  └─────────────┘  └──────────────┘  └────────────────────────┘ │
//! │                                              ↓                  │
//! │                          ┌───────────────────────────────────┐ │
//! │                          │   GossipNetworkBridge             │ │
//! │                          │  (Patchlet broadcast + voting)    │ │
//! │                          └───────────────────────────────────┘ │
//! └─────────────────────────────────────────────────────────────────┘
//! ```
//!
//! # Requirements Implemented
//!
//! - AUTO-001: AutoLearner orchestrator struct
//! - AUTO-002: Ministral-3B integration for patchlet synthesis
//! - AUTO-003: Anomaly → synthesis → verification → propagation loop
//! - AUTO-004: Gossip protocol for swarm-wide patchlet distribution
//! - AUTO-005: Zero-trust verification for remote patches
//!
//! # Example
//!
//! ```ignore
//! use arkavo_autolearn::{AutoLearner, AutoLearnerBuilder, AutoLearnConfig};
//! use arkavo_ensemble::{PolicyEnsemble, ConstantCost, PolicyLayer};
//! use arkavo_sbe::InvariantLayer;
//! use std::sync::Arc;
//!
//! // Create the auto-learner
//! let learner = AutoLearnerBuilder::new()
//!     .synthesizer(synthesizer)
//!     .invariant_layer(Arc::new(InvariantLayer::new()))
//!     .network(network)
//!     .ensemble(ensemble)
//!     .build()?;
//!
//! // Run the learning loop
//! let cancel = CancellationToken::new();
//! learner.run(cancel).await?;
//! ```

mod error;
mod learner;
mod network;
mod patchlet;
mod signals;
mod synthesizer;
mod verifier;

// Re-export main types
pub use error::{AutoLearnError, AutoLearnResult};
pub use learner::{AutoLearnConfig, AutoLearnStats, AutoLearner, AutoLearnerBuilder};
pub use network::{
    GossipNetworkBridge, IncomingPatch, NetworkConfig, NetworkHandle, create_network,
};
pub use patchlet::{PainTrigger, Patchlet, VerificationSummary};
pub use signals::{DEFAULT_MAX_SIGNALS, PainAggregator, PainSignal, PainSource};
pub use synthesizer::{MinistralSynthesizer, SynthesizerConfig};
pub use verifier::{
    DEFAULT_VERIFICATION_TIMEOUT_MS, ImmuneVerifier, InvariantViolation, StressStats,
    VerificationResult, VerifierConfig,
};

// Re-export dependencies for convenience
pub use arkavo_ensemble::{CostFunction, GenerationMethod, PolicyEnsemble, PolicyLayer};
pub use arkavo_gossip::{GossipConfig, GossipMessage, PatchStatus};
pub use arkavo_sbe::InvariantLayer;

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;
    use arkavo_ensemble::ConstantCost;
    use std::sync::Arc;
    use torg_core::{Builder, Graph, Token};

    fn create_simple_graph() -> Graph {
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn test_exports_available() {
        // Verify all public types are accessible
        let _config = AutoLearnConfig::default();
        let _stats = AutoLearnStats::default();
        let _synth_config = SynthesizerConfig::default();
        let _verifier_config = VerifierConfig::default();
        let _network_config = NetworkConfig::default();
    }

    #[test]
    fn test_pain_signal_workflow() {
        use arkavo_ensemble::SynthesisContext;

        // Create a pain signal
        let ctx = SynthesisContext::new("test-model".to_string(), "Test synthesis".to_string());
        let signal = PainSignal::new(
            PainSource::External {
                description: "Test pain".to_string(),
            },
            0.8,
            ctx,
        );

        // Add to aggregator
        let mut aggregator = PainAggregator::new();
        aggregator.push(signal);

        // Should have one pending signal
        assert_eq!(aggregator.pending_count(), 1);

        // Get the signal back
        let retrieved = aggregator.next_signal().unwrap();
        assert_eq!(retrieved.severity, 0.8);
    }

    #[test]
    fn test_patchlet_serialization() {
        use chrono::Utc;
        use uuid::Uuid;

        let graph = create_simple_graph();
        let trigger = PainTrigger {
            anomaly_id: Uuid::new_v4(),
            description: "Test trigger".to_string(),
            severity: 0.7,
            timestamp: Utc::now(),
        };

        let patchlet = Patchlet::new(graph.clone(), trigger, GenerationMethod::Manual);

        // Serialize and deserialize
        let bytes = patchlet.to_bytes().unwrap();
        let restored = Patchlet::from_bytes(&bytes).unwrap();

        assert_eq!(restored.graph.inputs, graph.inputs);
        assert_eq!(restored.graph.outputs, graph.outputs);
    }

    #[test]
    fn test_verifier_integration() {
        use arkavo_ensemble::SynthesisContext;

        let graph = create_simple_graph();
        let invariant_layer = Arc::new(InvariantLayer::new());
        let verifier = ImmuneVerifier::new(invariant_layer);

        let ctx = SynthesisContext::new("test-model".to_string(), "Test".to_string());
        let signal = PainSignal::new(
            PainSource::External {
                description: "test".to_string(),
            },
            0.5,
            ctx,
        );

        let result = verifier.verify(&graph, &signal).unwrap();
        assert!(result.passed);
    }

    #[tokio::test]
    async fn test_network_bridge() {
        let keypair = AgentKeypair::generate();
        let bridge =
            GossipNetworkBridge::new("test-agent".to_string(), keypair, NetworkConfig::default());

        bridge.add_peer("peer-1".to_string()).await;
        assert_eq!(bridge.peer_count().await, 1);
    }

    #[tokio::test]
    async fn test_full_integration() {
        // Create all components
        let graph = create_simple_graph();
        let production = PolicyLayer::new(graph);
        let invariant_layer = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::new(1.0);
        let ensemble = PolicyEnsemble::new(production, invariant_layer.clone(), cost);

        let synthesizer = Arc::new(MinistralSynthesizer::new().unwrap());
        let keypair = AgentKeypair::generate();
        let network = Arc::new(GossipNetworkBridge::new(
            "test-agent".to_string(),
            keypair,
            NetworkConfig::default(),
        ));

        // Build the learner
        let learner = AutoLearnerBuilder::new()
            .synthesizer(synthesizer)
            .invariant_layer(invariant_layer)
            .network(network)
            .ensemble(ensemble)
            .build()
            .unwrap();

        // Verify it was created
        assert_eq!(learner.stats().signals_processed, 0);
        assert!(learner.ensemble().candidates().is_empty());
    }
}
