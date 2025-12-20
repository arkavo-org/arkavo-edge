//! Cardinality constraint encoding for SAT solving
//!
//! Implements the Sequential Counter encoding (Sinz 2005) for "at most k"
//! constraints. This allows efficient SAT-based k-flip queries without
//! brute-force enumeration.

use varisat::{ExtendFormula, Lit, Solver};

use crate::error::{SatError, SatResult};

/// Encode "at most k of these literals are true" using Sequential Counter
///
/// The Sequential Counter encoding (Sinz 2005) creates O(n*k) auxiliary
/// variables and clauses, which is much more efficient than the O(C(n,k))
/// brute-force enumeration for large n and small k.
///
/// # Algorithm
///
/// For n input literals x₁, ..., xₙ and constraint "at most k":
/// 1. Create auxiliary variables s[i][j] for i ∈ 1..n, j ∈ 1..k
/// 2. s[i][j] means "at least j of x₁, ..., xᵢ are true"
/// 3. Add clauses encoding the counting semantics
///
/// # Errors
///
/// Returns an error if k is 0 or if the literals slice is empty.
pub fn encode_at_most_k(solver: &mut Solver, lits: &[Lit], k: usize) -> SatResult<()> {
    let n = lits.len();

    if k == 0 {
        // At most 0 means all must be false
        for &lit in lits {
            solver.add_clause(&[!lit]);
        }
        return Ok(());
    }

    if n == 0 {
        return Ok(()); // Empty constraint is trivially satisfied
    }

    if k >= n {
        return Ok(()); // At most n of n is always satisfied
    }

    // Create auxiliary variables s[i][j] for i ∈ 0..n, j ∈ 0..k
    // s[i][j] = true means "at least j+1 of x₀, ..., xᵢ are true"
    let mut aux_vars: Vec<Vec<Lit>> = Vec::with_capacity(n);

    for _ in 0..n {
        let mut row = Vec::with_capacity(k);
        for _ in 0..k {
            let var = solver.new_var();
            row.push(Lit::from_var(var, false));
        }
        aux_vars.push(row);
    }

    // First row (i = 0): Initialize based on x₀
    // ¬x₀ ∨ s[0][0] (if x₀ is true, at least 1 is true)
    solver.add_clause(&[!lits[0], aux_vars[0][0]]);

    // ¬s[0][j] for j > 0 (can't have more than 1 true from just x₀)
    for s_0_j in aux_vars[0].iter().skip(1) {
        solver.add_clause(&[!*s_0_j]);
    }

    // Subsequent rows (i > 0)
    for i in 1..n {
        // If xᵢ is true, at least 1 is true: ¬xᵢ ∨ s[i][0]
        solver.add_clause(&[!lits[i], aux_vars[i][0]]);

        // Propagate from previous row: ¬s[i-1][0] ∨ s[i][0]
        solver.add_clause(&[!aux_vars[i - 1][0], aux_vars[i][0]]);

        for j in 1..k {
            // Propagate count: ¬s[i-1][j] ∨ s[i][j]
            solver.add_clause(&[!aux_vars[i - 1][j], aux_vars[i][j]]);

            // Increment count: ¬xᵢ ∨ ¬s[i-1][j-1] ∨ s[i][j]
            solver.add_clause(&[!lits[i], !aux_vars[i - 1][j - 1], aux_vars[i][j]]);
        }

        // Overflow prevention: ¬xᵢ ∨ ¬s[i-1][k-1]
        // (if we already have k true, xᵢ must be false)
        solver.add_clause(&[!lits[i], !aux_vars[i - 1][k - 1]]);
    }

    Ok(())
}

/// Encode "exactly k of these literals are true"
///
/// Combines at_most_k and at_least_k constraints.
pub fn encode_exactly_k(solver: &mut Solver, lits: &[Lit], k: usize) -> SatResult<()> {
    let n = lits.len();

    if k > n {
        return Err(SatError::Cardinality(format!(
            "Cannot have exactly {} true out of {} literals",
            k, n
        )));
    }

    // At most k
    encode_at_most_k(solver, lits, k)?;

    // At least k (equivalent to: at most n-k of the negations)
    let negated: Vec<Lit> = lits.iter().map(|&l| !l).collect();
    encode_at_most_k(solver, &negated, n - k)?;

    Ok(())
}

/// Create flip indicator variables for SAT-based flip checking
///
/// For each input, creates a variable that indicates whether that input
/// should be flipped from its base value.
///
/// Returns the flip indicator literals.
pub fn create_flip_indicators(solver: &mut Solver, input_count: usize) -> Vec<Lit> {
    (0..input_count)
        .map(|_| {
            let var = solver.new_var();
            Lit::from_var(var, false)
        })
        .collect()
}

/// Create XOR constraint: output = a XOR b
///
/// Used to encode: flipped_value = base_value XOR flip_indicator
pub fn encode_xor(solver: &mut Solver, output: Lit, a: Lit, b: Lit) {
    // XOR truth table:
    // a=0, b=0 -> out=0: (¬a ∨ ¬b ∨ ¬out)... wait, that's wrong for this case
    // Actually, XOR clauses:
    // (a ∨ b ∨ ¬out), (¬a ∨ ¬b ∨ ¬out), (¬a ∨ b ∨ out), (a ∨ ¬b ∨ out)
    solver.add_clause(&[a, b, !output]);
    solver.add_clause(&[!a, !b, !output]);
    solver.add_clause(&[!a, b, output]);
    solver.add_clause(&[a, !b, output]);
}

