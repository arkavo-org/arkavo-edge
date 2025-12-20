//! AutoLearner orchestrator
//!
//! Implements the four-step auto-learning loop:
//! 1. Pain Signal - Anomalies from runtime or proactive probing
//! 2. Synthesis - Ministral-3B generates TØRG graph
//! 3. Immune Response - Verify with InvariantLayer (distrust own LLM)
//! 4. Swarm Propagation - Broadcast verified patches via gossip

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use arkavo_ensemble::{CostFunction, GenerationMethod, PolicyEnsemble, PolicyLayer};
use arkavo_sat::ProbeScheduler;
use arkavo_sbe::InvariantLayer;
use torg_core::Graph;

use crate::error::{AutoLearnError, AutoLearnResult};
use crate::network::{GossipNetworkBridge, IncomingPatch};
use crate::patchlet::{PainTrigger, Patchlet, VerificationSummary};
use crate::signals::{PainAggregator, PainSignal};
use crate::synthesizer::MinistralSynthesizer;
use crate::verifier::{ImmuneVerifier, VerifierConfig};

/// Configuration for the auto-learning loop
#[derive(Debug, Clone)]
pub struct AutoLearnConfig {
    /// Interval for proactive boundary probing
    pub probe_interval: Duration,
    /// Minimum severity to trigger synthesis
    pub synthesis_threshold: f64,
    /// Timeout for synthesis operations
    pub synthesis_timeout: Duration,
    /// Maximum concurrent synthesis operations
    pub max_concurrent_synthesis: usize,
}

impl Default for AutoLearnConfig {
    fn default() -> Self {
        Self {
            probe_interval: Duration::from_secs(60),
            synthesis_threshold: 0.5,
            synthesis_timeout: Duration::from_secs(30),
            max_concurrent_synthesis: 2,
        }
    }
}

/// Statistics from the auto-learning loop
#[derive(Debug, Clone, Default)]
pub struct AutoLearnStats {
    /// Total pain signals processed
    pub signals_processed: u64,
    /// Successful syntheses
    pub syntheses_succeeded: u64,
    /// Failed syntheses
    pub syntheses_failed: u64,
    /// Patches broadcast
    pub patches_broadcast: u64,
    /// Remote patches received
    pub patches_received: u64,
    /// Remote patches accepted
    pub patches_accepted: u64,
    /// Remote patches rejected
    pub patches_rejected: u64,
}

/// The main auto-learning orchestrator
///
/// Coordinates the four-step learning cycle:
/// 1. Pain Signal aggregation
/// 2. LLM Synthesis via Ministral-3B
/// 3. Immune Response verification
/// 4. Swarm Propagation via gossip
pub struct AutoLearner<C: CostFunction> {
    /// Configuration
    config: AutoLearnConfig,
    /// The synthesizer (Ministral-3B)
    synthesizer: MinistralSynthesizer,
    /// The immune verifier
    verifier: ImmuneVerifier,
    /// Network bridge for gossip
    network: Arc<GossipNetworkBridge>,
    /// Policy ensemble for counterfactual evaluation
    ensemble: PolicyEnsemble<C>,
    /// Pain signal aggregator
    aggregator: PainAggregator,
    /// Probe scheduler for proactive boundary testing
    probe_scheduler: Option<ProbeScheduler>,
    /// Receiver for incoming patches
    patch_rx: Option<mpsc::Receiver<IncomingPatch>>,
    /// Statistics
    stats: AutoLearnStats,
}

impl<C: CostFunction> AutoLearner<C> {
    /// Create a new auto-learner
    pub fn new(
        config: AutoLearnConfig,
        synthesizer: MinistralSynthesizer,
        verifier: ImmuneVerifier,
        network: Arc<GossipNetworkBridge>,
        ensemble: PolicyEnsemble<C>,
    ) -> Self {
        Self {
            config,
            synthesizer,
            verifier,
            network,
            ensemble,
            aggregator: PainAggregator::new(),
            probe_scheduler: None,
            patch_rx: None,
            stats: AutoLearnStats::default(),
        }
    }

    /// Set the probe scheduler for proactive boundary testing
    pub fn with_probe_scheduler(mut self, scheduler: ProbeScheduler) -> Self {
        self.probe_scheduler = Some(scheduler);
        self
    }

