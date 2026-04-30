//! Coordination, constraints, evaluation, completion, and provenance per spec §4.4–§4.8.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoordinationSpec {
    pub topology: Topology,
    #[serde(default = "default_protocol")]
    pub protocol: String,
    pub routing: Routing,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_sharing: Option<ContextSharing>,
}

fn default_protocol() -> String {
    "a2a-jsonrpc-2.0".to_string()
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum Topology {
    Mesh,
    Pipeline,
    HubSpoke,
    Hierarchical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Routing {
    pub strategy: RoutingStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RoutingStrategy {
    ThompsonSampling,
    RoundRobin,
    CriticDirected,
    Static,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ContextSharing {
    pub store: ContextStore,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<CompactionSpec>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ContextStore {
    Shared,
    PerRole,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompactionSpec {
    pub strategy: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parameters: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConstraintsSpec {
    pub global_budget: GlobalBudget,
    #[serde(default)]
    pub data_classifications: Vec<String>,
    #[serde(default)]
    pub jurisdiction: Vec<String>,
    pub network: NetworkConstraints,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GlobalBudget {
    pub max_wallclock_seconds: u64,
    pub max_total_tokens: u64,
    pub max_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetworkConstraints {
    pub egress_allowed: bool,
    #[serde(default)]
    pub egress_allowlist: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvaluationSpec {
    pub rubric: EvaluationRubric,
    pub critic_role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvaluationRubric {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    pub dimensions: Vec<EvaluationDimension>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvaluationDimension {
    pub name: String,
    pub weight: f64,
    pub threshold: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompletionSpec {
    pub rules: Vec<String>,
    pub on_failure: OnFailure,
    pub max_retries: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OnFailure {
    Retry,
    Abort,
    Escalate,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProvenanceSpec {
    #[serde(default)]
    pub c2pa_assertions: Vec<C2paAssertion>,
    pub signatures: Vec<Signature>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct C2paAssertion {
    pub assertion_uri: String,
    pub digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Signature {
    pub signer_did: String,
    pub algorithm: String,
    pub signature: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn topology_serializes_kebab_case() {
        let t = Topology::HubSpoke;
        assert_eq!(serde_json::to_string(&t).unwrap(), "\"hub-spoke\"");
    }

    #[test]
    fn routing_strategy_serializes_snake_case() {
        let s = RoutingStrategy::ThompsonSampling;
        assert_eq!(serde_json::to_string(&s).unwrap(), "\"thompson_sampling\"");
    }

    #[test]
    fn protocol_default_a2a() {
        let json = r#"{
            "topology": "mesh",
            "routing": {"strategy": "static"}
        }"#;
        let c: CoordinationSpec = serde_json::from_str(json).unwrap();
        assert_eq!(c.protocol, "a2a-jsonrpc-2.0");
    }
}
