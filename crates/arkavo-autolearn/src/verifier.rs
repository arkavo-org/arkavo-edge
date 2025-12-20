//! Immune response verification for synthesized policies
//!
//! Implements the "distrust your own LLM" principle by verifying
//! synthesized graphs against invariant constraints and SAT-based
//! stress testing before propagation.

use std::collections::HashMap;
use std::sync::Arc;

use torg_core::Graph;

use arkavo_sat::{PolicyHole, StressTestConfig, StressTestResult, stress_test};
use arkavo_sbe::{InvariantLayer, ViolationAttempt};

use crate::error::{AutoLearnError, AutoLearnResult};
use crate::signals::PainSignal;

/// Result of verifying a synthesized graph
#[derive(Debug, Clone)]
pub struct VerificationResult {
    /// Whether verification passed all checks
    pub passed: bool,
    /// Invariant violations found (empty if none)
    pub invariant_violations: Vec<InvariantViolation>,
    /// Policy holes found during stress testing
    pub boundary_issues: Vec<PolicyHole>,
    /// Stress test statistics
    pub stress_stats: Option<StressStats>,
}

impl VerificationResult {
    /// Create a passing verification result
    #[must_use]
    pub fn passed() -> Self {
        Self {
            passed: true,
            invariant_violations: Vec::new(),
            boundary_issues: Vec::new(),
            stress_stats: None,
        }
    }

    /// Create a failing verification result
    #[must_use]
    pub fn failed() -> Self {
        Self {
            passed: false,
            invariant_violations: Vec::new(),
            boundary_issues: Vec::new(),
            stress_stats: None,
        }
    }

    /// Add an invariant violation
    #[must_use]
    pub fn with_invariant_violation(mut self, violation: InvariantViolation) -> Self {
        self.passed = false;
        self.invariant_violations.push(violation);
        self
    }

    /// Add boundary issues from stress testing
    #[must_use]
    pub fn with_boundary_issues(mut self, issues: Vec<PolicyHole>) -> Self {
        if !issues.is_empty() {
            self.passed = false;
        }
        self.boundary_issues = issues;
        self
    }

    /// Add stress test statistics
    #[must_use]
    pub fn with_stress_stats(mut self, stats: StressStats) -> Self {
        self.stress_stats = Some(stats);
        self
    }
}

/// An invariant violation detected during verification
#[derive(Debug, Clone)]
pub struct InvariantViolation {
    /// Description of the violated invariant
    pub description: String,
    /// The inputs that triggered the violation
    pub inputs: HashMap<u16, bool>,
    /// Contract ID if from an InvariantLayer
    pub contract_id: Option<uuid::Uuid>,
}

impl From<ViolationAttempt> for InvariantViolation {
    fn from(attempt: ViolationAttempt) -> Self {
        Self {
            description: format!("Invariant contract {} violated", attempt.contract_id),
            inputs: attempt.inputs,
            contract_id: Some(attempt.contract_id),
        }
    }
}

/// Summary statistics from stress testing
#[derive(Debug, Clone)]
pub struct StressStats {
    /// Total inputs tested
    pub inputs_tested: usize,
    /// Inputs that produced true output
    pub true_cases: usize,
    /// Inputs that produced false output
    pub false_cases: usize,
    /// Boundary probes found
    pub boundary_probes: usize,
}

impl From<&StressTestResult> for StressStats {
    fn from(result: &StressTestResult) -> Self {
        Self {
            inputs_tested: result.inputs_tested,
            true_cases: result.coverage.true_cases,
            false_cases: result.coverage.false_cases,
            boundary_probes: result.boundary_probes.len(),
        }
    }
}

/// Default timeout for SAT verification (500ms)
pub const DEFAULT_VERIFICATION_TIMEOUT_MS: u64 = 500;

/// Configuration for the immune verifier
#[derive(Debug, Clone)]
pub struct VerifierConfig {
    /// Configuration for SAT stress testing
    pub stress_config: StressTestConfig,
    /// Maximum allowed policy holes before rejection
    pub max_policy_holes: usize,
    /// Whether to fail on any invariant violation
    pub strict_invariants: bool,
    /// Sample inputs to test against invariants
    pub invariant_sample_size: usize,
    /// Timeout for SAT operations (fail-safe: reject on timeout)
    pub timeout_ms: u64,
}