    /// Set the patch receiver for incoming network patches
    pub fn with_patch_receiver(mut self, rx: mpsc::Receiver<IncomingPatch>) -> Self {
        self.patch_rx = Some(rx);
        self
    }

    /// Get the pain aggregator for adding signals
    pub fn aggregator_mut(&mut self) -> &mut PainAggregator {
        &mut self.aggregator
    }

    /// Get the ensemble for direct access
    pub fn ensemble(&self) -> &PolicyEnsemble<C> {
        &self.ensemble
    }

    /// Get mutable ensemble access
    pub fn ensemble_mut(&mut self) -> &mut PolicyEnsemble<C> {
        &mut self.ensemble
    }

    /// Get current statistics
    pub fn stats(&self) -> &AutoLearnStats {
        &self.stats
    }

    /// Run the main auto-learning loop
    pub async fn run(&mut self, cancel: CancellationToken) -> AutoLearnResult<()> {
        let mut probe_timer = interval(self.config.probe_interval);

        loop {
            // Check for cancellation first
            if cancel.is_cancelled() {
                tracing::info!("AutoLearner shutting down");
                break;
            }

            // Try to receive incoming patches
            let incoming_patch = if let Some(ref mut rx) = self.patch_rx {
                rx.try_recv().ok()
            } else {
                None
            };

            if let Some(patch) = incoming_patch {
                if let Err(e) = self.handle_remote_patch(patch).await {
                    tracing::warn!("Failed to handle remote patch: {}", e);
                }
                continue;
            }

            // Process local pain signals
            if let Some(signal) = self.aggregator.next_signal() {
                if signal.severity >= self.config.synthesis_threshold
                    && let Err(e) = self.process_signal(signal).await
                {
                    tracing::warn!("Failed to process signal: {}", e);
                }
                continue;
            }

            // Use select for timer-based operations only
            tokio::select! {
                _ = cancel.cancelled() => {
                    tracing::info!("AutoLearner shutting down");
                    break;
                }

                // Periodic proactive probing
                _ = probe_timer.tick() => {
                    if let Err(e) = self.probe_cycle().await {
                        tracing::debug!("Probe cycle error: {}", e);
                    }
                }

                // Small sleep to avoid busy-waiting
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }

        Ok(())
    }

    /// Process a single pain signal through the learning cycle
    async fn process_signal(&mut self, signal: PainSignal) -> AutoLearnResult<Option<Uuid>> {
        self.stats.signals_processed += 1;
        tracing::debug!("Processing pain signal: {}", signal.source.description());

        // Step 2: Synthesis via Ministral-3B
        let graph = match tokio::time::timeout(
            self.config.synthesis_timeout,
            self.synthesizer.synthesize_patchlet(&signal),
        )
        .await
        {
            Ok(Ok(graph)) => graph,
            Ok(Err(e)) => {
                self.stats.syntheses_failed += 1;
                tracing::debug!("Synthesis failed: {}", e.message);
                return Ok(None);
            }
            Err(_) => {
                self.stats.syntheses_failed += 1;
                return Err(AutoLearnError::Timeout);
            }
        };

        self.stats.syntheses_succeeded += 1;

        // Step 3: Immune Response - verify with InvariantLayer
        let verification = self.verifier.verify(&graph, &signal)?;
        if !verification.passed {
            tracing::debug!(
                "Verification failed: {} invariant violations, {} boundary issues",
                verification.invariant_violations.len(),
                verification.boundary_issues.len()
            );
            return Ok(None);
        }

        // Step 4: Swarm Propagation
        let patchlet = self.create_patchlet(&graph, &signal, &verification);
        let patch_id = self.network.broadcast_patch(&patchlet).await?;
        self.stats.patches_broadcast += 1;

        // Add to local ensemble for counterfactual evaluation
        let policy = PolicyLayer::new(graph);
        if let Err(e) = self.ensemble.add_candidate(
            policy,
            GenerationMethod::AnomalyRemediation {
                anomaly_id: signal.id,
            },
        ) {
            tracing::debug!("Failed to add candidate: {}", e);
        }

        tracing::info!("Broadcast patch {}", patch_id);
        Ok(Some(patch_id))
    }

    /// Handle a patch received from the network (zero-trust)
    async fn handle_remote_patch(&mut self, incoming: IncomingPatch) -> AutoLearnResult<()> {
        self.stats.patches_received += 1;
        tracing::debug!("Received remote patch: {}", incoming.patch_id);

        let graph = incoming.patchlet.graph.clone();

        // Zero-trust: verify independently (distrust remote LLM)
        let verification = self.verifier.deep_verify(&graph)?;
        let approve = verification.passed;

        // Vote on the patch
        self.network.vote_on_patch(incoming.patch_id, approve).await?;

        if approve {
            self.stats.patches_accepted += 1;

            // Add to local ensemble
            let policy = PolicyLayer::new(graph);
            if let Err(e) = self.ensemble.add_candidate(
                policy,
                GenerationMethod::AnomalyRemediation {
                    anomaly_id: incoming.patchlet.trigger.anomaly_id,
                },
            ) {
                tracing::debug!("Failed to add remote candidate: {}", e);
            }

            tracing::info!("Accepted remote patch {}", incoming.patch_id);
        } else {
            self.stats.patches_rejected += 1;
            tracing::info!("Rejected remote patch {}", incoming.patch_id);
        }

        Ok(())
    }

    /// Run a proactive probing cycle
    async fn probe_cycle(&mut self) -> AutoLearnResult<()> {
        let Some(ref _scheduler) = self.probe_scheduler else {
            return Ok(());
        };

        // Get the production graph for probing
        let graph = &self.ensemble.production().graph;

        // Select top nodes for probing
        let targets = self.aggregator.select_probe_targets(graph, 3);

        for target in targets {
            // Record the probe in the prioritizer
            tracing::trace!(
                "Probing output {} with score {:.3}",
                target.output_id,
                target.score
            );
        }

        Ok(())
    }

    /// Create a patchlet from a verified graph
    fn create_patchlet(
        &self,
        graph: &Graph,
        signal: &PainSignal,
        verification: &crate::verifier::VerificationResult,
    ) -> Patchlet {
        let trigger = PainTrigger {
            anomaly_id: signal.id,
            description: signal.source.description(),
            severity: signal.severity,
            timestamp: signal.timestamp,
        };

        let verification_summary = VerificationSummary {
            passed: verification.passed,
            invariant_checks: verification.invariant_violations.len() as u32,
            sat_probes: verification
                .stress_stats
                .as_ref()
                .map(|s| s.inputs_tested as u32)
                .unwrap_or(0),
        };

        let method = GenerationMethod::AnomalyRemediation {
            anomaly_id: signal.id,
        };

        Patchlet::new(graph.clone(), trigger, method).with_verification(verification_summary)
    }
}

/// Builder for AutoLearner with sensible defaults
pub struct AutoLearnerBuilder<C: CostFunction> {
    config: AutoLearnConfig,
    synthesizer: Option<MinistralSynthesizer>,
    invariant_layer: Option<Arc<InvariantLayer>>,
    verifier_config: VerifierConfig,
    network: Option<Arc<GossipNetworkBridge>>,
    ensemble: Option<PolicyEnsemble<C>>,
    probe_scheduler: Option<ProbeScheduler>,
    patch_rx: Option<mpsc::Receiver<IncomingPatch>>,
}

impl<C: CostFunction> AutoLearnerBuilder<C> {
    /// Create a new builder with default config
    pub fn new() -> Self {
        Self {
            config: AutoLearnConfig::default(),
            synthesizer: None,
            invariant_layer: None,
            verifier_config: VerifierConfig::default(),
            network: None,
            ensemble: None,
            probe_scheduler: None,
            patch_rx: None,
        }
    }

