//! Session visibility model for configurable access control
//!
//! Controls which agents can access session history based on ownership
//! and spawn chain relationships.

use serde::{Deserialize, Serialize};

/// Visibility level for session history access.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionVisibility {
    /// Only the session owner can access history.
    #[default]
    Own,
    /// Owner and agents in the same spawn tree can access.
    Tree,
    /// All agents of the same type can access.
    Agent,
    /// All agents can access (use with caution).
    All,
}

/// Checks whether an agent can access a session based on visibility policy.
pub struct VisibilityChecker;

impl VisibilityChecker {
    /// Check if the requester can access a session.
    ///
    /// - `visibility`: The session's visibility policy
    /// - `owner`: Agent ID that owns the session
    /// - `requester`: Agent ID requesting access
    /// - `spawn_chain`: Chain of agent IDs from root to requester
    pub fn can_access(
        visibility: SessionVisibility,
        owner: &str,
        requester: &str,
        spawn_chain: &[String],
    ) -> bool {
        match visibility {
            SessionVisibility::Own => owner == requester,
            SessionVisibility::Tree => {
                owner == requester || spawn_chain.contains(&owner.to_string())
            }
            SessionVisibility::Agent => {
                // Same agent type: compare base name (strip instance suffix)
                let owner_base = Self::agent_base_name(owner);
                let requester_base = Self::agent_base_name(requester);
                owner_base == requester_base
            }
            SessionVisibility::All => true,
        }
    }

    /// Clamp visibility for sandboxed agents.
    ///
    /// Sandboxed agents can never have visibility beyond `Tree`.
    #[must_use]
    pub fn clamp_for_sandbox(visibility: SessionVisibility) -> SessionVisibility {
        match visibility {
            SessionVisibility::Agent | SessionVisibility::All => SessionVisibility::Tree,
            other => other,
        }
    }

    /// Extract base agent name by stripping instance suffixes.
    ///
    /// e.g., "code-reviewer-1" → "code-reviewer", "planner" → "planner"
    fn agent_base_name(agent_id: &str) -> &str {
        // Strip trailing "-N" instance suffix
        agent_id
            .rfind('-')
            .and_then(|pos| {
                let suffix = &agent_id[pos + 1..];
                if suffix.chars().all(|c| c.is_ascii_digit()) {
                    Some(&agent_id[..pos])
                } else {
                    None
                }
            })
            .unwrap_or(agent_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_own_visibility() {
        assert!(VisibilityChecker::can_access(
            SessionVisibility::Own,
            "agent-a",
            "agent-a",
            &[],
        ));
        assert!(!VisibilityChecker::can_access(
            SessionVisibility::Own,
            "agent-a",
            "agent-b",
            &[],
        ));
    }

    #[test]
    fn test_tree_visibility() {
        let chain = vec!["orchestrator".to_string(), "agent-a".to_string()];
        // Owner can access own session
        assert!(VisibilityChecker::can_access(
            SessionVisibility::Tree,
            "agent-a",
            "agent-a",
            &chain,
        ));
        // Agent in spawn chain can access
        assert!(VisibilityChecker::can_access(
            SessionVisibility::Tree,
            "orchestrator",
            "agent-a",
            chain.as_slice(),
        ));
        // Unrelated agent cannot
        assert!(!VisibilityChecker::can_access(
            SessionVisibility::Tree,
            "agent-c",
            "agent-a",
            &chain,
        ));
    }

    #[test]
    fn test_agent_visibility_same_type() {
        // Same base agent type, different instances
        assert!(VisibilityChecker::can_access(
            SessionVisibility::Agent,
            "code-reviewer-1",
            "code-reviewer-2",
            &[],
        ));
    }

    #[test]
    fn test_agent_visibility_different_type() {
        assert!(!VisibilityChecker::can_access(
            SessionVisibility::Agent,
            "code-reviewer-1",
            "planner-1",
            &[],
        ));
    }

    #[test]
    fn test_all_visibility() {
        assert!(VisibilityChecker::can_access(
            SessionVisibility::All,
            "any-agent",
            "other-agent",
            &[],
        ));
    }

    #[test]
    fn test_clamp_for_sandbox() {
        assert_eq!(
            VisibilityChecker::clamp_for_sandbox(SessionVisibility::All),
            SessionVisibility::Tree
        );
        assert_eq!(
            VisibilityChecker::clamp_for_sandbox(SessionVisibility::Agent),
            SessionVisibility::Tree
        );
        assert_eq!(
            VisibilityChecker::clamp_for_sandbox(SessionVisibility::Own),
            SessionVisibility::Own
        );
    }

    #[test]
    fn test_agent_base_name() {
        assert_eq!(VisibilityChecker::agent_base_name("reviewer-1"), "reviewer");
        assert_eq!(VisibilityChecker::agent_base_name("planner"), "planner");
        assert_eq!(
            VisibilityChecker::agent_base_name("code-reviewer-42"),
            "code-reviewer"
        );
        // Non-numeric suffix is kept
        assert_eq!(
            VisibilityChecker::agent_base_name("agent-alpha"),
            "agent-alpha"
        );
    }

    #[test]
    fn test_default_visibility_is_own() {
        assert_eq!(SessionVisibility::default(), SessionVisibility::Own);
    }
}
