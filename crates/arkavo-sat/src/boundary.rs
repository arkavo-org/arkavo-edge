//! Boundary probing for policy stress testing
//!
//! Generates synthetic inputs near decision boundaries using SAT solving.
//! Uses cardinality constraints for efficient k-flip queries without
//! brute-force enumeration.

use std::collections::HashMap;

use torg_core::Graph;
use varisat::{ExtendFormula, Lit, Solver};

use crate::cardinality::{create_flip_indicators, encode_at_most_k};
use crate::cnf::{CnfFormula, extract_cnf};
use crate::error::{SatError, SatResult};

/// A probe result showing an input near a decision boundary
#[derive(Debug, Clone)]
pub struct BoundaryProbe {
    /// Input assignment that produces a specific output
    pub input_assignment: HashMap<u16, bool>,
    /// The output node ID being probed
    pub output_id: u16,
    /// The output value for this assignment
    pub output_value: bool,
    /// Number of input flips needed to change the output
    pub distance_to_flip: u32,
}

/// Find inputs that satisfy the graph with a specific output value
pub fn find_satisfying_inputs(
    graph: &Graph,
    output_id: u16,
    output_value: bool,
) -> SatResult<Option<HashMap<u16, bool>>> {
    let cnf = extract_cnf(graph)?;

    let mut solver = Solver::new();
    for clause in &cnf.clauses {
        solver.add_clause(clause);
    }

    // Add constraint for desired output value
    if let Some(&output_lit) = cnf.variable_map.get(&output_id) {
        let constraint = if output_value {
            output_lit
        } else {
            !output_lit
        };
        solver.add_clause(&[constraint]);
    } else {
        return Err(SatError::InvalidGraph(format!(
            "Output {} not found in graph",
            output_id
        )));
    }

    // Solve
    match solver.solve() {
        Ok(true) => {
            let model = solver
                .model()
                .ok_or_else(|| SatError::Solver("No model available after SAT".into()))?;

            let mut assignment = HashMap::new();
            for &input_id in &graph.inputs {
                if let Some(&lit) = cnf.variable_map.get(&input_id) {
                    let var = lit.var();
                    let value = model
                        .iter()
                        .find(|l| l.var() == var)
                        .map(|l| l.is_positive())
                        .unwrap_or(false);
                    assignment.insert(input_id, value);
                }
            }
            Ok(Some(assignment))
        }
        Ok(false) => Ok(None),
        Err(e) => Err(SatError::Solver(e.to_string())),
    }
}

/// Find boundary probes for a graph output
///
/// Returns inputs where the output is true and false, allowing
/// analysis of the decision boundary.
pub fn probe_boundary(graph: &Graph, output_id: u16) -> SatResult<Vec<BoundaryProbe>> {
    let mut probes = Vec::new();

    // Find input where output is true
    if let Some(true_inputs) = find_satisfying_inputs(graph, output_id, true)? {
        probes.push(BoundaryProbe {
            input_assignment: true_inputs,
            output_id,
            output_value: true,
            distance_to_flip: 0, // Will be computed if needed
        });
    }

    // Find input where output is false
    if let Some(false_inputs) = find_satisfying_inputs(graph, output_id, false)? {
        probes.push(BoundaryProbe {
            input_assignment: false_inputs,
            output_id,
            output_value: false,
            distance_to_flip: 0,
        });
    }

    Ok(probes)
}

/// Find inputs within epsilon distance of a decision boundary
///
/// Uses SAT solving to find inputs where flipping at most `epsilon`
/// input bits would change the output.
pub fn find_epsilon_boundary(
    graph: &Graph,
    output_id: u16,
    epsilon: u32,
) -> SatResult<Vec<BoundaryProbe>> {
    let cnf = extract_cnf(graph)?;
    let mut probes = Vec::new();

    // For each target output value
    for target_output in [true, false] {
        // Find a satisfying input
        if let Some(base_inputs) = find_satisfying_inputs(graph, output_id, target_output)? {
            // Check if flipping any single input changes the output
            let distance = compute_flip_distance(
                graph,
                &cnf,
                &base_inputs,
                output_id,
                !target_output,
                epsilon,
            )?;

            if distance <= epsilon {
                probes.push(BoundaryProbe {
                    input_assignment: base_inputs,
                    output_id,
                    output_value: target_output,
                    distance_to_flip: distance,
                });
            }
        }
    }

    Ok(probes)
}

/// Compute minimum number of input flips to change output
fn compute_flip_distance(
    graph: &Graph,
    cnf: &CnfFormula,
    base_inputs: &HashMap<u16, bool>,
    output_id: u16,
    target_output: bool,
    max_distance: u32,
) -> SatResult<u32> {
    // Try flipping 1, 2, ... inputs until we find a solution
    for distance in 1..=max_distance {
        if can_flip_to_target(graph, cnf, base_inputs, output_id, target_output, distance)? {
            return Ok(distance);
        }
    }

    Ok(max_distance + 1)
}

