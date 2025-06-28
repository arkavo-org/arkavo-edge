use semver::Version;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blueprint {
    pub version: Version,
    pub name: Option<String>,
    pub description: Option<String>,
    pub nodes: Vec<Node>,
    pub links: Vec<Link>,
    pub metadata: Option<HashMap<String, serde_json::Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Node {
    pub id: String,
    pub kind: NodeKind,
    pub params: HashMap<String, serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Source,
    Transform,
    Sink,
    Router,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rule>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    #[serde(rename = "type")]
    pub rule_type: RuleType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conditions: Option<Vec<Condition>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transform: Option<Transform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub split_config: Option<SplitConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_config: Option<MergeConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleType {
    Filter,
    Transform,
    Split,
    Merge,
    Passthrough,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Condition {
    pub field: String,
    pub operator: ConditionOperator,
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConditionOperator {
    Contains,
    Equals,
    Matches,
    Gt,
    Lt,
    Gte,
    Lte,
    Exists,
    NotExists,
    StartsWith,
    EndsWith,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transform {
    #[serde(rename = "type")]
    pub transform_type: TransformType,
    pub params: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TransformType {
    Extract,
    Format,
    Aggregate,
    Enrich,
    Map,
    Reduce,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplitConfig {
    pub strategy: SplitStrategy,
    pub targets: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SplitStrategy {
    RoundRobin,
    Hash,
    Broadcast,
    Conditional,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MergeConfig {
    pub strategy: MergeStrategy,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeStrategy {
    First,
    All,
    Concat,
    Vote,
}

impl Blueprint {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            version: Version::new(1, 0, 0),
            name: Some(name.into()),
            description: None,
            nodes: Vec::new(),
            links: Vec::new(),
            metadata: None,
        }
    }

    pub fn add_node(&mut self, node: Node) {
        self.nodes.push(node);
    }

    pub fn add_link(&mut self, link: Link) {
        self.links.push(link);
    }

    pub fn find_node(&self, id: &str) -> Option<&Node> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn find_node_mut(&mut self, id: &str) -> Option<&mut Node> {
        self.nodes.iter_mut().find(|n| n.id == id)
    }

    pub fn validate(&self) -> Result<(), String> {
        // Check all links reference existing nodes
        for link in &self.links {
            if !self.nodes.iter().any(|n| n.id == link.from) {
                return Err(format!(
                    "Link references non-existent source node: {}",
                    link.from
                ));
            }
            if !self.nodes.iter().any(|n| n.id == link.to) {
                return Err(format!(
                    "Link references non-existent target node: {}",
                    link.to
                ));
            }
        }

        // Check for duplicate node IDs
        let mut seen = std::collections::HashSet::new();
        for node in &self.nodes {
            if !seen.insert(&node.id) {
                return Err(format!("Duplicate node ID: {}", node.id));
            }
        }

        Ok(())
    }
}

impl Node {
    pub fn source(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: NodeKind::Source,
            params: HashMap::new(),
            auth_ref: None,
        }
    }

    pub fn transform(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: NodeKind::Transform,
            params: HashMap::new(),
            auth_ref: None,
        }
    }

    pub fn sink(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: NodeKind::Sink,
            params: HashMap::new(),
            auth_ref: None,
        }
    }

    pub fn router(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: NodeKind::Router,
            params: HashMap::new(),
            auth_ref: None,
        }
    }

    pub fn with_param(
        mut self,
        key: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        self.params.insert(key.into(), value.into());
        self
    }

    pub fn with_auth_ref(mut self, auth_ref: impl Into<String>) -> Self {
        self.auth_ref = Some(auth_ref.into());
        self
    }
}

impl Link {
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            rule: None,
        }
    }

    pub fn with_rule(mut self, rule: Rule) -> Self {
        self.rule = Some(rule);
        self
    }
}

impl Rule {
    pub fn passthrough() -> Self {
        Self {
            rule_type: RuleType::Passthrough,
            conditions: None,
            transform: None,
            split_config: None,
            merge_config: None,
        }
    }

    pub fn filter(conditions: Vec<Condition>) -> Self {
        Self {
            rule_type: RuleType::Filter,
            conditions: Some(conditions),
            transform: None,
            split_config: None,
            merge_config: None,
        }
    }

    pub fn transform(transform: Transform) -> Self {
        Self {
            rule_type: RuleType::Transform,
            conditions: None,
            transform: Some(transform),
            split_config: None,
            merge_config: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_blueprint_validation() {
        let mut blueprint = Blueprint::new("test");

        // Add nodes
        blueprint.add_node(Node::source("input"));
        blueprint.add_node(Node::transform("processor"));
        blueprint.add_node(Node::sink("output"));

        // Add valid links
        blueprint.add_link(Link::new("input", "processor"));
        blueprint.add_link(Link::new("processor", "output"));

        assert!(blueprint.validate().is_ok());

        // Add invalid link
        blueprint.add_link(Link::new("nonexistent", "output"));
        assert!(blueprint.validate().is_err());
    }

    #[test]
    fn test_rule_construction() {
        let filter_rule = Rule::filter(vec![Condition {
            field: "message".to_string(),
            operator: ConditionOperator::Contains,
            value: serde_json::json!("error"),
        }]);

        assert!(matches!(filter_rule.rule_type, RuleType::Filter));
        assert!(filter_rule.conditions.is_some());
        assert_eq!(filter_rule.conditions.unwrap().len(), 1);
    }
}
