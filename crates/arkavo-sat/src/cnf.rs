//! CNF extraction from TØRG graphs using Tseitin transformation
//!
//! Converts boolean circuits to Conjunctive Normal Form for SAT solving.

use std::collections::HashMap;

use torg_core::{Graph, token::BoolOp};
use varisat::Lit;

use crate::error::SatResult;

/// A CNF formula extracted from a TØRG graph
#[derive(Debug, Clone)]
pub struct CnfFormula {
    /// Clauses in CNF (each clause is a disjunction of literals)
    pub clauses: Vec<Vec<Lit>>,
    /// Mapping from TØRG node IDs to varisat variables
    pub variable_map: HashMap<u16, Lit>,
    /// Number of variables in the formula
    pub num_variables: usize,
}

impl CnfFormula {
    /// Create a new empty CNF formula
    #[must_use]
    pub fn new() -> Self {
        Self {
            clauses: Vec::new(),
            variable_map: HashMap::new(),
            num_variables: 0,
        }
    }

    /// Add a clause to the formula
    pub fn add_clause(&mut self, clause: Vec<Lit>) {
        self.clauses.push(clause);
    }

    /// Get or create a variable for a node ID
    pub fn get_or_create_var(&mut self, node_id: u16) -> Lit {
        if let Some(&lit) = self.variable_map.get(&node_id) {
            lit
        } else {
            let var = Lit::from_dimacs((self.num_variables + 1) as isize);
            self.num_variables += 1;
            self.variable_map.insert(node_id, var);
            var
        }
    }

    /// Get the total number of clauses
    #[must_use]
    pub fn clause_count(&self) -> usize {
        self.clauses.len()
    }
}

impl Default for CnfFormula {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract CNF from a TØRG graph using Tseitin transformation
///
/// The Tseitin transformation converts a boolean circuit to CNF by:
/// 1. Assigning a variable to each node
/// 2. Adding clauses that encode each gate's behavior
///
/// This preserves satisfiability while producing a formula with
/// linear size increase.
pub fn extract_cnf(graph: &Graph) -> SatResult<CnfFormula> {
    let mut formula = CnfFormula::new();

    // Create variables for all inputs
    for &input_id in &graph.inputs {
        formula.get_or_create_var(input_id);
    }

    // Process each node and add gate clauses
    for node in &graph.nodes {
        let output = formula.get_or_create_var(node.id);
        let left = resolve_source(&node.left, &mut formula)?;
        let right = resolve_source(&node.right, &mut formula)?;

        // Add clauses based on operator
        match node.op {
            BoolOp::Or => add_or_clauses(&mut formula, output, left, right),
            BoolOp::Nor => add_nor_clauses(&mut formula, output, left, right),
            BoolOp::Xor => add_xor_clauses(&mut formula, output, left, right),
        }
    }

    Ok(formula)
}

/// Resolve a source operand to a literal
fn resolve_source(source: &torg_core::token::Source, formula: &mut CnfFormula) -> SatResult<Lit> {
    match source {
        torg_core::token::Source::Id(id) => Ok(formula.get_or_create_var(*id)),
        torg_core::token::Source::True => {
            // Create a fresh variable constrained to true
            let var = Lit::from_dimacs((formula.num_variables + 1) as isize);
            formula.num_variables += 1;
            formula.add_clause(vec![var]); // Force to true
            Ok(var)
        }
        torg_core::token::Source::False => {
            // Create a fresh variable constrained to false
            let var = Lit::from_dimacs((formula.num_variables + 1) as isize);
            formula.num_variables += 1;
            formula.add_clause(vec![!var]); // Force to false
            Ok(var)
        }
    }
}

/// Add clauses for OR gate: o = a OR b
/// Clauses: (a ∨ b ∨ ¬o), (¬a ∨ o), (¬b ∨ o)
fn add_or_clauses(formula: &mut CnfFormula, o: Lit, a: Lit, b: Lit) {
    formula.add_clause(vec![a, b, !o]);
    formula.add_clause(vec![!a, o]);
    formula.add_clause(vec![!b, o]);
}

/// Add clauses for NOR gate: o = NOT(a OR b) = ¬a AND ¬b
/// Clauses: (¬a ∨ ¬o), (¬b ∨ ¬o), (a ∨ b ∨ o)
fn add_nor_clauses(formula: &mut CnfFormula, o: Lit, a: Lit, b: Lit) {
    formula.add_clause(vec![!a, !o]);
    formula.add_clause(vec![!b, !o]);
    formula.add_clause(vec![a, b, o]);
}

/// Add clauses for XOR gate: o = a XOR b
/// Clauses:
///   (a ∨ b ∨ ¬o), (¬a ∨ ¬b ∨ ¬o),
///   (¬a ∨ b ∨ o), (a ∨ ¬b ∨ o)
fn add_xor_clauses(formula: &mut CnfFormula, o: Lit, a: Lit, b: Lit) {
    formula.add_clause(vec![a, b, !o]);
    formula.add_clause(vec![!a, !b, !o]);
    formula.add_clause(vec![!a, b, o]);
    formula.add_clause(vec![a, !b, o]);
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
    fn test_extract_cnf_or_gate() {
        let graph = create_or_graph();
        let cnf = extract_cnf(&graph).unwrap();

        // Should have 3 variables (2 inputs + 1 node)
        assert_eq!(cnf.num_variables, 3);

        // Should have 3 clauses for OR gate
        assert_eq!(cnf.clause_count(), 3);

        // All variables should be mapped
        assert!(cnf.variable_map.contains_key(&0));
        assert!(cnf.variable_map.contains_key(&1));
        assert!(cnf.variable_map.contains_key(&2));
    }

    #[test]
    fn test_cnf_formula_basic() {
        let mut formula = CnfFormula::new();
        assert_eq!(formula.num_variables, 0);
        assert_eq!(formula.clause_count(), 0);

        let var = formula.get_or_create_var(5);
        assert_eq!(formula.num_variables, 1);

        // Same ID should return same variable
        let var2 = formula.get_or_create_var(5);
        assert_eq!(var, var2);
        assert_eq!(formula.num_variables, 1);

        // Different ID creates new variable
        formula.get_or_create_var(10);
        assert_eq!(formula.num_variables, 2);
    }
}
