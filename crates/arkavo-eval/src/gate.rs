//! Critic pre-flight gate. The contract's required preconditions are AND-ed
//! together as a real torg_core boolean circuit; the gate allows iff every
//! required precondition holds. If denied, the first failing precondition
//! becomes the typed refusal reason.

use crate::status::TypedStatus;
use std::collections::HashMap;
use torg_core::{evaluate, BoolOp, Graph, Node, Source};

/// Boolean state of each known precondition. Fields not enforced in this slice
/// (provenance/attestation) default to `true` so they never block the gate; the
/// Operator records evidence separately.
#[derive(Debug, Clone)]
pub struct Preconditions {
    pub weights_present: bool,
    pub weights_attested: bool,
    pub provenance_valid: bool,
    pub baseline_present: bool,
}

impl Default for Preconditions {
    fn default() -> Self {
        Self {
            weights_present: false,
            weights_attested: false,
            provenance_valid: true,
            baseline_present: false,
        }
    }
}

impl Preconditions {
    fn value(&self, name: &str) -> Option<bool> {
        match name {
            "weights_present" => Some(self.weights_present),
            "weights_attested" => Some(self.weights_attested),
            "provenance_valid" => Some(self.provenance_valid),
            "baseline_present" => Some(self.baseline_present),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GateDecision {
    Allow,
    Deny { reason: String },
}

impl GateDecision {
    pub fn into_status_if_denied(self) -> Option<TypedStatus> {
        match self {
            GateDecision::Allow => None,
            GateDecision::Deny { reason } => Some(TypedStatus::Refused { reason }),
        }
    }
}

/// Build a graph whose single output is the AND of `n` inputs (ids 0..n).
/// AND(a,b) = NOR(NOT a, NOT b); NOT x = NOR(x,x). Chained for n>2.
fn build_and_graph(n: usize) -> Graph {
    assert!(n >= 1, "gate requires at least one precondition");
    let inputs: Vec<u16> = (0..n as u16).collect();
    if n == 1 {
        return Graph {
            inputs,
            nodes: vec![],
            outputs: vec![0],
        };
    }
    let mut nodes = Vec::new();
    let mut next_id: u16 = n as u16;
    // running AND accumulator, starts as input 0
    let mut acc: u16 = 0;
    for i in 1..n as u16 {
        // not_acc = NOR(acc, acc)
        let not_acc = next_id;
        next_id += 1;
        nodes.push(Node::new(
            not_acc,
            BoolOp::Nor,
            Source::Id(acc),
            Source::Id(acc),
        ));
        // not_i = NOR(i, i)
        let not_i = next_id;
        next_id += 1;
        nodes.push(Node::new(not_i, BoolOp::Nor, Source::Id(i), Source::Id(i)));
        // and = NOR(not_acc, not_i)
        let and = next_id;
        next_id += 1;
        nodes.push(Node::new(
            and,
            BoolOp::Nor,
            Source::Id(not_acc),
            Source::Id(not_i),
        ));
        acc = and;
    }
    Graph {
        inputs,
        nodes,
        outputs: vec![acc],
    }
}

/// Evaluate the gate over the contract's required precondition names.
pub fn evaluate_gate(pre: &Preconditions, required: &[String]) -> GateDecision {
    if required.is_empty() {
        return GateDecision::Allow;
    }
    // Resolve each required precondition to a bool; an unknown name is a refusal.
    let mut values = Vec::with_capacity(required.len());
    for name in required {
        match pre.value(name) {
            Some(v) => values.push((name.clone(), v)),
            None => {
                return GateDecision::Deny {
                    reason: format!("unknown precondition: {name}"),
                }
            }
        }
    }
    let graph = build_and_graph(values.len());
    let mut inputs = HashMap::new();
    for (i, (_, v)) in values.iter().enumerate() {
        inputs.insert(i as u16, *v);
    }
    let out_id = *graph.outputs.first().expect("one output");
    match evaluate(&graph, &inputs) {
        Ok(result) if result.get(&out_id).copied().unwrap_or(false) => GateDecision::Allow,
        Ok(_) => {
            // Denied — name the first failing precondition.
            let failed = values
                .iter()
                .find(|(_, v)| !v)
                .map(|(n, _)| n.clone())
                .unwrap_or_default();
            GateDecision::Deny {
                reason: format!("precondition not met: {failed}"),
            }
        }
        Err(e) => GateDecision::Deny {
            reason: format!("policy circuit error: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> Vec<String> {
        vec![
            "weights_present".into(),
            "weights_attested".into(),
            "baseline_present".into(),
        ]
    }

    #[test]
    fn allows_when_all_required_true() {
        let pre = Preconditions {
            weights_present: true,
            weights_attested: true,
            provenance_valid: true,
            baseline_present: true,
        };
        assert_eq!(evaluate_gate(&pre, &req()), GateDecision::Allow);
    }

    #[test]
    fn denies_and_names_failed_precondition() {
        let pre = Preconditions {
            weights_present: true,
            weights_attested: true,
            provenance_valid: true,
            baseline_present: false,
        };
        match evaluate_gate(&pre, &req()) {
            GateDecision::Deny { reason } => assert!(reason.contains("baseline_present")),
            other => panic!("expected deny, got {other:?}"),
        }
    }

    #[test]
    fn unknown_precondition_is_denied() {
        let pre = Preconditions::default();
        match evaluate_gate(&pre, &["nonsense".to_string()]) {
            GateDecision::Deny { reason } => assert!(reason.contains("unknown")),
            other => panic!("expected deny, got {other:?}"),
        }
    }

    #[test]
    fn empty_required_allows() {
        assert_eq!(
            evaluate_gate(&Preconditions::default(), &[]),
            GateDecision::Allow
        );
    }
}
