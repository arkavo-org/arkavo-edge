use crate::dsl::Blueprint;
use std::collections::HashSet;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("Duplicate node ID: {0}")]
    DuplicateNodeId(String),

    #[error("Link references non-existent node: {0}")]
    NonExistentNode(String),

    #[error("Cyclic dependency detected involving node: {0}")]
    CyclicDependency(String),

    #[error("Source node {0} has incoming connections")]
    SourceWithIncoming(String),

    #[error("Sink node {0} has outgoing connections")]
    SinkWithOutgoing(String),

    #[error("Node {0} has no connections")]
    IsolatedNode(String),

    #[error("Invalid rule configuration: {0}")]
    InvalidRule(String),

    #[error("Invalid auth reference format: {0}")]
    InvalidAuthRef(String),
}

pub struct BlueprintValidator;

impl BlueprintValidator {
    pub fn validate(blueprint: &Blueprint) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        // Check for duplicate node IDs
        let mut seen_ids = HashSet::new();
        for node in &blueprint.nodes {
            if !seen_ids.insert(&node.id) {
                errors.push(ValidationError::DuplicateNodeId(node.id.clone()));
            }
        }

        // Check all links reference existing nodes
        let node_ids: HashSet<_> = blueprint.nodes.iter().map(|n| &n.id).collect();
        for link in &blueprint.links {
            if !node_ids.contains(&link.from) {
                errors.push(ValidationError::NonExistentNode(link.from.clone()));
            }
            if !node_ids.contains(&link.to) {
                errors.push(ValidationError::NonExistentNode(link.to.clone()));
            }
        }

        // Check for node type constraints
        for node in &blueprint.nodes {
            match node.kind {
                crate::dsl::NodeKind::Source => {
                    // Sources shouldn't have incoming connections
                    if blueprint.links.iter().any(|l| l.to == node.id) {
                        errors.push(ValidationError::SourceWithIncoming(node.id.clone()));
                    }
                }
                crate::dsl::NodeKind::Sink => {
                    // Sinks shouldn't have outgoing connections
                    if blueprint.links.iter().any(|l| l.from == node.id) {
                        errors.push(ValidationError::SinkWithOutgoing(node.id.clone()));
                    }
                }
                _ => {}
            }
        }

        // Check for isolated nodes (except sources and sinks can be terminal)
        for node in &blueprint.nodes {
            let has_incoming = blueprint.links.iter().any(|l| l.to == node.id);
            let has_outgoing = blueprint.links.iter().any(|l| l.from == node.id);

            match node.kind {
                crate::dsl::NodeKind::Transform | crate::dsl::NodeKind::Router => {
                    if !has_incoming || !has_outgoing {
                        errors.push(ValidationError::IsolatedNode(node.id.clone()));
                    }
                }
                _ => {}
            }
        }

        // Check for cycles
        if let Some(cycle_node) = detect_cycle(blueprint) {
            errors.push(ValidationError::CyclicDependency(cycle_node));
        }

        // Validate auth references
        for node in &blueprint.nodes {
            if let Some(auth_ref) = &node.auth_ref {
                if !is_valid_env_var_ref(auth_ref) {
                    errors.push(ValidationError::InvalidAuthRef(auth_ref.clone()));
                }
            }
        }

        // Validate rules
        for link in &blueprint.links {
            if let Some(rule) = &link.rule {
                if let Err(e) = validate_rule(rule) {
                    errors.push(e);
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn detect_cycle(blueprint: &Blueprint) -> Option<String> {
    use std::collections::{HashMap, VecDeque};

    // Build adjacency list
    let mut graph: HashMap<&str, Vec<&str>> = HashMap::new();
    let mut in_degree: HashMap<&str, usize> = HashMap::new();

    // Initialize nodes
    for node in &blueprint.nodes {
        graph.entry(&node.id).or_default();
        in_degree.entry(&node.id).or_insert(0);
    }

    // Build edges
    for link in &blueprint.links {
        graph.get_mut(link.from.as_str())?.push(&link.to);
        *in_degree.get_mut(link.to.as_str())? += 1;
    }

    // Topological sort using Kahn's algorithm
    let mut queue = VecDeque::new();
    for (node, &degree) in &in_degree {
        if degree == 0 {
            queue.push_back(*node);
        }
    }

    let mut sorted_count = 0;
    while let Some(node) = queue.pop_front() {
        sorted_count += 1;

        if let Some(neighbors) = graph.get(node) {
            for &neighbor in neighbors {
                let degree = in_degree.get_mut(neighbor)?;
                *degree -= 1;
                if *degree == 0 {
                    queue.push_back(neighbor);
                }
            }
        }
    }

    // If we couldn't sort all nodes, there's a cycle
    if sorted_count < blueprint.nodes.len() {
        // Find a node that's part of the cycle
        for (node, &degree) in &in_degree {
            if degree > 0 {
                return Some((*node).to_string());
            }
        }
    }

    None
}

fn is_valid_env_var_ref(auth_ref: &str) -> bool {
    // Must be in format ${VAR_NAME}
    if !auth_ref.starts_with("${") || !auth_ref.ends_with('}') {
        return false;
    }

    let var_name = &auth_ref[2..auth_ref.len() - 1];

    // Check valid environment variable name (uppercase letters, numbers, underscores)
    !var_name.is_empty()
        && var_name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn validate_rule(rule: &crate::dsl::Rule) -> Result<(), ValidationError> {
    use crate::dsl::RuleType;

    match rule.rule_type {
        RuleType::Filter => {
            if rule.conditions.as_ref().is_none_or(|c| c.is_empty()) {
                return Err(ValidationError::InvalidRule(
                    "Filter rule must have at least one condition".to_string(),
                ));
            }
        }
        RuleType::Transform => {
            if rule.transform.is_none() {
                return Err(ValidationError::InvalidRule(
                    "Transform rule must have transform configuration".to_string(),
                ));
            }
        }
        RuleType::Split => {
            if rule.split_config.is_none() {
                return Err(ValidationError::InvalidRule(
                    "Split rule must have split configuration".to_string(),
                ));
            }
        }
        RuleType::Merge => {
            if rule.merge_config.is_none() {
                return Err(ValidationError::InvalidRule(
                    "Merge rule must have merge configuration".to_string(),
                ));
            }
        }
        RuleType::Passthrough => {
            // No additional validation needed
        }
    }

    Ok(())
}

// Re-export thiserror for error derive
pub use thiserror;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::{Link, Node};

    #[test]
    fn test_cycle_detection() {
        let mut blueprint = Blueprint::new("test");

        // Create a cycle: A -> B -> C -> A
        blueprint.add_node(Node::transform("A"));
        blueprint.add_node(Node::transform("B"));
        blueprint.add_node(Node::transform("C"));

        blueprint.add_link(Link::new("A", "B"));
        blueprint.add_link(Link::new("B", "C"));
        blueprint.add_link(Link::new("C", "A"));

        let result = BlueprintValidator::validate(&blueprint);
        assert!(result.is_err());

        let errors = result.unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ValidationError::CyclicDependency(_)))
        );
    }

    #[test]
    fn test_env_var_validation() {
        assert!(is_valid_env_var_ref("${API_KEY}"));
        assert!(is_valid_env_var_ref("${OAUTH_TOKEN_123}"));
        assert!(!is_valid_env_var_ref("${api-key}")); // Lowercase not allowed
        assert!(!is_valid_env_var_ref("API_KEY")); // Missing ${}
        assert!(!is_valid_env_var_ref("${}")); // Empty var name
    }
}