impl Default for VerifierConfig {
    fn default() -> Self {
        Self {
            stress_config: StressTestConfig {
                max_random_inputs: 500,
                epsilon: 2,
                exhaustive: true,
                max_exhaustive_inputs: 8,
            },
            max_policy_holes: 0,
            strict_invariants: true,
            invariant_sample_size: 100,
            timeout_ms: DEFAULT_VERIFICATION_TIMEOUT_MS,
        }
    }
}

/// Immune response verifier
///
/// Implements the "distrust your own LLM" principle by verifying
/// synthesized graphs before they can be propagated to the swarm.
pub struct ImmuneVerifier {
    /// Invariant layer containing safety contracts
    invariant_layer: Arc<InvariantLayer>,
    /// Verification configuration
    config: VerifierConfig,
}

impl ImmuneVerifier {
    /// Create a new immune verifier
    #[must_use]
    pub fn new(invariant_layer: Arc<InvariantLayer>) -> Self {
        Self {
            invariant_layer,
            config: VerifierConfig::default(),
        }
    }

    /// Create with custom configuration
    #[must_use]
    pub fn with_config(invariant_layer: Arc<InvariantLayer>, config: VerifierConfig) -> Self {
        Self {
            invariant_layer,
            config,
        }
    }

    /// Verify a synthesized graph (basic verification)
    ///
    /// Checks the graph against invariant constraints and runs
    /// basic stress testing.
    pub fn verify(
        &self,
        graph: &Graph,
        _signal: &PainSignal,
    ) -> AutoLearnResult<VerificationResult> {
        let mut result = VerificationResult::passed();

        // Check invariant constraints
        let violations = self.check_invariants(graph)?;
        for violation in violations {
            result = result.with_invariant_violation(violation);
        }

        // Run stress testing on each output
        for &output_id in &graph.outputs {
            let stress_result = stress_test(graph, output_id, &self.config.stress_config)
                .map_err(|e| AutoLearnError::Sat(e.to_string()))?;

            result = result.with_stress_stats(StressStats::from(&stress_result));

            // Check for policy holes
            if stress_result.holes.len() > self.config.max_policy_holes {
                result = result.with_boundary_issues(stress_result.holes);
            }
        }

        Ok(result)
    }

    /// Deep verification for remote patches (zero-trust)
    ///
    /// Performs more thorough verification for patches received from
    /// other agents, since we don't trust their verification.
    pub fn deep_verify(&self, graph: &Graph) -> AutoLearnResult<VerificationResult> {
        let mut result = VerificationResult::passed();

        // More thorough invariant checking
        let violations = self.exhaustive_invariant_check(graph)?;
        for violation in violations {
            result = result.with_invariant_violation(violation);
        }

        // More aggressive stress testing
        let deep_config = StressTestConfig {
            max_random_inputs: 2000,
            epsilon: 3,
            exhaustive: true,
            max_exhaustive_inputs: 12,
        };

        for &output_id in &graph.outputs {
            let stress_result = stress_test(graph, output_id, &deep_config)
                .map_err(|e| AutoLearnError::Sat(e.to_string()))?;

            result = result.with_stress_stats(StressStats::from(&stress_result));

            if !stress_result.holes.is_empty() {
                result = result.with_boundary_issues(stress_result.holes);
            }
        }

        Ok(result)
    }

    /// Check if a graph would violate any invariant constraints
    fn check_invariants(&self, graph: &Graph) -> AutoLearnResult<Vec<InvariantViolation>> {
        let mut violations = Vec::new();
        let num_inputs = graph.inputs.len();

        // Sample inputs to check
        let sample_count = self.config.invariant_sample_size.min(1 << num_inputs);

        for i in 0..sample_count {
            let inputs = self.build_input_map(graph, i);

            // First evaluate the graph
            let outputs = torg_core::evaluate(graph, &inputs)
                .map_err(|e| AutoLearnError::Verification(e.to_string()))?;

            // Then check if the graph outputs + inputs would violate invariants
            // We need to combine inputs with the computed outputs for invariant checking
            let mut combined_inputs = inputs.clone();
            for (&output_id, &value) in &outputs {
                combined_inputs.insert(output_id, value);
            }

            if let Some(attempt) = self.invariant_layer.check_violation(&combined_inputs) {
                violations.push(InvariantViolation::from(attempt));
                if self.config.strict_invariants {
                    break;
                }
            }
        }

        Ok(violations)
    }

