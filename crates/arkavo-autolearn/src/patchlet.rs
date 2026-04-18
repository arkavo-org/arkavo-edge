//! Patchlet serialization for network transmission
//!
//! A patchlet is a serializable policy graph with metadata for swarm propagation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use torg_core::Graph;
use uuid::Uuid;

use arkavo_ensemble::GenerationMethod;

use crate::error::{AutoLearnError, AutoLearnResult};

/// Why a patchlet was generated
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PainTrigger {
    /// Unique ID of the anomaly/signal that triggered synthesis
    pub anomaly_id: Uuid,
    /// Human-readable description
    pub description: String,
    /// Severity score (0.0-1.0)
    pub severity: f64,
    /// When the trigger occurred
    pub timestamp: DateTime<Utc>,
}

/// Summary of verification results
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerificationSummary {
    /// Whether verification passed
    pub passed: bool,
    /// Number of invariant checks run
    pub invariant_checks: u32,
    /// Number of SAT probes run
    pub sat_probes: u32,
}

/// Serializable patchlet for network transmission
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Patchlet {
    /// Graph data in portable format
    #[serde(with = "graph_serde")]
    pub graph: Graph,
    /// Why this patchlet was generated
    pub trigger: PainTrigger,
    /// How it was generated
    pub method: GenerationMethod,
    /// Verification results from originator
    pub verification: VerificationSummary,
    /// Creation timestamp
    pub created_at: DateTime<Utc>,
}

impl Patchlet {
    /// Create a new patchlet from a graph and pain signal
    #[must_use]
    pub fn new(graph: Graph, trigger: PainTrigger, method: GenerationMethod) -> Self {
        Self {
            graph,
            trigger,
            method,
            verification: VerificationSummary::default(),
            created_at: Utc::now(),
        }
    }

    /// Set verification summary
    #[must_use]
    pub fn with_verification(mut self, verification: VerificationSummary) -> Self {
        self.verification = verification;
        self
    }

    /// Compute SHA-256 hash of the patchlet content
    #[must_use]
    pub fn content_hash(&self) -> [u8; 32] {
        let json = serde_json::to_vec(self).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(&json);
        hasher.finalize().into()
    }

    /// Serialize to bytes for network transmission
    pub fn to_bytes(&self) -> AutoLearnResult<Vec<u8>> {
        serde_json::to_vec(self).map_err(AutoLearnError::from)
    }

    /// Deserialize from bytes
    pub fn from_bytes(data: &[u8]) -> AutoLearnResult<Self> {
        serde_json::from_slice(data).map_err(AutoLearnError::from)
    }
}

/// Graph serialization module (re-implements arkavo-sbe pattern)
mod graph_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use torg_core::{Graph, graph::Node, token::BoolOp};

    #[derive(Serialize, Deserialize)]
    struct SerializableGraph {
        inputs: Vec<u16>,
        nodes: Vec<SerializableNode>,
        outputs: Vec<u16>,
    }

    #[derive(Serialize, Deserialize)]
    struct SerializableNode {
        id: u16,
        op: String,
        left: SerializableSource,
        right: SerializableSource,
    }

    #[derive(Serialize, Deserialize)]
    #[serde(tag = "type", content = "value")]
    enum SerializableSource {
        Id(u16),
        True,
        False,
    }

    impl From<&torg_core::token::Source> for SerializableSource {
        fn from(source: &torg_core::token::Source) -> Self {
            match source {
                torg_core::token::Source::Id(id) => Self::Id(*id),
                torg_core::token::Source::True => Self::True,
                torg_core::token::Source::False => Self::False,
            }
        }
    }

    impl From<SerializableSource> for torg_core::token::Source {
        fn from(source: SerializableSource) -> Self {
            match source {
                SerializableSource::Id(id) => Self::Id(id),
                SerializableSource::True => Self::True,
                SerializableSource::False => Self::False,
            }
        }
    }

    fn op_to_string(op: &BoolOp) -> String {
        match op {
            BoolOp::Or => "or".to_string(),
            BoolOp::Nor => "nor".to_string(),
            BoolOp::Xor => "xor".to_string(),
        }
    }

    fn string_to_op(s: &str) -> Result<BoolOp, String> {
        match s {
            "or" => Ok(BoolOp::Or),
            "nor" => Ok(BoolOp::Nor),
            "xor" => Ok(BoolOp::Xor),
            _ => Err(format!("Unknown operator: {s}")),
        }
    }

    pub fn serialize<S>(graph: &Graph, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let nodes: Vec<SerializableNode> = graph
            .nodes
            .iter()
            .map(|n| SerializableNode {
                id: n.id,
                op: op_to_string(&n.op),
                left: (&n.left).into(),
                right: (&n.right).into(),
            })
            .collect();

        let serializable = SerializableGraph {
            inputs: graph.inputs.clone(),
            nodes,
            outputs: graph.outputs.clone(),
        };

        serializable.serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Graph, D::Error>
    where
        D: Deserializer<'de>,
    {
        let serializable = SerializableGraph::deserialize(deserializer)?;

        let nodes: Result<Vec<_>, _> = serializable
            .nodes
            .into_iter()
            .map(|n| {
                let op = string_to_op(&n.op).map_err(serde::de::Error::custom)?;
                Ok(Node::new(n.id, op, n.left.into(), n.right.into()))
            })
            .collect();

        Ok(Graph {
            inputs: serializable.inputs,
            nodes: nodes?,
            outputs: serializable.outputs,
        })
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
    fn test_patchlet_roundtrip() {
        let graph = create_simple_graph();
        let trigger = PainTrigger {
            anomaly_id: Uuid::new_v4(),
            description: "Test anomaly".to_string(),
            severity: 0.75,
            timestamp: Utc::now(),
        };

        let patchlet = Patchlet::new(graph.clone(), trigger, GenerationMethod::Manual);

        let bytes = patchlet.to_bytes().unwrap();
        let restored = Patchlet::from_bytes(&bytes).unwrap();

        assert_eq!(patchlet.trigger.anomaly_id, restored.trigger.anomaly_id);
        assert_eq!(graph.inputs, restored.graph.inputs);
        assert_eq!(graph.outputs, restored.graph.outputs);
        assert_eq!(graph.nodes.len(), restored.graph.nodes.len());
    }

    #[test]
    fn test_content_hash_deterministic() {
        let graph = create_simple_graph();
        let trigger = PainTrigger {
            anomaly_id: Uuid::nil(),
            description: "Test".to_string(),
            severity: 0.5,
            timestamp: DateTime::from_timestamp(0, 0).unwrap(),
        };

        let patchlet = Patchlet::new(graph, trigger.clone(), GenerationMethod::Manual);
        let hash1 = patchlet.content_hash();
        let hash2 = patchlet.content_hash();

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_verification_summary() {
        let graph = create_simple_graph();
        let trigger = PainTrigger {
            anomaly_id: Uuid::new_v4(),
            description: "Test".to_string(),
            severity: 0.5,
            timestamp: Utc::now(),
        };

        let verification = VerificationSummary {
            passed: true,
            invariant_checks: 5,
            sat_probes: 100,
        };

        let patchlet =
            Patchlet::new(graph, trigger, GenerationMethod::Manual).with_verification(verification);

        assert!(patchlet.verification.passed);
        assert_eq!(patchlet.verification.invariant_checks, 5);
    }
}
