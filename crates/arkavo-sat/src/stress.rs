//! Policy stress testing using SAT solving
//!
//! Identifies potential policy holes by systematically exploring
//! the input space near decision boundaries.

use std::collections::HashMap;

use torg_core::Graph;

use crate::boundary::{BoundaryProbe, find_epsilon_boundary, find_satisfying_inputs};
use crate::error::{SatError, SatResult};

/// A policy hole - an input where the policy produces an unexpected result
#[derive(Debug, Clone)]
pub struct PolicyHole {
    /// Input assignment that triggers the hole
    pub input: HashMap<u16, bool>,
    /// Expected output value
    pub expected: bool,
    /// Actual output value
    pub actual: bool,
    /// Output node ID
    pub output_id: u16,
    /// Description of why this is unexpected
    pub description: String,
}

/// Result of stress testing a policy
#[derive(Debug, Clone)]
pub struct StressTestResult {
    /// Total inputs tested
    pub inputs_tested: usize,
    /// Boundary probes found
    pub boundary_probes: Vec<BoundaryProbe>,
    /// Policy holes discovered
    pub holes: Vec<PolicyHole>,
    /// Coverage statistics
    pub coverage: CoverageStats,
}

/// Coverage statistics from stress testing
#[derive(Debug, Clone, Default)]
pub struct CoverageStats {
    /// Number of input combinations where output is true
    pub true_cases: usize,
    /// Number of input combinations where output is false
    pub false_cases: usize,
    /// Inputs that are near decision boundaries
    pub boundary_inputs: usize,
    /// Maximum distance to flip seen
    pub max_flip_distance: u32,
}

/// Configuration for stress testing
#[derive(Debug, Clone)]
pub struct StressTestConfig {
    /// Maximum number of random inputs to test
    pub max_random_inputs: usize,
    /// Epsilon for boundary probing (max flips to consider)
    pub epsilon: u32,
    /// Whether to test all input combinations (only for small graphs)
    pub exhaustive: bool,
    /// Maximum inputs for exhaustive testing
    pub max_exhaustive_inputs: usize,
}

impl Default for StressTestConfig {
    fn default() -> Self {
        Self {
            max_random_inputs: 1000,
            epsilon: 3,
            exhaustive: true,
            max_exhaustive_inputs: 10,
        }
    }
}

/// Run stress testing on a policy graph
///
/// Systematically explores the input space to find boundary cases
/// and potential policy holes.
pub fn stress_test(
    graph: &Graph,
    output_id: u16,
    config: &StressTestConfig,
) -> SatResult<StressTestResult> {
    let mut result = StressTestResult {
        inputs_tested: 0,
        boundary_probes: Vec::new(),
        holes: Vec::new(),
        coverage: CoverageStats::default(),
    };

    // Find boundary probes using SAT solving
    result.boundary_probes = find_epsilon_boundary(graph, output_id, config.epsilon)?;

    for probe in &result.boundary_probes {
        result.coverage.boundary_inputs += 1;
        if probe.distance_to_flip > result.coverage.max_flip_distance {
            result.coverage.max_flip_distance = probe.distance_to_flip;
        }
    }

    // If small enough, test exhaustively
    let num_inputs = graph.inputs.len();
    if config.exhaustive && num_inputs <= config.max_exhaustive_inputs {
        exhaustive_test(graph, output_id, &mut result)?;
    } else {
        random_test(graph, output_id, config.max_random_inputs, &mut result)?;
    }

    Ok(result)
}

/// Exhaustively test all input combinations
fn exhaustive_test(graph: &Graph, output_id: u16, result: &mut StressTestResult) -> SatResult<()> {
    let num_inputs = graph.inputs.len();
    let total_combinations = 1usize << num_inputs;

    for i in 0..total_combinations {
        let inputs = build_input_map(graph, i);
        test_input(graph, output_id, &inputs, result)?;
    }

    Ok(())
}

/// Test random input combinations
fn random_test(
    graph: &Graph,
    output_id: u16,
    max_inputs: usize,
    result: &mut StressTestResult,
) -> SatResult<()> {
    use std::collections::HashSet;

    let num_inputs = graph.inputs.len();
    let mut tested = HashSet::new();

    // Always test all-false and all-true first
    let all_false = build_input_map(graph, 0);
    test_input(graph, output_id, &all_false, result)?;
    tested.insert(0usize);

    let all_true = build_input_map(graph, (1 << num_inputs) - 1);
    test_input(graph, output_id, &all_true, result)?;
    tested.insert((1 << num_inputs) - 1);

    // Random sampling
    let mut rng_state = 42u64; // Simple LCG for deterministic testing
    for _ in 0..max_inputs.saturating_sub(2) {
        rng_state = rng_state.wrapping_mul(6364136223846793005).wrapping_add(1);
        let idx = (rng_state as usize) % (1 << num_inputs);

        if !tested.contains(&idx) {
            tested.insert(idx);
            let inputs = build_input_map(graph, idx);
            test_input(graph, output_id, &inputs, result)?;
        }
    }

    Ok(())
}

/// Build an input map from a bit pattern
fn build_input_map(graph: &Graph, pattern: usize) -> HashMap<u16, bool> {
    let mut inputs = HashMap::new();
    for (bit, &input_id) in graph.inputs.iter().enumerate() {
        inputs.insert(input_id, (pattern >> bit) & 1 == 1);
    }
    inputs
}