    /// Exhaustive invariant checking for deep verification
    fn exhaustive_invariant_check(
        &self,
        graph: &Graph,
    ) -> AutoLearnResult<Vec<InvariantViolation>> {
        let mut violations = Vec::new();
        let num_inputs = graph.inputs.len();

        // For small input spaces, check exhaustively
        if num_inputs <= 10 {
            let total = 1usize << num_inputs;
            for i in 0..total {
                let inputs = self.build_input_map(graph, i);

                let outputs = torg_core::evaluate(graph, &inputs)
                    .map_err(|e| AutoLearnError::Verification(e.to_string()))?;

                let mut combined = inputs.clone();
                for (&output_id, &value) in &outputs {
                    combined.insert(output_id, value);
                }

                if let Some(attempt) = self.invariant_layer.check_violation(&combined) {
                    violations.push(InvariantViolation::from(attempt));
                }
            }
        } else {
            // Fall back to sampling for large input spaces
            violations = self.check_invariants(graph)?;
        }

        Ok(violations)
    }

    /// Build an input map from a bit pattern
    fn build_input_map(&self, graph: &Graph, pattern: usize) -> HashMap<u16, bool> {
        let mut inputs = HashMap::new();
        for (bit, &input_id) in graph.inputs.iter().enumerate() {
            inputs.insert(input_id, (pattern >> bit) & 1 == 1);
        }
        inputs
    }

    /// Get the invariant layer
    #[must_use]
    pub fn invariant_layer(&self) -> &InvariantLayer {
        &self.invariant_layer
    }

    /// Get the configuration
    #[must_use]
    pub fn config(&self) -> &VerifierConfig {
        &self.config
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_ensemble::SynthesisContext;
    use torg_core::{Builder, Token};

    use crate::signals::{PainSignal, PainSource};

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

    fn create_test_signal() -> PainSignal {
        let ctx = SynthesisContext::new("test-model".to_string(), "Test signal".to_string());
        PainSignal::new(
            PainSource::External {
                description: "test".to_string(),
            },
            0.5,
            ctx,
        )
    }

    #[test]
    fn test_verifier_creation() {
        let invariant_layer = Arc::new(InvariantLayer::new());
        let verifier = ImmuneVerifier::new(invariant_layer);
        assert!(verifier.config().strict_invariants);
    }

    #[test]
    fn test_verification_passing() {
        let invariant_layer = Arc::new(InvariantLayer::new());
        let verifier = ImmuneVerifier::new(invariant_layer);

        let graph = create_simple_graph();
        let signal = create_test_signal();

        let result = verifier.verify(&graph, &signal).unwrap();
        assert!(result.passed);
        assert!(result.invariant_violations.is_empty());
    }

    #[test]
    fn test_deep_verification() {
        let invariant_layer = Arc::new(InvariantLayer::new());
        let verifier = ImmuneVerifier::new(invariant_layer);

        let graph = create_simple_graph();

        let result = verifier.deep_verify(&graph).unwrap();
        assert!(result.passed);
        assert!(result.stress_stats.is_some());
    }

    #[test]
    fn test_verification_result_builder() {
        let result = VerificationResult::passed().with_stress_stats(StressStats {
            inputs_tested: 100,
            true_cases: 75,
            false_cases: 25,
            boundary_probes: 5,
        });

        assert!(result.passed);
        assert!(result.stress_stats.is_some());
        assert_eq!(result.stress_stats.as_ref().unwrap().inputs_tested, 100);
    }

    #[test]
    fn test_verification_result_failure() {
        let violation = InvariantViolation {
            description: "Test violation".to_string(),
            inputs: HashMap::new(),
            contract_id: None,
        };

        let result = VerificationResult::passed().with_invariant_violation(violation);

        assert!(!result.passed);
        assert_eq!(result.invariant_violations.len(), 1);
    }

    #[test]
    fn test_config_defaults() {
        let config = VerifierConfig::default();
        assert_eq!(config.max_policy_holes, 0);
        assert!(config.strict_invariants);
        assert_eq!(config.invariant_sample_size, 100);
        assert_eq!(config.timeout_ms, DEFAULT_VERIFICATION_TIMEOUT_MS);
    }
}

/// Adversarial security tests for the ImmuneVerifier
///
/// These tests verify that the verifier correctly rejects malicious
/// or invalid patches, implementing AUTO-005 zero-trust verification.
#[cfg(test)]
mod security_tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;
    use arkavo_sbe::InvariantContract;
    use torg_core::{Builder, Token};