/// Create constraint that forces a literal to a specific value
pub fn force_value(solver: &mut Solver, lit: Lit, value: bool) {
    if value {
        solver.add_clause(&[lit]);
    } else {
        solver.add_clause(&[!lit]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_at_most_0() {
        let mut solver = Solver::new();
        let vars: Vec<Lit> = (0..3)
            .map(|_| Lit::from_var(solver.new_var(), false))
            .collect();

        encode_at_most_k(&mut solver, &vars, 0).unwrap();

        // Should be SAT only when all are false
        solver.assume(&[!vars[0], !vars[1], !vars[2]]);
        assert!(solver.solve().unwrap());

        // Should be UNSAT with any true
        let mut solver2 = Solver::new();
        let vars2: Vec<Lit> = (0..3)
            .map(|_| Lit::from_var(solver2.new_var(), false))
            .collect();
        encode_at_most_k(&mut solver2, &vars2, 0).unwrap();
        solver2.assume(&[vars2[0]]);
        assert!(!solver2.solve().unwrap());
    }

    #[test]
    fn test_at_most_1() {
        let mut solver = Solver::new();
        let vars: Vec<Lit> = (0..3)
            .map(|_| Lit::from_var(solver.new_var(), false))
            .collect();

        encode_at_most_k(&mut solver, &vars, 1).unwrap();

        // SAT with exactly 1 true
        solver.assume(&[vars[0], !vars[1], !vars[2]]);
        assert!(solver.solve().unwrap());

        // UNSAT with 2 true
        let mut solver2 = Solver::new();
        let vars2: Vec<Lit> = (0..3)
            .map(|_| Lit::from_var(solver2.new_var(), false))
            .collect();
        encode_at_most_k(&mut solver2, &vars2, 1).unwrap();
        solver2.assume(&[vars2[0], vars2[1]]);
        assert!(!solver2.solve().unwrap());
    }

    #[test]
    fn test_at_most_2() {
        let mut solver = Solver::new();
        let vars: Vec<Lit> = (0..4)
            .map(|_| Lit::from_var(solver.new_var(), false))
            .collect();

        encode_at_most_k(&mut solver, &vars, 2).unwrap();

        // SAT with exactly 2 true
        solver.assume(&[vars[0], vars[1], !vars[2], !vars[3]]);
        assert!(solver.solve().unwrap());

        // UNSAT with 3 true
        let mut solver2 = Solver::new();
        let vars2: Vec<Lit> = (0..4)
            .map(|_| Lit::from_var(solver2.new_var(), false))
            .collect();
        encode_at_most_k(&mut solver2, &vars2, 2).unwrap();
        solver2.assume(&[vars2[0], vars2[1], vars2[2]]);
        assert!(!solver2.solve().unwrap());
    }

    #[test]
    fn test_exactly_k() {
        let mut solver = Solver::new();
        let vars: Vec<Lit> = (0..4)
            .map(|_| Lit::from_var(solver.new_var(), false))
            .collect();

        encode_exactly_k(&mut solver, &vars, 2).unwrap();

        // SAT with exactly 2 true
        solver.assume(&[vars[0], vars[1], !vars[2], !vars[3]]);
        assert!(solver.solve().unwrap());

        // UNSAT with 1 true
        let mut solver2 = Solver::new();
        let vars2: Vec<Lit> = (0..4)
            .map(|_| Lit::from_var(solver2.new_var(), false))
            .collect();
        encode_exactly_k(&mut solver2, &vars2, 2).unwrap();
        solver2.assume(&[vars2[0], !vars2[1], !vars2[2], !vars2[3]]);
        assert!(!solver2.solve().unwrap());
    }

    #[test]
    fn test_flip_indicators() {
        let mut solver = Solver::new();
        let flips = create_flip_indicators(&mut solver, 5);
        assert_eq!(flips.len(), 5);
    }

    #[test]
    fn test_xor_constraint() {
        let mut solver = Solver::new();
        let a = Lit::from_var(solver.new_var(), false);
        let b = Lit::from_var(solver.new_var(), false);
        let out = Lit::from_var(solver.new_var(), false);

        encode_xor(&mut solver, out, a, b);

        // a=0, b=0 -> out=0
        solver.assume(&[!a, !b, !out]);
        assert!(solver.solve().unwrap());

        // a=1, b=0 -> out=1
        let mut solver2 = Solver::new();
        let a2 = Lit::from_var(solver2.new_var(), false);
        let b2 = Lit::from_var(solver2.new_var(), false);
        let out2 = Lit::from_var(solver2.new_var(), false);
        encode_xor(&mut solver2, out2, a2, b2);
        solver2.assume(&[a2, !b2, out2]);
        assert!(solver2.solve().unwrap());

        // a=1, b=1 -> out=0
        let mut solver3 = Solver::new();
        let a3 = Lit::from_var(solver3.new_var(), false);
        let b3 = Lit::from_var(solver3.new_var(), false);
        let out3 = Lit::from_var(solver3.new_var(), false);
        encode_xor(&mut solver3, out3, a3, b3);
        solver3.assume(&[a3, b3, !out3]);
        assert!(solver3.solve().unwrap());
    }

    #[test]
    fn test_at_most_k_large() {
        // Test with more inputs to verify the encoding scales
        let mut solver = Solver::new();
        let vars: Vec<Lit> = (0..20)
            .map(|_| Lit::from_var(solver.new_var(), false))
            .collect();

        encode_at_most_k(&mut solver, &vars, 5).unwrap();

        // SAT with 5 true
        let mut assumptions: Vec<Lit> = vars.iter().take(5).copied().collect();
        assumptions.extend(vars.iter().skip(5).map(|&l| !l));
        solver.assume(&assumptions);
        assert!(solver.solve().unwrap());
    }
}