    /// Set the configuration
    pub fn config(mut self, config: AutoLearnConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the synthesizer
    pub fn synthesizer(mut self, synthesizer: MinistralSynthesizer) -> Self {
        self.synthesizer = Some(synthesizer);
        self
    }

    /// Set the invariant layer
    pub fn invariant_layer(mut self, layer: Arc<InvariantLayer>) -> Self {
        self.invariant_layer = Some(layer);
        self
    }

    /// Set the verifier config
    pub fn verifier_config(mut self, config: VerifierConfig) -> Self {
        self.verifier_config = config;
        self
    }

    /// Set the network bridge
    pub fn network(mut self, network: Arc<GossipNetworkBridge>) -> Self {
        self.network = Some(network);
        self
    }

    /// Set the policy ensemble
    pub fn ensemble(mut self, ensemble: PolicyEnsemble<C>) -> Self {
        self.ensemble = Some(ensemble);
        self
    }

    /// Set the probe scheduler
    pub fn probe_scheduler(mut self, scheduler: ProbeScheduler) -> Self {
        self.probe_scheduler = Some(scheduler);
        self
    }

    /// Set the patch receiver
    pub fn patch_receiver(mut self, rx: mpsc::Receiver<IncomingPatch>) -> Self {
        self.patch_rx = Some(rx);
        self
    }

    /// Build the AutoLearner
    pub fn build(self) -> AutoLearnResult<AutoLearner<C>> {
        let synthesizer = self
            .synthesizer
            .ok_or_else(|| AutoLearnError::Internal("Synthesizer required".into()))?;

        let invariant_layer = self
            .invariant_layer
            .ok_or_else(|| AutoLearnError::Internal("Invariant layer required".into()))?;

        let network = self
            .network
            .ok_or_else(|| AutoLearnError::Internal("Network required".into()))?;

        let ensemble = self
            .ensemble
            .ok_or_else(|| AutoLearnError::Internal("Ensemble required".into()))?;

        let verifier = ImmuneVerifier::with_config(invariant_layer, self.verifier_config);

        let mut learner = AutoLearner::new(
            self.config,
            synthesizer,
            verifier,
            network,
            ensemble,
        );

        if let Some(scheduler) = self.probe_scheduler {
            learner = learner.with_probe_scheduler(scheduler);
        }

        if let Some(rx) = self.patch_rx {
            learner = learner.with_patch_receiver(rx);
        }

        Ok(learner)
    }
}

impl<C: CostFunction> Default for AutoLearnerBuilder<C> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;
    use arkavo_ensemble::ConstantCost;
    use torg_core::{Builder, Token};

