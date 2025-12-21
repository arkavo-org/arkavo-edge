//! SAT solver integration for TØRG policy analysis
//!
//! This crate provides SAT-based analysis tools for TØRG boolean policy graphs:
//!
//! - **CNF Extraction**: Convert TØRG graphs to CNF using Tseitin transformation
//! - **Boundary Probing**: Find inputs near decision boundaries
//! - **Policy Stress Testing**: Identify potential policy holes
//!
//! # Example
//!
//! ```ignore
//! use arkavo_sat::{extract_cnf, probe_boundary};
//! use torg_core::Graph;
//!
//! // Extract CNF from a policy graph
//! let cnf = extract_cnf(&graph)?;
//! println!("Formula has {} clauses", cnf.clause_count());
//!
//! // Find inputs near the decision boundary
//! let probes = probe_boundary(&graph, output_id)?;
//! for probe in probes {
//!     println!("Found boundary probe: {:?}", probe.input_assignment);
//! }
//! ```
//!
//! # SAT Solver
//!
//! This crate uses [varisat](https://crates.io/crates/varisat), a pure-Rust
//! CDCL SAT solver with good performance characteristics.

pub mod boundary;
pub mod cache;
pub mod cardinality;
pub mod cnf;
pub mod error;
pub mod incremental;
pub mod prioritizer;
pub mod scheduler;
pub mod stress;

pub use boundary::{
    BoundaryProbe, can_flip_to_target_sat, find_epsilon_boundary, find_satisfying_inputs,
    probe_boundary,
};
pub use cache::{
    CacheStats, DEFAULT_CACHE_CAPACITY, DEFAULT_CACHE_TTL, ProbeCache, ProbeCacheKey,
    compute_graph_hash, find_epsilon_boundary_cached,
};
pub use cardinality::{
    create_flip_indicators, encode_at_most_k, encode_exactly_k, encode_xor, force_value,
};
pub use cnf::{CnfFormula, extract_cnf};
pub use error::{SatError, SatResult};
pub use incremental::{IncrementalAnalysis, ModificationTracker};
pub use prioritizer::{
    AnomalyPrioritizer, NodeHistory, PriorityReason, PriorityWeights, ScoredNode,
};
pub use scheduler::{
    DEFAULT_BATCH_SIZE, DEFAULT_CPU_BUDGET, DEFAULT_MIN_INTERVAL, ProbeResult, ProbeScheduler,
    ProbeTask, SchedulerConfig, SchedulerStats, probe_sync,
};
pub use stress::{
    CoverageStats, PolicyHole, StressTestConfig, StressTestResult, find_contradictions,
    is_satisfiable, is_tautology, stress_test,
};

// Re-export torg-core and varisat types for convenience
pub use torg_core::Graph;
pub use varisat::Lit;

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::{Builder, Token};

    fn create_and_graph() -> Graph {
        // AND = NOR(NOR(a,a), NOR(b,b))
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(1)).unwrap();

        // NOT a = NOR(a, a)
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Nor).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeEnd).unwrap();

        // NOT b = NOR(b, b)
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(3)).unwrap();
        builder.push(Token::Nor).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::NodeEnd).unwrap();

        // AND = NOR(NOT a, NOT b)
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(4)).unwrap();
        builder.push(Token::Nor).unwrap();
        builder.push(Token::Id(2)).unwrap();
        builder.push(Token::Id(3)).unwrap();
        builder.push(Token::NodeEnd).unwrap();

        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(4)).unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn test_and_gate_analysis() {
        let graph = create_and_graph();

        // Extract CNF
        let cnf = extract_cnf(&graph).unwrap();
        assert!(cnf.clause_count() > 0);

        // Find satisfying inputs for true output
        let true_inputs = find_satisfying_inputs(&graph, 4, true).unwrap();
        assert!(true_inputs.is_some());

        let inputs = true_inputs.unwrap();
        // For AND to be true, both inputs must be true
        assert_eq!(inputs.get(&0), Some(&true));
        assert_eq!(inputs.get(&1), Some(&true));

        // Find satisfying inputs for false output
        let false_inputs = find_satisfying_inputs(&graph, 4, false).unwrap();
        assert!(false_inputs.is_some());

        let inputs = false_inputs.unwrap();
        // For AND to be false, at least one input must be false
        let a = *inputs.get(&0).unwrap_or(&true);
        let b = *inputs.get(&1).unwrap_or(&true);
        assert!(!a || !b);
    }

    #[test]
    fn test_boundary_analysis() {
        let graph = create_and_graph();
        let probes = probe_boundary(&graph, 4).unwrap();

        // Should find both true and false cases
        assert!(!probes.is_empty());
    }
}