/// Check if we can reach target output by flipping at most `k` inputs
///
/// Uses SAT solver with cardinality constraints for efficient search.
/// This removes the k≤3, n≤10 limitations of the brute-force approach.
fn can_flip_to_target(
    graph: &Graph,
    cnf: &CnfFormula,
    base_inputs: &HashMap<u16, bool>,
    output_id: u16,
    target_output: bool,
    k: u32,
) -> SatResult<bool> {
    can_flip_to_target_sat(cnf, base_inputs, output_id, target_output, k, &graph.inputs)
        .map(|opt| opt.is_some())
}

/// SAT-based k-flip query using cardinality constraints
///
/// Returns the flipped input assignment if one exists, None otherwise.
///
/// # Algorithm
///
/// 1. Create fresh solver with base CNF
/// 2. Create flip indicator variables for each input
/// 3. Add cardinality constraint: at_most_k(flip_indicators, k)
/// 4. For each input: constrain CNF input = base_value XOR flip_indicator
/// 5. Assert output = target
/// 6. Solve and extract final input assignment from CNF variables
pub fn can_flip_to_target_sat(
    cnf: &CnfFormula,
    base_inputs: &HashMap<u16, bool>,
    output_id: u16,
    target_output: bool,
    k: u32,
    input_ids: &[u16],
) -> SatResult<Option<HashMap<u16, bool>>> {
    let mut solver = Solver::new();

    // Add base CNF clauses
    for clause in &cnf.clauses {
        solver.add_clause(clause);
    }

    // Create flip indicator variables
    let flip_indicators = create_flip_indicators(&mut solver, input_ids.len());

    // Add cardinality constraint: at most k flips
    encode_at_most_k(&mut solver, &flip_indicators, k as usize)?;

    // Collect input literals for later extraction
    let mut input_lits: Vec<(u16, Lit)> = Vec::with_capacity(input_ids.len());

    for (i, &input_id) in input_ids.iter().enumerate() {
        let base_value = *base_inputs.get(&input_id).unwrap_or(&false);

        // Get the original input literal from CNF
        let Some(&input_lit) = cnf.variable_map.get(&input_id) else {
            return Err(SatError::InvalidGraph(format!(
                "Input {} not found in CNF variable map",
                input_id
            )));
        };
        input_lits.push((input_id, input_lit));

        // Encode: input_lit = base_value XOR flip_indicator
        //
        // If base_value = true:
        //   flip=false -> input=true:  (¬flip → input)  = (flip ∨ input)
        //   flip=true  -> input=false: (flip → ¬input)  = (¬flip ∨ ¬input)
        //   Combined: input ↔ ¬flip
        //
        // If base_value = false:
        //   flip=false -> input=false: (¬flip → ¬input) = (flip ∨ ¬input)
        //   flip=true  -> input=true:  (flip → input)   = (¬flip ∨ input)
        //   Combined: input ↔ flip
        let flip = flip_indicators[i];
        if base_value {
            // input ↔ ¬flip: (input ∨ flip) ∧ (¬input ∨ ¬flip)
            solver.add_clause(&[input_lit, flip]);
            solver.add_clause(&[!input_lit, !flip]);
        } else {
            // input ↔ flip: (input ∨ ¬flip) ∧ (¬input ∨ flip)
            solver.add_clause(&[input_lit, !flip]);
            solver.add_clause(&[!input_lit, flip]);
        }
    }

    // Add constraint for desired output value
    if let Some(&output_lit) = cnf.variable_map.get(&output_id) {
        if target_output {
            solver.add_clause(&[output_lit]);
        } else {
            solver.add_clause(&[!output_lit]);
        }
    } else {
        return Err(SatError::InvalidGraph(format!(
            "Output {} not found in CNF",
            output_id
        )));
    }

    // Solve
    match solver.solve() {
        Ok(true) => {
            let model = solver
                .model()
                .ok_or_else(|| SatError::Solver("No model available after SAT".into()))?;

            // Extract input assignment from CNF input variables
            let mut assignment = HashMap::new();
            for (input_id, input_lit) in input_lits {
                let var = input_lit.var();
                let value = model
                    .iter()
                    .find(|l| l.var() == var)
                    .map(|l| l.is_positive())
                    .unwrap_or(false);
                assignment.insert(input_id, value);
            }
            Ok(Some(assignment))
        }
        Ok(false) => Ok(None),
        Err(e) => Err(SatError::Solver(e.to_string())),
    }
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

    fn create_many_input_graph() -> Graph {
        // Create a graph with 15 inputs: output = OR(all inputs)
        let mut builder = Builder::new();

        for i in 0..15 {
            builder.push(Token::InputDecl).unwrap();
            builder.push(Token::Id(i)).unwrap();
        }

        // Chain of OR gates: n15 = 0 OR 1, n16 = n15 OR 2, ...
        let mut prev = 0;
        for i in 1..15 {
            let node_id = 15 + i as u16;
            builder.push(Token::NodeStart).unwrap();
            builder.push(Token::Id(node_id)).unwrap();
            builder.push(Token::Or).unwrap();
            if i == 1 {
                builder.push(Token::Id(0)).unwrap();
            } else {
                builder.push(Token::Id(prev)).unwrap();
            }
            builder.push(Token::Id(i as u16)).unwrap();
            builder.push(Token::NodeEnd).unwrap();
            prev = node_id;
        }

        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(prev)).unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn test_find_satisfying_true() {
        let graph = create_or_graph();
        let result = find_satisfying_inputs(&graph, 2, true).unwrap();

        assert!(result.is_some());
        let inputs = result.unwrap();

        // For OR, at least one input should be true
        let a = *inputs.get(&0).unwrap_or(&false);
        let b = *inputs.get(&1).unwrap_or(&false);
        assert!(a || b);
    }

    #[test]
    fn test_find_satisfying_false() {
        let graph = create_or_graph();
        let result = find_satisfying_inputs(&graph, 2, false).unwrap();

        assert!(result.is_some());
        let inputs = result.unwrap();

        // For OR to be false, both inputs must be false
        let a = *inputs.get(&0).unwrap_or(&true);
        let b = *inputs.get(&1).unwrap_or(&true);
        assert!(!a && !b);
    }

    #[test]
    fn test_probe_boundary() {
        let graph = create_or_graph();
        let probes = probe_boundary(&graph, 2).unwrap();

        // Should find both true and false outputs
        assert_eq!(probes.len(), 2);

        let has_true = probes.iter().any(|p| p.output_value);
        let has_false = probes.iter().any(|p| !p.output_value);
        assert!(has_true);
        assert!(has_false);
    }

    #[test]
    fn test_sat_based_flip_single() {
        let graph = create_or_graph();
        let cnf = extract_cnf(&graph).unwrap();

        // Start with all false (output = false)
        let mut base_inputs = HashMap::new();
        base_inputs.insert(0, false);
        base_inputs.insert(1, false);

        // Can we flip 1 input to get output = true?
        let result =
            can_flip_to_target_sat(&cnf, &base_inputs, 2, true, 1, &graph.inputs).unwrap();

        assert!(result.is_some());
        let flipped = result.unwrap();

        // Exactly one should be true now
        let a = *flipped.get(&0).unwrap_or(&false);
        let b = *flipped.get(&1).unwrap_or(&false);
        assert!(a ^ b, "Expected exactly one input flipped");
    }

    #[test]
    fn test_sat_based_flip_impossible() {
        let graph = create_or_graph();
        let cnf = extract_cnf(&graph).unwrap();

        // Start with all true (output = true for OR)
        let mut base_inputs = HashMap::new();
        base_inputs.insert(0, true);
        base_inputs.insert(1, true);

        // Can we flip 1 input to get output = false? No, need to flip both
        let result =
            can_flip_to_target_sat(&cnf, &base_inputs, 2, false, 1, &graph.inputs).unwrap();

        assert!(result.is_none());

        // But flipping 2 should work
        let result =
            can_flip_to_target_sat(&cnf, &base_inputs, 2, false, 2, &graph.inputs).unwrap();

        assert!(result.is_some());
        let flipped = result.unwrap();
        let a = *flipped.get(&0).unwrap_or(&true);
        let b = *flipped.get(&1).unwrap_or(&true);
        assert!(!a && !b, "Both should be false after flip");
    }

    #[test]
    fn test_sat_based_flip_large_graph() {
        // Test with >10 inputs (previously would fail due to brute-force limits)
        let graph = create_many_input_graph();
        let cnf = extract_cnf(&graph).unwrap();

        // Start with all false (output = false for big OR)
        let mut base_inputs = HashMap::new();
        for &input_id in &graph.inputs {
            base_inputs.insert(input_id, false);
        }

        // Find the output node ID (last node)
        let output_id = *graph.outputs.first().unwrap();

        // Can we flip 1 input to get output = true?
        let result =
            can_flip_to_target_sat(&cnf, &base_inputs, output_id, true, 1, &graph.inputs).unwrap();

        assert!(result.is_some(), "Should find a 1-flip solution for large OR");

        // Verify only 1 input was flipped
        let flipped = result.unwrap();
        let flip_count = flipped.values().filter(|&&v| v).count();
        assert_eq!(flip_count, 1, "Should flip exactly 1 input");
    }

    #[test]
    fn test_epsilon_boundary_uses_sat() {
        let graph = create_or_graph();
        let probes = find_epsilon_boundary(&graph, 2, 2).unwrap();

        // Should find boundary probes
        assert!(!probes.is_empty());

        // All probes should have distance <= epsilon
        for probe in &probes {
            assert!(probe.distance_to_flip <= 2);
        }
    }
}
