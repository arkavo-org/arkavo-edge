//! Boundary probing for policy stress testing
//!
//! Generates synthetic inputs near decision boundaries using SAT solving.

use std::collections::HashMap;

use torg_core::Graph;
use varisat::{ExtendFormula, Solver};

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

/// Check if we can reach target output by flipping exactly `k` inputs
fn can_flip_to_target(
    graph: &Graph,
    _cnf: &CnfFormula,
    base_inputs: &HashMap<u16, bool>,
    output_id: u16,
    target_output: bool,
    k: u32,
) -> SatResult<bool> {
    // For small k, enumerate all k-subsets
    let input_ids: Vec<u16> = graph.inputs.clone();

    if k == 1 {
        for &input_id in &input_ids {
            let mut flipped = base_inputs.clone();
            if let Some(val) = flipped.get_mut(&input_id) {
                *val = !*val;
            }

            // Check if this produces the target output
            let result = torg_core::evaluate(graph, &flipped);
            if let Ok(outputs) = result
                && outputs.get(&output_id) == Some(&target_output)
            {
                return Ok(true);
            }
        }
        return Ok(false);
    }

    // For larger k, use cardinality constraints with SAT solver
    // (simplified: just try all combinations up to small k)
    if k <= 3 && input_ids.len() <= 10 {
        let n = input_ids.len();
        for combo in combinations(n, k as usize) {
            let mut flipped = base_inputs.clone();
            for &idx in &combo {
                let input_id = input_ids[idx];
                if let Some(val) = flipped.get_mut(&input_id) {
                    *val = !*val;
                }
            }

            let result = torg_core::evaluate(graph, &flipped);
            if let Ok(outputs) = result
                && outputs.get(&output_id) == Some(&target_output)
            {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

/// Generate all k-combinations of indices from 0..n
fn combinations(n: usize, k: usize) -> Vec<Vec<usize>> {
    let mut result = Vec::new();
    let mut combo = vec![0usize; k];

    fn generate(
        start: usize,
        n: usize,
        k: usize,
        pos: usize,
        combo: &mut Vec<usize>,
        result: &mut Vec<Vec<usize>>,
    ) {
        if pos == k {
            result.push(combo.clone());
            return;
        }
        for i in start..=(n - k + pos) {
            combo[pos] = i;
            generate(i + 1, n, k, pos + 1, combo, result);
        }
    }

    if k <= n {
        generate(0, n, k, 0, &mut combo, &mut result);
    }
    result
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
    fn test_combinations() {
        let combos = combinations(4, 2);
        assert_eq!(combos.len(), 6); // C(4,2) = 6
        assert!(combos.contains(&vec![0, 1]));
        assert!(combos.contains(&vec![2, 3]));
    }
}
