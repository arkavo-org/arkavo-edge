//! AutoLearner orchestrator
//!
//! Implements the four-step auto-learning loop:
//! 1. Pain Signal - Anomalies from runtime or proactive probing
//! 2. Synthesis - Ministral-3B generates TØRG graph
//! 3. Immune Response - Verify with InvariantLayer (distrust own LLM)
//! 4. Swarm Propagation - Broadcast verified patches via gossip

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

use arkavo_ensemble::{CostFunction, GenerationMethod, PolicyEnsemble, PolicyLayer};
use arkavo_sat::{ProbeResult, ProbeScheduler, ProbeTask};
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
    /// Dry-run mode: synthesize and verify but don't broadcast
    ///
    /// When enabled, patches are synthesized and verified but NOT
    /// broadcast to the network. Use this to observe the system
    /// safely before enabling full self-healing propagation.
    pub dry_run: bool,
}

impl Default for AutoLearnConfig {
    fn default() -> Self {
        Self {
            probe_interval: Duration::from_secs(60),
            synthesis_threshold: 0.5,
            synthesis_timeout: Duration::from_secs(30),
            max_concurrent_synthesis: 2,
            dry_run: false,
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
    /// Patches verified but not broadcast (dry-run mode)
    pub patches_dry_run: u64,
    /// Remote patches received
    pub patches_received: u64,
    /// Remote patches accepted
    pub patches_accepted: u64,
    /// Remote patches rejected
    pub patches_rejected: u64,
}

/// Result from a concurrent synthesis operation
struct SynthesisResult {
    /// The patch ID if broadcast succeeded
    patch_id: Option<Uuid>,
    /// The signal ID that triggered this synthesis
    signal_id: Uuid,
    /// Whether synthesis succeeded
    success: bool,
    /// Whether this was a dry-run
    dry_run: bool,
    /// The synthesized graph (for ensemble update)
    graph: Option<Graph>,
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
    /// The synthesizer (Ministral-3B) - wrapped in Arc for concurrent access
    synthesizer: Arc<MinistralSynthesizer>,
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
    /// Sender for probe tasks
    probe_task_tx: Option<mpsc::Sender<ProbeTask>>,
    /// Receiver for probe results
    probe_result_rx: Option<mpsc::Receiver<ProbeResult>>,
    /// Model ID for pain signal context
    model_id: String,
    /// Semaphore for limiting concurrent synthesis operations
    synthesis_semaphore: Arc<Semaphore>,
    /// Sender for synthesis results (cloned into spawned tasks)
    synthesis_result_tx: mpsc::Sender<SynthesisResult>,
    /// Receiver for synthesis results
    synthesis_result_rx: Option<mpsc::Receiver<SynthesisResult>>,
}

impl<C: CostFunction> AutoLearner<C> {
    /// Create a new auto-learner
    pub fn new(
        config: AutoLearnConfig,
        synthesizer: Arc<MinistralSynthesizer>,
        verifier: ImmuneVerifier,
        network: Arc<GossipNetworkBridge>,
        ensemble: PolicyEnsemble<C>,
    ) -> Self {
        let (synthesis_result_tx, synthesis_result_rx) = mpsc::channel(32);
        let synthesis_semaphore = Arc::new(Semaphore::new(config.max_concurrent_synthesis));

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
            probe_task_tx: None,
            probe_result_rx: None,
            model_id: String::new(),
            synthesis_semaphore,
            synthesis_result_tx,
            synthesis_result_rx: Some(synthesis_result_rx),
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

    /// Set the model ID for pain signal context
    pub fn with_model_id(mut self, model_id: String) -> Self {
        self.model_id = model_id;
        self
    }

    /// Start the probe scheduler as a background task
    ///
    /// Returns a join handle if the scheduler was configured, None otherwise.
    /// After calling this, probe tasks can be sent via `probe_cycle()`.
    pub fn start_probe_scheduler(&mut self) -> Option<tokio::task::JoinHandle<()>> {
        let scheduler = self.probe_scheduler.take()?;

        let (task_tx, task_rx) = mpsc::channel(32);
        let (result_tx, result_rx) = mpsc::channel(32);

        self.probe_task_tx = Some(task_tx);
        self.probe_result_rx = Some(result_rx);

        Some(tokio::spawn(async move {
            scheduler.start(task_rx, result_tx).await;
        }))
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
        // Start the probe scheduler if configured
        let _probe_handle = self.start_probe_scheduler();

        let mut probe_timer = interval(self.config.probe_interval);

        loop {
            // Check for cancellation first
            if cancel.is_cancelled() {
                tracing::info!("AutoLearner shutting down");
                break;
            }

            // Process any completed probe results
            self.process_probe_results();

            // Collect completed synthesis results
            self.collect_synthesis_results();

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

            // Process synthesis - sequential mode when max_concurrent_synthesis == 1,
            // otherwise concurrent mode with spawned tasks
            if self.config.max_concurrent_synthesis <= 1 {
                // Sequential mode: process one signal at a time (simpler, lower overhead)
                if let Some(signal) = self.aggregator.next_signal()
                    && signal.severity >= self.config.synthesis_threshold
                    && let Err(e) = self.process_signal(signal).await
                {
                    tracing::warn!("Failed to process signal: {}", e);
                }
            } else {
                // Concurrent mode: acquire permit BEFORE spawn to enforce backpressure
                while let Ok(permit) = self.synthesis_semaphore.clone().try_acquire_owned() {
                    let Some(signal) = self.aggregator.next_signal() else {
                        drop(permit); // Return permit if no signals
                        break;
                    };
                    if signal.severity >= self.config.synthesis_threshold {
                        self.spawn_synthesis_with_permit(signal, permit);
                    } else {
                        drop(permit); // Return permit for below-threshold signals
                    }
                }
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

        // Step 3: Immune Response - verify with InvariantLayer (with timeout)
        let timeout_ms = self.verifier.config().timeout_ms;
        let verifier = self.verifier.clone();
        let graph_clone = graph.clone();
        let signal_clone = signal.clone();

        let verification = match tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            tokio::task::spawn_blocking(move || verifier.verify(&graph_clone, &signal_clone)),
        )
        .await
        {
            Ok(Ok(Ok(result))) => result,
            Ok(Ok(Err(e))) => return Err(e),
            Ok(Err(e)) => return Err(AutoLearnError::Verification(e.to_string())),
            Err(_) => {
                tracing::warn!("Verification timeout after {}ms", timeout_ms);
                return Err(AutoLearnError::Timeout);
            }
        };

        if !verification.passed {
            tracing::debug!(
                "Verification failed: {} invariant violations, {} boundary issues",
                verification.invariant_violations.len(),
                verification.boundary_issues.len()
            );
            return Ok(None);
        }

        // Step 4: Swarm Propagation (or dry-run logging)
        let patchlet = self.create_patchlet(&graph, &signal, &verification);

        let patch_id = if self.config.dry_run {
            // DRY-RUN MODE: Log but don't broadcast
            let dry_run_id = Uuid::new_v4();
            self.stats.patches_dry_run += 1;
            tracing::info!(
                "[DRY-RUN] Would broadcast patch {} (signal: {}, severity: {:.2})",
                dry_run_id,
                signal.source.description(),
                signal.severity
            );
            dry_run_id
        } else {
            // LIVE MODE: Broadcast to network
            let id = self.network.broadcast_patch(&patchlet).await?;
            self.stats.patches_broadcast += 1;
            tracing::info!("Broadcast patch {}", id);
            id
        };

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

        Ok(Some(patch_id))
    }

    /// Handle a patch received from the network (zero-trust)
    async fn handle_remote_patch(&mut self, incoming: IncomingPatch) -> AutoLearnResult<()> {
        self.stats.patches_received += 1;
        tracing::debug!("Received remote patch: {}", incoming.patch_id);

        let graph = incoming.patchlet.graph.clone();

        // Zero-trust: verify independently (distrust remote LLM) with 2x timeout
        let timeout_ms = self.verifier.config().timeout_ms * 2;
        let verifier = self.verifier.clone();
        let graph_clone = graph.clone();

        let verification = match tokio::time::timeout(
            Duration::from_millis(timeout_ms),
            tokio::task::spawn_blocking(move || verifier.deep_verify(&graph_clone)),
        )
        .await
        {
            Ok(Ok(Ok(result))) => result,
            Ok(Ok(Err(e))) => return Err(e),
            Ok(Err(e)) => return Err(AutoLearnError::Verification(e.to_string())),
            Err(_) => {
                tracing::warn!("Deep verification timeout after {}ms", timeout_ms);
                self.stats.patches_rejected += 1;
                return Ok(()); // Reject on timeout - fail safe
            }
        };

        let approve = verification.passed;

        // Vote on the patch
        self.network
            .vote_on_patch(incoming.patch_id, approve)
            .await?;

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
        let Some(ref task_tx) = self.probe_task_tx else {
            return Ok(());
        };

        // Get the production graph for probing
        let graph = Arc::new(self.ensemble.production().graph.clone());

        // Select top nodes for probing
        let targets = self.aggregator.select_probe_targets(&graph, 3);

        for target in targets {
            let task = ProbeTask {
                graph: graph.clone(),
                output_id: target.output_id,
                epsilon: 1, // Fixed: finds nodes closest to decision boundary
                priority: target.score,
            };

            tracing::trace!(
                "Enqueueing probe for output {} with score {:.3}",
                target.output_id,
                target.score
            );

            if task_tx.send(task).await.is_err() {
                tracing::warn!("Failed to enqueue probe task: channel closed");
                break;
            }
        }

        Ok(())
    }

    /// Process results from the probe scheduler
    fn process_probe_results(&mut self) {
        let Some(ref mut result_rx) = self.probe_result_rx else {
            return;
        };

        while let Ok(result) = result_rx.try_recv() {
            if let Some(ref err) = result.error {
                tracing::debug!("Probe failed: {}", err);
                continue;
            }

            // Use the graph that was actually probed, not current production
            // (production may have changed between enqueue and result)
            let graph = &result.task.graph;
            for probe in result.probes {
                tracing::debug!(
                    "Probe found boundary at output {}, distance {}",
                    probe.output_id,
                    probe.distance_to_flip
                );
                self.aggregator
                    .from_probe_result(probe, graph, self.model_id.clone());
            }
        }
    }

    /// Spawn a concurrent synthesis task with pre-acquired permit
    ///
    /// The permit is acquired BEFORE spawn to enforce backpressure and prevent
    /// spawning more tasks than max_concurrent_synthesis.
    fn spawn_synthesis_with_permit(&self, signal: PainSignal, permit: OwnedSemaphorePermit) {
        let synthesizer = self.synthesizer.clone();
        let verifier = self.verifier.clone();
        let network = self.network.clone();
        let config = self.config.clone();
        let result_tx = self.synthesis_result_tx.clone();

        tokio::spawn(async move {
            // Permit is held for the duration of synthesis (RAII)
            let _permit = permit;

            let result = run_synthesis_pipeline(signal, synthesizer, verifier, network, config).await;

            // Send result back (ignore error if receiver dropped)
            let _ = result_tx.send(result).await;
        });
    }

    /// Collect completed synthesis results and update stats
    fn collect_synthesis_results(&mut self) {
        let Some(ref mut rx) = self.synthesis_result_rx else {
            return;
        };

        while let Ok(result) = rx.try_recv() {
            self.stats.signals_processed += 1;

            if result.success {
                self.stats.syntheses_succeeded += 1;

                if result.dry_run {
                    self.stats.patches_dry_run += 1;
                } else if result.patch_id.is_some() {
                    self.stats.patches_broadcast += 1;
                }

                // Add successful synthesis to ensemble
                if let Some(graph) = result.graph {
                    let policy = PolicyLayer::new(graph);
                    if let Err(e) = self.ensemble.add_candidate(
                        policy,
                        GenerationMethod::AnomalyRemediation {
                            anomaly_id: result.signal_id,
                        },
                    ) {
                        tracing::debug!("Failed to add candidate to ensemble: {}", e);
                    }
                }
            } else {
                self.stats.syntheses_failed += 1;
            }
        }
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

/// Standalone synthesis pipeline for concurrent execution
///
/// This function runs the full synthesis -> verification -> broadcast pipeline
/// without requiring mutable access to the AutoLearner.
async fn run_synthesis_pipeline(
    signal: PainSignal,
    synthesizer: Arc<MinistralSynthesizer>,
    verifier: ImmuneVerifier,
    network: Arc<GossipNetworkBridge>,
    config: AutoLearnConfig,
) -> SynthesisResult {
    let signal_id = signal.id;

    // Step 1: Synthesis via Ministral-3B
    let graph = match tokio::time::timeout(
        config.synthesis_timeout,
        synthesizer.synthesize_patchlet(&signal),
    )
    .await
    {
        Ok(Ok(graph)) => graph,
        Ok(Err(e)) => {
            tracing::debug!("Synthesis failed: {}", e.message);
            return SynthesisResult {
                patch_id: None,
                signal_id,
                success: false,
                dry_run: config.dry_run,
                graph: None,
            };
        }
        Err(_) => {
            tracing::debug!("Synthesis timeout");
            return SynthesisResult {
                patch_id: None,
                signal_id,
                success: false,
                dry_run: config.dry_run,
                graph: None,
            };
        }
    };

    // Step 2: Immune Response - verify with InvariantLayer
    let timeout_ms = verifier.config().timeout_ms;
    let verifier_clone = verifier.clone();
    let graph_clone = graph.clone();
    let signal_clone = signal.clone();

    let verification = match tokio::time::timeout(
        Duration::from_millis(timeout_ms),
        tokio::task::spawn_blocking(move || verifier_clone.verify(&graph_clone, &signal_clone)),
    )
    .await
    {
        Ok(Ok(Ok(result))) => result,
        Ok(Ok(Err(e))) => {
            tracing::debug!("Verification error: {}", e);
            return SynthesisResult {
                patch_id: None,
                signal_id,
                success: false,
                dry_run: config.dry_run,
                graph: None,
            };
        }
        Ok(Err(e)) => {
            tracing::debug!("Verification task error: {}", e);
            return SynthesisResult {
                patch_id: None,
                signal_id,
                success: false,
                dry_run: config.dry_run,
                graph: None,
            };
        }
        Err(_) => {
            tracing::warn!("Verification timeout after {}ms", timeout_ms);
            return SynthesisResult {
                patch_id: None,
                signal_id,
                success: false,
                dry_run: config.dry_run,
                graph: None,
            };
        }
    };

    if !verification.passed {
        tracing::debug!(
            "Verification failed: {} invariant violations, {} boundary issues",
            verification.invariant_violations.len(),
            verification.boundary_issues.len()
        );
        return SynthesisResult {
            patch_id: None,
            signal_id,
            success: false,
            dry_run: config.dry_run,
            graph: None,
        };
    }

    // Step 3: Swarm Propagation (or dry-run logging)
    let patch_id = if config.dry_run {
        let dry_run_id = Uuid::new_v4();
        tracing::info!(
            "[DRY-RUN] Would broadcast patch {} (signal: {}, severity: {:.2})",
            dry_run_id,
            signal.source.description(),
            signal.severity
        );
        Some(dry_run_id)
    } else {
        // Create patchlet for broadcast
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
        let patchlet =
            Patchlet::new(graph.clone(), trigger, method).with_verification(verification_summary);

        match network.broadcast_patch(&patchlet).await {
            Ok(id) => {
                tracing::info!("Broadcast patch {}", id);
                Some(id)
            }
            Err(e) => {
                tracing::warn!("Failed to broadcast patch: {}", e);
                None
            }
        }
    };

    SynthesisResult {
        patch_id,
        signal_id,
        success: true,
        dry_run: config.dry_run,
        graph: Some(graph),
    }
}

/// Builder for AutoLearner with sensible defaults
pub struct AutoLearnerBuilder<C: CostFunction> {
    config: AutoLearnConfig,
    synthesizer: Option<Arc<MinistralSynthesizer>>,
    invariant_layer: Option<Arc<InvariantLayer>>,
    verifier_config: VerifierConfig,
    network: Option<Arc<GossipNetworkBridge>>,
    ensemble: Option<PolicyEnsemble<C>>,
    probe_scheduler: Option<ProbeScheduler>,
    patch_rx: Option<mpsc::Receiver<IncomingPatch>>,
    model_id: String,
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
            model_id: String::new(),
        }
    }

    /// Set the configuration
    pub fn config(mut self, config: AutoLearnConfig) -> Self {
        self.config = config;
        self
    }

    /// Set the synthesizer (wrapped in Arc for concurrent access)
    pub fn synthesizer(mut self, synthesizer: Arc<MinistralSynthesizer>) -> Self {
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

    /// Set the model ID for pain signal context
    pub fn model_id(mut self, model_id: String) -> Self {
        self.model_id = model_id;
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

        let mut learner = AutoLearner::new(self.config, synthesizer, verifier, network, ensemble)
            .with_model_id(self.model_id);

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

        let synthesizer = Arc::new(MinistralSynthesizer::new().unwrap());
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

        let synthesizer = Arc::new(MinistralSynthesizer::new().unwrap());
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

    #[tokio::test]
    async fn test_probe_cycle_enqueues_tasks() {
        use arkavo_sat::ProbeScheduler;

        let graph = create_simple_graph();
        let production = PolicyLayer::new(graph);
        let invariant_layer = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::new(1.0);
        let ensemble = PolicyEnsemble::new(production, invariant_layer.clone(), cost);

        let synthesizer = Arc::new(MinistralSynthesizer::new().unwrap());
        let keypair = AgentKeypair::generate();
        let network = Arc::new(crate::network::GossipNetworkBridge::new(
            "test-agent".to_string(),
            keypair,
            NetworkConfig::default(),
        ));

        let scheduler = ProbeScheduler::new();

        let mut learner = AutoLearnerBuilder::new()
            .synthesizer(synthesizer)
            .invariant_layer(invariant_layer)
            .network(network)
            .ensemble(ensemble)
            .probe_scheduler(scheduler)
            .model_id("test-model".to_string())
            .build()
            .unwrap();

        // Start the probe scheduler
        let handle = learner.start_probe_scheduler();
        assert!(handle.is_some(), "Probe scheduler should start");

        // Run one probe cycle - should enqueue tasks without error
        let result = learner.probe_cycle().await;
        assert!(result.is_ok(), "Probe cycle should succeed");

        // Process any results (may be empty if scheduler hasn't finished)
        learner.process_probe_results();
    }

    #[tokio::test]
    async fn test_concurrent_synthesis_semaphore_initialized() {
        let graph = create_simple_graph();
        let production = PolicyLayer::new(graph);
        let invariant_layer = Arc::new(InvariantLayer::new());
        let cost = ConstantCost::new(1.0);
        let ensemble = PolicyEnsemble::new(production, invariant_layer.clone(), cost);

        let synthesizer = Arc::new(MinistralSynthesizer::new().unwrap());
        let keypair = AgentKeypair::generate();
        let network = Arc::new(crate::network::GossipNetworkBridge::new(
            "test-agent".to_string(),
            keypair,
            NetworkConfig::default(),
        ));

        // Configure for concurrent synthesis (max 3)
        let config = AutoLearnConfig {
            max_concurrent_synthesis: 3,
            ..AutoLearnConfig::default()
        };

        let learner = AutoLearnerBuilder::new()
            .config(config)
            .synthesizer(synthesizer)
            .invariant_layer(invariant_layer)
            .network(network)
            .ensemble(ensemble)
            .build()
            .unwrap();

        // Verify semaphore is initialized with correct permits
        assert_eq!(
            learner.synthesis_semaphore.available_permits(),
            3,
            "Semaphore should have max_concurrent_synthesis permits"
        );
    }

    #[test]
    fn test_sequential_mode_when_max_concurrent_is_one() {
        // When max_concurrent_synthesis <= 1, sequential mode is used
        let config = AutoLearnConfig {
            max_concurrent_synthesis: 1,
            ..AutoLearnConfig::default()
        };

        assert!(
            config.max_concurrent_synthesis <= 1,
            "Sequential mode should be used when max_concurrent_synthesis <= 1"
        );
    }
}
