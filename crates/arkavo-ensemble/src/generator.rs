//! Candidate generation hooks
//!
//! Provides traits and helpers for generating new policy candidates.

use std::future::Future;
use std::pin::Pin;

use torg_core::Graph;
use uuid::Uuid;

use arkavo_sbe::PolicyLayer;

use crate::candidate::{GenerationMethod, PolicyCandidate};
use crate::error::EnsembleResult;

/// Error during synthesis
#[derive(Debug)]
pub struct SynthesisError {
    pub message: String,
}

impl std::fmt::Display for SynthesisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Synthesis error: {}", self.message)
    }
}

impl std::error::Error for SynthesisError {}

/// Trait for LLM-based policy synthesis (CFT-002)
///
/// Implementations use language models to generate new policy graphs
/// from natural language descriptions or context.
pub trait LlmSynthesizer: Send + Sync {
    /// Synthesize a policy graph from a prompt
    fn synthesize(
        &self,
        prompt: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Graph, SynthesisError>> + Send + '_>>;
}

/// Context for candidate generation
#[derive(Debug, Clone)]
pub struct SynthesisContext {
    /// Model identifier to use
    pub model_id: String,
    /// Description of the desired policy behavior
    pub behavior_description: String,
    /// Input node IDs the policy should use
    pub input_ids: Vec<u16>,
    /// Output node IDs the policy should produce
    pub output_ids: Vec<u16>,
    /// Optional constraints in natural language
    pub constraints: Vec<String>,
}

impl SynthesisContext {
    /// Create a new synthesis context
    pub fn new(model_id: String, behavior_description: String) -> Self {
        Self {
            model_id,
            behavior_description,
            input_ids: Vec::new(),
            output_ids: Vec::new(),
            constraints: Vec::new(),
        }
    }

    /// Add input IDs
    pub fn with_inputs(mut self, inputs: Vec<u16>) -> Self {
        self.input_ids = inputs;
        self
    }

    /// Add output IDs
    pub fn with_outputs(mut self, outputs: Vec<u16>) -> Self {
        self.output_ids = outputs;
        self
    }

    /// Add a constraint
    pub fn with_constraint(mut self, constraint: String) -> Self {
        self.constraints.push(constraint);
        self
    }

    /// Build a prompt from the context
    pub fn build_prompt(&self) -> String {
        let mut prompt = format!(
            "Generate a boolean policy graph that implements: {}\n",
            self.behavior_description
        );

        if !self.input_ids.is_empty() {
            prompt.push_str(&format!("Inputs: {:?}\n", self.input_ids));
        }

        if !self.output_ids.is_empty() {
            prompt.push_str(&format!("Outputs: {:?}\n", self.output_ids));
        }

        for constraint in &self.constraints {
            prompt.push_str(&format!("Constraint: {}\n", constraint));
        }

        prompt
    }
}

/// Generate a candidate from a synthesizer
pub async fn generate_llm_candidate<S: LlmSynthesizer>(
    synthesizer: &S,
    context: &SynthesisContext,
) -> EnsembleResult<PolicyCandidate> {
    let prompt = context.build_prompt();
    let prompt_hash = hash_string(&prompt);

    let graph = synthesizer
        .synthesize(&prompt)
        .await
        .map_err(|e| crate::error::EnsembleError::Synthesis(e.message))?;

    let policy = PolicyLayer::new(graph);

    Ok(PolicyCandidate::new(
        policy,
        GenerationMethod::LlmSynthesis {
            model_id: context.model_id.clone(),
            prompt_hash,
        },
    ))
}

/// Generate a candidate via boundary mutation
pub fn generate_boundary_mutation(
    _base_policy: &PolicyLayer,
    base_id: Uuid,
    mutation_desc: String,
    mutated_graph: Graph,
) -> PolicyCandidate {
    let policy = PolicyLayer::new(mutated_graph);

    PolicyCandidate::new(
        policy,
        GenerationMethod::BoundaryMutation {
            base_id,
            mutation_desc,
        },
    )
}

/// Generate a candidate to remediate an anomaly
pub fn generate_anomaly_remediation(anomaly_id: Uuid, remediation_graph: Graph) -> PolicyCandidate {
    let policy = PolicyLayer::new(remediation_graph);

    PolicyCandidate::new(policy, GenerationMethod::AnomalyRemediation { anomaly_id })
}

/// Hash a string for prompt tracking
fn hash_string(s: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    s.hash(&mut hasher);
    hasher.finish()
}

/// Mock synthesizer for testing
#[cfg(test)]
pub struct MockSynthesizer {
    graph: Graph,
}

#[cfg(test)]
impl MockSynthesizer {
    pub fn new(graph: Graph) -> Self {
        Self { graph }
    }
}

#[cfg(test)]
impl LlmSynthesizer for MockSynthesizer {
    fn synthesize(
        &self,
        _prompt: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Graph, SynthesisError>> + Send + '_>> {
        let graph = self.graph.clone();
        Box::pin(async move { Ok(graph) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::{Builder, Token};

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
    fn test_synthesis_context() {
        let ctx = SynthesisContext::new("gpt-4".to_string(), "Allow if user is admin".to_string())
            .with_inputs(vec![0, 1])
            .with_outputs(vec![2])
            .with_constraint("Must check user role".to_string());

        let prompt = ctx.build_prompt();
        assert!(prompt.contains("Allow if user is admin"));
        assert!(prompt.contains("Inputs:"));
        assert!(prompt.contains("Outputs:"));
        assert!(prompt.contains("Must check user role"));
    }

    #[tokio::test]
    async fn test_generate_llm_candidate() {
        let graph = create_simple_graph();
        let synthesizer = MockSynthesizer::new(graph);
        let context = SynthesisContext::new("mock".to_string(), "test policy".to_string());

        let candidate = generate_llm_candidate(&synthesizer, &context)
            .await
            .unwrap();

        match candidate.generation_method {
            GenerationMethod::LlmSynthesis { model_id, .. } => {
                assert_eq!(model_id, "mock");
            }
            _ => panic!("Expected LlmSynthesis"),
        }
    }

    #[test]
    fn test_generate_boundary_mutation() {
        let base = PolicyLayer::new(create_simple_graph());
        let base_id = Uuid::new_v4();
        let mutated = create_simple_graph();

        let candidate =
            generate_boundary_mutation(&base, base_id, "flip node 1".to_string(), mutated);

        match candidate.generation_method {
            GenerationMethod::BoundaryMutation {
                base_id: id,
                mutation_desc,
            } => {
                assert_eq!(id, base_id);
                assert_eq!(mutation_desc, "flip node 1");
            }
            _ => panic!("Expected BoundaryMutation"),
        }
    }

    #[test]
    fn test_generate_anomaly_remediation() {
        let anomaly_id = Uuid::new_v4();
        let remediation = create_simple_graph();

        let candidate = generate_anomaly_remediation(anomaly_id, remediation);

        match candidate.generation_method {
            GenerationMethod::AnomalyRemediation { anomaly_id: id } => {
                assert_eq!(id, anomaly_id);
            }
            _ => panic!("Expected AnomalyRemediation"),
        }
    }
}
