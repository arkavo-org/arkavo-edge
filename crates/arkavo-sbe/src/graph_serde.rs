//! Serialization helper for torg_core::Graph
//!
//! Since torg_core::Graph doesn't implement Serialize/Deserialize,
//! we provide custom serialization that converts to/from a portable format.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use torg_core::{Graph, token::BoolOp};

/// Serializable representation of a Graph
#[derive(Serialize, Deserialize)]
struct SerializableGraph {
    inputs: Vec<u16>,
    nodes: Vec<SerializableNode>,
    outputs: Vec<u16>,
}

/// Serializable representation of a Node
#[derive(Serialize, Deserialize)]
struct SerializableNode {
    id: u16,
    op: String,
    left: SerializableSource,
    right: SerializableSource,
}

/// Serializable representation of a Source
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
            Ok(torg_core::graph::Node::new(
                n.id,
                op,
                n.left.into(),
                n.right.into(),
            ))
        })
        .collect();

    Ok(Graph {
        inputs: serializable.inputs,
        nodes: nodes?,
        outputs: serializable.outputs,
    })
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
    fn test_graph_roundtrip() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            #[serde(with = "super")]
            graph: Graph,
        }

        let original = create_simple_graph();
        let wrapper = Wrapper {
            graph: original.clone(),
        };

        let json = serde_json::to_string(&wrapper).unwrap();
        let restored: Wrapper = serde_json::from_str(&json).unwrap();

        assert_eq!(original.inputs, restored.graph.inputs);
        assert_eq!(original.outputs, restored.graph.outputs);
        assert_eq!(original.nodes.len(), restored.graph.nodes.len());
    }
}
