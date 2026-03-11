//! Subagent spawn depth limits and cycle detection
//!
//! Prevents unbounded agent spawn chains and recursive delegation cycles.
//! Integrated into the orchestrator's task spawning path.

use std::collections::HashSet;

/// Maximum default spawn depth for agent chains.
pub const DEFAULT_MAX_DEPTH: u32 = 1;

/// Result of a spawn check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpawnCheckResult {
    /// Spawn is allowed at this depth.
    Allowed,
    /// Maximum spawn depth exceeded.
    DepthExceeded { current: u32, max: u32 },
    /// Agent already in the spawn chain (cycle detected).
    CycleDetected { agent_id: String },
}

/// Guards against unbounded agent spawn chains and delegation cycles.
///
/// Each `SpawnGuard` tracks the current depth and the set of agents
/// already in the spawn chain. Calling `child()` creates a new guard
/// with incremented depth for the spawned agent.
#[derive(Debug, Clone)]
pub struct SpawnGuard {
    max_depth: u32,
    current_depth: u32,
    visited: HashSet<String>,
}

impl SpawnGuard {
    /// Create a root-level spawn guard with default max depth.
    #[must_use]
    pub fn new() -> Self {
        Self {
            max_depth: DEFAULT_MAX_DEPTH,
            current_depth: 0,
            visited: HashSet::new(),
        }
    }

    /// Create a root-level spawn guard with a custom max depth.
    #[must_use]
    pub fn with_max_depth(max_depth: u32) -> Self {
        Self {
            max_depth,
            current_depth: 0,
            visited: HashSet::new(),
        }
    }

    /// Check whether spawning the given agent is allowed.
    pub fn check_spawn(&self, agent_id: &str) -> SpawnCheckResult {
        if self.visited.contains(agent_id) {
            return SpawnCheckResult::CycleDetected {
                agent_id: agent_id.to_string(),
            };
        }

        if self.current_depth >= self.max_depth {
            return SpawnCheckResult::DepthExceeded {
                current: self.current_depth,
                max: self.max_depth,
            };
        }

        SpawnCheckResult::Allowed
    }

    /// Create a child guard for a spawned agent, incrementing depth.
    ///
    /// The child inherits the visited set with the new agent added.
    #[must_use]
    pub fn child(&self, agent_id: &str) -> Self {
        let mut visited = self.visited.clone();
        visited.insert(agent_id.to_string());
        Self {
            max_depth: self.max_depth,
            current_depth: self.current_depth + 1,
            visited,
        }
    }

    /// Current spawn depth.
    #[must_use]
    pub fn current_depth(&self) -> u32 {
        self.current_depth
    }

    /// Maximum allowed spawn depth.
    #[must_use]
    pub fn max_depth(&self) -> u32 {
        self.max_depth
    }

    /// Set of agent IDs already in the spawn chain.
    #[must_use]
    pub fn visited_agents(&self) -> &HashSet<String> {
        &self.visited
    }
}

impl Default for SpawnGuard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_root_guard_allows_spawn() {
        let guard = SpawnGuard::new();
        assert_eq!(guard.check_spawn("agent-a"), SpawnCheckResult::Allowed);
        assert_eq!(guard.current_depth(), 0);
    }

    #[test]
    fn test_default_max_depth_is_one() {
        let guard = SpawnGuard::new();
        assert_eq!(guard.max_depth(), DEFAULT_MAX_DEPTH);
        assert_eq!(guard.check_spawn("agent-a"), SpawnCheckResult::Allowed);

        let child = guard.child("agent-a");
        assert_eq!(child.current_depth(), 1);
        // At depth 1 with max_depth 1, further spawns are blocked
        assert_eq!(
            child.check_spawn("agent-b"),
            SpawnCheckResult::DepthExceeded { current: 1, max: 1 }
        );
    }

    #[test]
    fn test_custom_max_depth() {
        let guard = SpawnGuard::with_max_depth(3);

        let child1 = guard.child("agent-a");
        assert_eq!(child1.check_spawn("agent-b"), SpawnCheckResult::Allowed);

        let child2 = child1.child("agent-b");
        assert_eq!(child2.check_spawn("agent-c"), SpawnCheckResult::Allowed);

        let child3 = child2.child("agent-c");
        assert_eq!(
            child3.check_spawn("agent-d"),
            SpawnCheckResult::DepthExceeded { current: 3, max: 3 }
        );
    }

    #[test]
    fn test_cycle_detection() {
        let guard = SpawnGuard::with_max_depth(10);

        let child1 = guard.child("agent-a");
        let child2 = child1.child("agent-b");

        // Trying to spawn agent-a again creates a cycle
        assert_eq!(
            child2.check_spawn("agent-a"),
            SpawnCheckResult::CycleDetected {
                agent_id: "agent-a".to_string()
            }
        );
    }

    #[test]
    fn test_cycle_takes_priority_over_depth() {
        let guard = SpawnGuard::with_max_depth(1);
        let child = guard.child("agent-a");

        // Both depth exceeded and cycle apply — cycle detected first
        assert_eq!(
            child.check_spawn("agent-a"),
            SpawnCheckResult::CycleDetected {
                agent_id: "agent-a".to_string()
            }
        );
    }

    #[test]
    fn test_visited_agents_tracked() {
        let guard = SpawnGuard::with_max_depth(5);
        let child = guard.child("agent-a").child("agent-b");

        assert!(child.visited_agents().contains("agent-a"));
        assert!(child.visited_agents().contains("agent-b"));
        assert!(!child.visited_agents().contains("agent-c"));
    }

    #[test]
    fn test_zero_max_depth_blocks_all() {
        let guard = SpawnGuard::with_max_depth(0);
        assert_eq!(
            guard.check_spawn("agent-a"),
            SpawnCheckResult::DepthExceeded { current: 0, max: 0 }
        );
    }
}
