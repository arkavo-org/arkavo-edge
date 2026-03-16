use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// A vector clock for tracking causal ordering across agents.
///
/// Each agent maintains a logical counter. The clock enables detection of
/// concurrent vs causally-ordered events in the CRDT merge pipeline.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorClock {
    clocks: BTreeMap<String, u64>,
}

impl VectorClock {
    /// Create a new empty vector clock.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create from a raw map (e.g., deserialized from gossip messages).
    pub fn from_map(clocks: BTreeMap<String, u64>) -> Self {
        Self { clocks }
    }

    /// Get the raw map (for gossip serialization without cross-crate type dependency).
    pub fn to_map(&self) -> &BTreeMap<String, u64> {
        &self.clocks
    }

    /// Increment the counter for the given agent.
    pub fn increment(&mut self, agent_id: &str) {
        let counter = self.clocks.entry(agent_id.to_string()).or_insert(0);
        *counter += 1;
    }

    /// Get the counter value for an agent, defaulting to 0.
    pub fn get(&self, agent_id: &str) -> u64 {
        self.clocks.get(agent_id).copied().unwrap_or(0)
    }

    /// Merge another clock into this one (point-wise maximum).
    pub fn merge(&mut self, other: &VectorClock) {
        for (agent, &count) in &other.clocks {
            let entry = self.clocks.entry(agent.clone()).or_insert(0);
            *entry = (*entry).max(count);
        }
    }

    /// Returns true if this clock dominates (happens-after) the other.
    ///
    /// Dominates means every entry in self >= other, and at least one is strictly greater.
    pub fn dominates(&self, other: &VectorClock) -> bool {
        let mut strictly_greater = false;
        for (agent, &count) in &other.clocks {
            let self_count = self.get(agent);
            if self_count < count {
                return false;
            }
            if self_count > count {
                strictly_greater = true;
            }
        }
        // Also check entries in self not in other — those are strictly greater than 0
        for (agent, &count) in &self.clocks {
            if count > 0 && other.get(agent) == 0 {
                strictly_greater = true;
            }
        }
        strictly_greater
    }

    /// Returns true if neither clock dominates the other (concurrent events).
    pub fn concurrent(&self, other: &VectorClock) -> bool {
        let a_dominates = self.dominates(other);
        let b_dominates = other.dominates(self);
        !a_dominates && !b_dominates && self != other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_clock_is_empty() {
        let clock = VectorClock::new();
        assert_eq!(clock.get("agent-1"), 0);
    }

    #[test]
    fn increment_creates_entry() {
        let mut clock = VectorClock::new();
        clock.increment("agent-1");
        assert_eq!(clock.get("agent-1"), 1);
        clock.increment("agent-1");
        assert_eq!(clock.get("agent-1"), 2);
    }

    #[test]
    fn merge_takes_maximum() {
        let mut a = VectorClock::new();
        a.increment("x");
        a.increment("x");
        a.increment("y");

        let mut b = VectorClock::new();
        b.increment("x");
        b.increment("y");
        b.increment("y");
        b.increment("z");

        a.merge(&b);
        assert_eq!(a.get("x"), 2); // max(2, 1)
        assert_eq!(a.get("y"), 2); // max(1, 2)
        assert_eq!(a.get("z"), 1); // max(0, 1)
    }

    #[test]
    fn dominates_strict() {
        let mut a = VectorClock::new();
        a.increment("x");
        a.increment("x");

        let mut b = VectorClock::new();
        b.increment("x");

        assert!(a.dominates(&b));
        assert!(!b.dominates(&a));
    }

    #[test]
    fn equal_clocks_dont_dominate() {
        let mut a = VectorClock::new();
        a.increment("x");
        let b = a.clone();
        assert!(!a.dominates(&b));
        assert!(!b.dominates(&a));
    }

    #[test]
    fn concurrent_detection() {
        let mut a = VectorClock::new();
        a.increment("x");

        let mut b = VectorClock::new();
        b.increment("y");

        assert!(a.concurrent(&b));
        assert!(b.concurrent(&a));
    }

    #[test]
    fn not_concurrent_when_dominated() {
        let mut a = VectorClock::new();
        a.increment("x");
        a.increment("x");

        let mut b = VectorClock::new();
        b.increment("x");

        assert!(!a.concurrent(&b));
    }

    #[test]
    fn not_concurrent_when_equal() {
        let mut a = VectorClock::new();
        a.increment("x");
        let b = a.clone();
        assert!(!a.concurrent(&b));
    }

    #[test]
    fn from_map_roundtrip() {
        let mut map = BTreeMap::new();
        map.insert("agent-1".into(), 3);
        map.insert("agent-2".into(), 7);
        let clock = VectorClock::from_map(map.clone());
        assert_eq!(clock.to_map(), &map);
    }

    #[test]
    fn serde_roundtrip() {
        let mut clock = VectorClock::new();
        clock.increment("a");
        clock.increment("b");
        clock.increment("b");
        let json = serde_json::to_string(&clock).unwrap();
        let restored: VectorClock = serde_json::from_str(&json).unwrap();
        assert_eq!(clock, restored);
    }

    #[test]
    fn dominates_with_disjoint_agents() {
        let mut a = VectorClock::new();
        a.increment("x");
        a.increment("y");

        let mut b = VectorClock::new();
        b.increment("x");
        // b has no "y" entry — a dominates since a.y > 0 = b.y

        assert!(a.dominates(&b));
    }

    #[test]
    fn merge_is_commutative() {
        let mut a = VectorClock::new();
        a.increment("x");
        a.increment("x");

        let mut b = VectorClock::new();
        b.increment("y");

        let mut ab = a.clone();
        ab.merge(&b);
        let mut ba = b.clone();
        ba.merge(&a);
        assert_eq!(ab, ba);
    }
}