    use crate::network::NetworkConfig;

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
    fn test_config_defaults() {
        let config = AutoLearnConfig::default();
        assert_eq!(config.probe_interval, Duration::from_secs(60));
        assert_eq!(config.synthesis_threshold, 0.5);
        assert_eq!(config.max_concurrent_synthesis, 2);
    }

    #[test]
    fn test_stats_default() {
        let stats = AutoLearnStats::default();
        assert_eq!(stats.signals_processed, 0);
        assert_eq!(stats.syntheses_succeeded, 0);
    }

    #[tokio::test]
    async fn test_learner_creation() {
        let graph = create_simple_graph();
        let production = PolicyLayer::new(graph);
        let invariant_layer = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::new(1.0);
        let ensemble = PolicyEnsemble::new(production, invariant_layer.clone(), cost);

        let synthesizer = MinistralSynthesizer::new().unwrap();
        let verifier = ImmuneVerifier::new(invariant_layer);

        let keypair = AgentKeypair::generate();
        let network = Arc::new(crate::network::GossipNetworkBridge::new(
            "test-agent".to_string(),
            keypair,
            NetworkConfig::default(),
        ));

        let learner = AutoLearner::new(
            AutoLearnConfig::default(),
            synthesizer,
            verifier,
            network,
            ensemble,
        );

        assert_eq!(learner.stats().signals_processed, 0);
    }

    #[tokio::test]
    async fn test_builder_missing_required() {
        let result: Result<AutoLearner<ConstantCost>, _> = AutoLearnerBuilder::new().build();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_builder_complete() {
        let graph = create_simple_graph();
        let production = PolicyLayer::new(graph);
        let invariant_layer = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::new(1.0);
        let ensemble = PolicyEnsemble::new(production, invariant_layer.clone(), cost);

        let synthesizer = MinistralSynthesizer::new().unwrap();
        let keypair = AgentKeypair::generate();
        let network = Arc::new(crate::network::GossipNetworkBridge::new(
            "test-agent".to_string(),
            keypair,
            NetworkConfig::default(),
        ));

        let learner = AutoLearnerBuilder::new()
            .synthesizer(synthesizer)
            .invariant_layer(invariant_layer)
            .network(network)
            .ensemble(ensemble)
            .build()
            .unwrap();

        assert_eq!(learner.stats().signals_processed, 0);
    }
}