/// Test a single input combination
fn test_input(
    graph: &Graph,
    output_id: u16,
    inputs: &HashMap<u16, bool>,
    result: &mut StressTestResult,
) -> SatResult<()> {
    result.inputs_tested += 1;

    let outputs =
        torg_core::evaluate(graph, inputs).map_err(|e| SatError::Solver(e.to_string()))?;

    match outputs.get(&output_id) {
        Some(&true) => result.coverage.true_cases += 1,
        Some(&false) => result.coverage.false_cases += 1,
        None => {
            return Err(SatError::InvalidGraph(format!(
                "Output {} not found",
                output_id
            )));
        }
    }

    Ok(())
}

/// Find contradictions between the policy and expected behavior
///
/// Takes a specification function that defines expected outputs and
/// finds inputs where the policy disagrees.
pub fn find_contradictions<F>(
    graph: &Graph,
    output_id: u16,
    expected_fn: F,
    config: &StressTestConfig,
) -> SatResult<Vec<PolicyHole>>
where
    F: Fn(&HashMap<u16, bool>) -> Option<bool>,
{
    let mut holes = Vec::new();

    let num_inputs = graph.inputs.len();

    if config.exhaustive && num_inputs <= config.max_exhaustive_inputs {
        let total = 1usize << num_inputs;
        for i in 0..total {
            let inputs = build_input_map(graph, i);
            if let Some(hole) = check_contradiction(graph, output_id, &inputs, &expected_fn)? {
                holes.push(hole);
            }
        }
    } else {
        // Test boundary cases which are most likely to have issues
        let boundary_probes = find_epsilon_boundary(graph, output_id, config.epsilon)?;
        for probe in boundary_probes {
            if let Some(hole) =
                check_contradiction(graph, output_id, &probe.input_assignment, &expected_fn)?
            {
                holes.push(hole);
            }
        }
    }

    Ok(holes)
}

/// Check a single input for contradictions
fn check_contradiction<F>(
    graph: &Graph,
    output_id: u16,
    inputs: &HashMap<u16, bool>,
    expected_fn: &F,
) -> SatResult<Option<PolicyHole>>
where
    F: Fn(&HashMap<u16, bool>) -> Option<bool>,
{
    let outputs =
        torg_core::evaluate(graph, inputs).map_err(|e| SatError::Solver(e.to_string()))?;

    let actual = outputs
        .get(&output_id)
        .ok_or_else(|| SatError::InvalidGraph(format!("Output {} not found", output_id)))?;

    if let Some(expected) = expected_fn(inputs)
        && expected != *actual {
            return Ok(Some(PolicyHole {
                input: inputs.clone(),
                expected,
                actual: *actual,
                output_id,
                description: format!(
                    "Policy returned {} but expected {} for inputs {:?}",
                    actual, expected, inputs
                ),
            }));
        }

    Ok(None)
}

/// Check if a graph is satisfiable (has at least one valid input)
pub fn is_satisfiable(graph: &Graph, output_id: u16, target_value: bool) -> SatResult<bool> {
    Ok(find_satisfying_inputs(graph, output_id, target_value)?.is_some())
}

/// Check if a graph is a tautology (always produces target value)
pub fn is_tautology(graph: &Graph, output_id: u16, target_value: bool) -> SatResult<bool> {
    // If we can't find an input that produces the opposite value,
    // then the graph always produces the target value
    Ok(find_satisfying_inputs(graph, output_id, !target_value)?.is_none())
}

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::{Builder, Token};

    fn create_or_graph() -> Graph {
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn test_stress_test_basic() {
        let graph = create_or_graph();
        let config = StressTestConfig::default();

        let result = stress_test(&graph, 2, &config).unwrap();

        // Should have tested all 4 combinations
        assert_eq!(result.inputs_tested, 4);

        // OR: 3 true cases, 1 false case
        assert_eq!(result.coverage.true_cases, 3);
        assert_eq!(result.coverage.false_cases, 1);
    }

    #[test]
    fn test_stress_test_boundary_probes() {
        let graph = create_or_graph();
        let config = StressTestConfig {
            epsilon: 1,
            ..Default::default()
        };

        let result = stress_test(&graph, 2, &config).unwrap();
        assert!(!result.boundary_probes.is_empty());
    }

    #[test]
    fn test_find_contradictions() {
        let graph = create_or_graph();
        let config = StressTestConfig::default();

        // Define correct OR behavior - no contradictions
        let correct_or = |inputs: &HashMap<u16, bool>| -> Option<bool> {
            let a = inputs.get(&0).copied()?;
            let b = inputs.get(&1).copied()?;
            Some(a || b)
        };

        let holes = find_contradictions(&graph, 2, correct_or, &config).unwrap();
        assert!(holes.is_empty(), "OR graph should match OR spec");

        // Define incorrect spec - should find contradictions
        let wrong_spec = |inputs: &HashMap<u16, bool>| -> Option<bool> {
            let a = inputs.get(&0).copied()?;
            let b = inputs.get(&1).copied()?;
            Some(a && b) // AND instead of OR
        };

        let holes = find_contradictions(&graph, 2, wrong_spec, &config).unwrap();
        assert!(!holes.is_empty(), "OR graph should not match AND spec");
    }

    #[test]
    fn test_is_satisfiable() {
        let graph = create_or_graph();

        // OR can produce both true and false
        assert!(is_satisfiable(&graph, 2, true).unwrap());
        assert!(is_satisfiable(&graph, 2, false).unwrap());
    }

    #[test]
    fn test_is_tautology() {
        let graph = create_or_graph();

        // OR is not a tautology for either value
        assert!(!is_tautology(&graph, 2, true).unwrap());
        assert!(!is_tautology(&graph, 2, false).unwrap());
    }

    #[test]
    fn test_coverage_stats() {
        let graph = create_or_graph();
        let config = StressTestConfig::default();

        let result = stress_test(&graph, 2, &config).unwrap();

        assert_eq!(
            result.coverage.true_cases + result.coverage.false_cases,
            result.inputs_tested
        );
    }
}