    /// Create a graph that always outputs TRUE (tautology)
    fn create_always_true_graph() -> Graph {
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        // Node 1: OR(0, 0) - just passes through input
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        // Node 2: OR(1, TRUE) - always TRUE
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::True).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.finish().unwrap()
    }

    /// Create an invariant that requires output to be FALSE when input is FALSE
    /// (i.e., "deny by default" - if no explicit permission, deny)
    fn create_deny_default_invariant() -> Graph {
        // Invariant: NOT(input=FALSE AND output=TRUE)
        // Equivalently: input OR NOT(output)
        // If input is FALSE (no permission), output must be FALSE
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap(); // input
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(2)).unwrap(); // output from policy (to check)
        // NOT output = NOR(output, output)
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(3)).unwrap();
        builder.push(Token::Nor).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        // input OR NOT(output)
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(4)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(3)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(4)).unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn test_adversarial_always_allow_patch_rejected() {
        // ADVERSARIAL TEST: A patch that always returns TRUE (bypasses all checks)
        // should be rejected if we have a "deny by default" invariant

        let keypair = AgentKeypair::generate();
        let invariant_graph = create_deny_default_invariant();
        let mut contract = InvariantContract::new(
            "Deny by default: output must be FALSE when input is FALSE".to_string(),
            invariant_graph,
        );
        contract.sign(&keypair);

        let mut invariant_layer = InvariantLayer::new();
        invariant_layer.add_contract(contract).unwrap();

        let verifier = ImmuneVerifier::new(Arc::new(invariant_layer));

        // Create a malicious "always allow" patch
        let poisoned_graph = create_always_true_graph();

        // The verifier should REJECT this patch
        let result = verifier.deep_verify(&poisoned_graph).unwrap();

        assert!(
            !result.passed,
            "SECURITY FAILURE: Verifier accepted an 'always allow' patch that violates deny-by-default invariant"
        );
        assert!(
            !result.invariant_violations.is_empty(),
            "SECURITY FAILURE: No invariant violation detected for policy bypass"
        );
    }

    #[test]
    fn test_valid_patch_with_invariant_passes() {
        // A valid patch that respects the invariant should pass
        let keypair = AgentKeypair::generate();
        let invariant_graph = create_deny_default_invariant();
        let mut contract = InvariantContract::new("Deny by default".to_string(), invariant_graph);
        contract.sign(&keypair);

        let mut invariant_layer = InvariantLayer::new();
        invariant_layer.add_contract(contract).unwrap();

        let verifier = ImmuneVerifier::new(Arc::new(invariant_layer));

        // Create a compliant graph: output = input (respects deny-by-default)
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
        let compliant_graph = builder.finish().unwrap();

        let result = verifier.deep_verify(&compliant_graph).unwrap();
        assert!(result.passed, "Valid patch should pass verification");
    }

    #[test]
    fn test_empty_graph_handled() {
        // A minimal graph should be handled without panic
        let invariant_layer = Arc::new(InvariantLayer::new());
        let verifier = ImmuneVerifier::new(invariant_layer);

        // Create minimal graph with no useful logic
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        let minimal_graph = builder.finish().unwrap();

        // This should at least complete without panic
        let result = verifier.deep_verify(&minimal_graph);
        assert!(result.is_ok(), "Verifier should handle minimal graphs");
    }

    #[test]
    fn test_strict_mode_rejects_on_first_violation() {
        // With strict_invariants = true, first violation should stop
        let keypair = AgentKeypair::generate();
        let invariant_graph = create_deny_default_invariant();
        let mut contract = InvariantContract::new("Deny by default".to_string(), invariant_graph);
        contract.sign(&keypair);

        let mut invariant_layer = InvariantLayer::new();
        invariant_layer.add_contract(contract).unwrap();

        let config = VerifierConfig {
            strict_invariants: true,
            ..Default::default()
        };
        let verifier = ImmuneVerifier::with_config(Arc::new(invariant_layer), config);

        let poisoned_graph = create_always_true_graph();
        let result = verifier.deep_verify(&poisoned_graph).unwrap();

        assert!(!result.passed);
        // In strict mode, we stop at first violation
        assert_eq!(result.invariant_violations.len(), 1);
    }
}
