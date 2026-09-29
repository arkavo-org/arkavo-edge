mod clock;
#[cfg(test)]
mod window_tests;

pub use clock::BudgetClock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Per-agent compute budget: what the agent may spend before its window ends.
///
/// A window is started in one of two ways. A caller grants one with
/// [`refresh`](Self::refresh), which is how a commander paces a specialist it
/// delegates to. An agent that was given an allocation of its own starts one
/// for itself, and only once the window before it has ended: a budget spent
/// inside its window stays spent until then.
///
/// An agent with no allocation of its own works only on what it is granted,
/// and is passive between grants.
#[derive(Debug, Clone)]
pub struct AgentComputeBudget {
    pub remaining_tokens: u64,
    pub remaining_cost_usd: f64,
    pub remaining_inferences: u32,

    // Additional constraints
    pub max_memory_bytes: u64,
    pub used_memory_bytes: u64,
    pub max_disk_bytes: u64,
    pub used_disk_bytes: u64,
    pub remaining_network_bytes: u64,
    pub remaining_io_ops: u64,
    pub remaining_mcp_calls: u32,

    pub expires_at: Instant,

    /// What the agent grants itself once a window has ended. `None` for an
    /// agent that works only on what a caller grants it.
    own_allocation: Option<BudgetAllocation>,
    clock: BudgetClock,
}

impl AgentComputeBudget {
    /// A budget with nothing to spend until a caller grants something.
    pub fn new_passive() -> Self {
        Self::passive_on(BudgetClock::system())
    }

    /// [`new_passive`](Self::new_passive), reading the time from `clock`.
    pub fn passive_on(clock: BudgetClock) -> Self {
        Self {
            remaining_tokens: 0,
            remaining_cost_usd: 0.0,
            remaining_inferences: 0,
            max_memory_bytes: 0,
            used_memory_bytes: 0,
            max_disk_bytes: 0,
            used_disk_bytes: 0,
            remaining_network_bytes: 0,
            remaining_io_ops: 0,
            remaining_mcp_calls: 0,
            expires_at: clock.now(),
            own_allocation: None,
            clock,
        }
    }

    /// A budget for an agent that paces itself with `allocation`, starting
    /// with a full window.
    ///
    /// Nothing outside the agent has to keep it supplied: when a window ends,
    /// whether the agent's own or one a caller granted, the next thing the
    /// agent does starts a new window of its own.
    pub fn self_managed(allocation: BudgetAllocation, clock: BudgetClock) -> Self {
        let mut budget = Self::passive_on(clock);
        budget.refresh(&allocation);
        budget.own_allocation = Some(allocation);
        budget
    }

    /// The budget as it will be once the agent starts its own next window, if
    /// the window in force has ended and the agent has an allocation of its
    /// own. This is the only refill that no caller granted.
    fn renewed(&self) -> Option<Self> {
        let own = self.own_allocation.as_ref()?;
        if self.clock.now() < self.expires_at {
            return None;
        }
        let mut renewed = self.clone();
        renewed.refresh(own);
        Some(renewed)
    }

    /// Start the agent's own next window if the window in force has ended.
    ///
    /// Reading the budget through [`has_remaining`](Self::has_remaining) or
    /// [`snapshot`](Self::snapshot) already accounts for a window that has
    /// ended, and spending from it starts the next one. This is for a caller
    /// about to read the counters directly. Returns whether a window started.
    pub fn renew_if_expired(&mut self) -> bool {
        let Some(renewed) = self.renewed() else {
            return false;
        };
        *self = renewed;
        true
    }

    pub fn has_remaining(&self) -> bool {
        self.renewed()
            .map_or_else(|| self.window_has_remaining(), |w| w.window_has_remaining())
    }

    fn window_has_remaining(&self) -> bool {
        self.remaining_inferences > 0
            && self.remaining_tokens > 0
            && self.remaining_cost_usd > 0.0
            && self.remaining_network_bytes > 0
            && self.remaining_io_ops > 0
            && self.remaining_mcp_calls > 0
            && self.used_memory_bytes <= self.max_memory_bytes
            && self.used_disk_bytes <= self.max_disk_bytes
            && self.clock.now() < self.expires_at
    }

    pub fn consume_inference(&mut self, tokens: u64, cost: f64) {
        self.renew_if_expired();
        self.remaining_inferences = self.remaining_inferences.saturating_sub(1);
        self.remaining_tokens = self.remaining_tokens.saturating_sub(tokens);
        self.remaining_cost_usd = (self.remaining_cost_usd - cost).max(0.0);
    }

    pub fn consume_mcp_call(&mut self) {
        self.renew_if_expired();
        self.remaining_mcp_calls = self.remaining_mcp_calls.saturating_sub(1);
    }

    pub fn consume_network(&mut self, bytes: u64) {
        self.renew_if_expired();
        self.remaining_network_bytes = self.remaining_network_bytes.saturating_sub(bytes);
    }

    pub fn consume_io_ops(&mut self, ops: u64) {
        self.renew_if_expired();
        self.remaining_io_ops = self.remaining_io_ops.saturating_sub(ops);
    }

    pub fn update_memory_usage(&mut self, bytes: u64) {
        self.used_memory_bytes = bytes;
    }

    pub fn update_disk_usage(&mut self, bytes: u64) {
        self.used_disk_bytes = bytes;
    }

    /// Start a window holding `allocation`, replacing the one in force.
    pub fn refresh(&mut self, allocation: &BudgetAllocation) {
        self.remaining_tokens = allocation.max_tokens;
        self.remaining_cost_usd = allocation.max_cost_usd;
        self.remaining_inferences = allocation.max_inferences;
        self.max_memory_bytes = allocation.max_memory_bytes;
        self.max_disk_bytes = allocation.max_disk_bytes;
        self.remaining_network_bytes = allocation.max_network_bytes;
        self.remaining_io_ops = allocation.max_io_ops;
        self.remaining_mcp_calls = allocation.max_mcp_calls;
        self.expires_at = self.clock.now() + Duration::from_secs(allocation.ttl_secs);
    }

    pub fn status_label(&self) -> &'static str {
        self.renewed()
            .map_or_else(|| self.window_status_label(), |w| w.window_status_label())
    }

    fn window_status_label(&self) -> &'static str {
        if !self.window_has_remaining() {
            if self.remaining_inferences == 0
                && self.remaining_tokens == 0
                && self.remaining_cost_usd <= 0.0
            {
                "exhausted"
            } else {
                "passive"
            }
        } else {
            "active"
        }
    }
}

impl Default for AgentComputeBudget {
    fn default() -> Self {
        Self::new_passive()
    }
}

/// Serializable snapshot of compute budget state for network monitoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeBudgetSnapshot {
    pub remaining_tokens: u64,
    pub remaining_cost_usd: f64,
    pub remaining_inferences: u32,
    pub max_memory_bytes: u64,
    pub used_memory_bytes: u64,
    pub max_disk_bytes: u64,
    pub used_disk_bytes: u64,
    pub remaining_network_bytes: u64,
    pub remaining_io_ops: u64,
    pub remaining_mcp_calls: u32,
    pub has_remaining: bool,
    pub status: String,
    pub ttl_remaining_secs: f64,
}

impl AgentComputeBudget {
    pub fn snapshot(&self) -> ComputeBudgetSnapshot {
        self.renewed()
            .map_or_else(|| self.window_snapshot(), |w| w.window_snapshot())
    }

    fn window_snapshot(&self) -> ComputeBudgetSnapshot {
        let ttl_remaining = self
            .expires_at
            .checked_duration_since(self.clock.now())
            .map(|d| d.as_secs_f64())
            .unwrap_or(0.0);

        ComputeBudgetSnapshot {
            remaining_tokens: self.remaining_tokens,
            remaining_cost_usd: self.remaining_cost_usd,
            remaining_inferences: self.remaining_inferences,
            max_memory_bytes: self.max_memory_bytes,
            used_memory_bytes: self.used_memory_bytes,
            max_disk_bytes: self.max_disk_bytes,
            used_disk_bytes: self.used_disk_bytes,
            remaining_network_bytes: self.remaining_network_bytes,
            remaining_io_ops: self.remaining_io_ops,
            remaining_mcp_calls: self.remaining_mcp_calls,
            has_remaining: self.window_has_remaining(),
            status: self.window_status_label().to_string(),
            ttl_remaining_secs: ttl_remaining,
        }
    }
}

/// Budget allocation sent from commander to specialist in task metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BudgetAllocation {
    pub max_tokens: u64,
    pub max_cost_usd: f64,
    pub max_inferences: u32,
    pub max_memory_bytes: u64,
    pub max_disk_bytes: u64,
    pub max_network_bytes: u64,
    pub max_io_ops: u64,
    pub max_mcp_calls: u32,
    pub ttl_secs: u64,
}

impl Default for BudgetAllocation {
    fn default() -> Self {
        Self {
            max_tokens: 10_000,
            max_cost_usd: 0.50,
            max_inferences: 8,
            max_memory_bytes: 2 * 1024 * 1024 * 1024, // 2GB
            max_disk_bytes: 10 * 1024 * 1024 * 1024,  // 10GB
            max_network_bytes: 100 * 1024 * 1024,     // 100MB
            max_io_ops: 100_000,
            max_mcp_calls: 100,
            ttl_secs: 120,
        }
    }
}

impl BudgetAllocation {
    /// What an agent grants itself when nothing outside it supplies a budget.
    ///
    /// Generous next to what a commander grants a specialist, so that an
    /// autonomous agent gets through several tool-loop iterations a window.
    pub fn self_managed() -> Self {
        Self {
            max_inferences: 32,
            max_tokens: 100_000,
            ttl_secs: 600,
            ..Self::default()
        }
    }
}

/// Shared compute budget handle for thread-safe access.
pub type SharedComputeBudget = Arc<RwLock<AgentComputeBudget>>;

/// The budget an agent starts with: one it manages itself.
pub fn new_shared_compute_budget() -> SharedComputeBudget {
    new_shared_compute_budget_on(BudgetClock::system())
}

/// [`new_shared_compute_budget`], reading the time from `clock`.
pub fn new_shared_compute_budget_on(clock: BudgetClock) -> SharedComputeBudget {
    Arc::new(RwLock::new(AgentComputeBudget::self_managed(
        BudgetAllocation::self_managed(),
        clock,
    )))
}

/// Urgency level derived from game state observation (e.g., alert count).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UrgencyLevel {
    Low,
    Medium,
    High,
    Critical,
}

/// Computes per-specialist `BudgetAllocation` from runtime signals.
pub struct BudgetPolicy;

impl BudgetPolicy {
    const MB: u64 = 1024 * 1024;

    /// Higher urgency → more inferences + shorter TTL (more frequent refresh).
    /// Pending backlog → reduce inferences to avoid overloading the specialist.
    /// Memory is capped per-specialist to prevent any single agent from exhausting RAM.
    ///
    /// Memory budget accounts for model weight loading (~550MB-2.5GB for local models)
    /// plus KV cache and runtime overhead. On a 16GB system with 4 agents,
    /// each specialist gets up to 2GB (leaves headroom for commander + OS).
    pub fn allocate(
        urgency: UrgencyLevel,
        pending_tasks: u32,
        per_agent_bytes: u64,
    ) -> BudgetAllocation {
        let (max_inferences, ttl_secs, max_memory_mb): (u32, u64, u64) = match urgency {
            UrgencyLevel::Low => (6, 120, 1024),
            UrgencyLevel::Medium => (8, 90, 2048),
            UrgencyLevel::High => (12, 60, 2048),
            UrgencyLevel::Critical => (16, 45, 2048),
        };
        let max_inferences = if pending_tasks >= 2 {
            max_inferences.saturating_sub(1).max(1)
        } else {
            max_inferences
        };
        let max_memory_bytes = if per_agent_bytes > 0 {
            per_agent_bytes
        } else {
            max_memory_mb * Self::MB
        };
        BudgetAllocation {
            max_inferences,
            ttl_secs,
            max_memory_bytes,
            max_disk_bytes: 512 * Self::MB,
            max_network_bytes: 50 * Self::MB,
            ..BudgetAllocation::default()
        }
    }
}

/// Summary of per-agent budget allocation for dashboard display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentAllocationSummary {
    pub agent_id: String,
    pub allocated_usd: f64,
    pub spent_usd: f64,
    pub remaining_usd: f64,
    pub allocated_tokens: u64,
    pub tokens_used: u64,
    pub model_type: String,
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_passive_budget_has_no_remaining() {
        let budget = AgentComputeBudget::new_passive();
        assert!(!budget.has_remaining());
        assert_eq!(budget.status_label(), "exhausted");
    }

    #[tokio::test]
    async fn test_shared_budget_starts_active() {
        let budget = new_shared_compute_budget();
        let b = budget.read().await;
        assert!(b.has_remaining());
        assert_eq!(b.status_label(), "active");
    }

    #[test]
    fn test_refresh_enables_budget() {
        let mut budget = AgentComputeBudget::new_passive();
        let allocation = BudgetAllocation::default();
        budget.refresh(&allocation);
        assert!(budget.has_remaining());
        assert_eq!(budget.status_label(), "active");
    }

    #[test]
    fn test_consume_depletes_budget() {
        let mut budget = AgentComputeBudget::new_passive();
        budget.refresh(&BudgetAllocation {
            max_tokens: 1000,
            max_cost_usd: 0.10,
            max_inferences: 1,
            ttl_secs: 60,
            ..Default::default()
        });
        assert!(budget.has_remaining());

        budget.consume_inference(500, 0.05);
        assert!(!budget.has_remaining()); // 0 inferences left
        assert_eq!(budget.status_label(), "passive");
    }

    #[test]
    fn test_budget_policy_low_urgency() {
        let alloc = BudgetPolicy::allocate(UrgencyLevel::Low, 0, 0);
        assert_eq!(alloc.max_inferences, 6);
        assert_eq!(alloc.ttl_secs, 120);
    }

    #[test]
    fn test_budget_policy_critical_urgency() {
        let alloc = BudgetPolicy::allocate(UrgencyLevel::Critical, 0, 0);
        assert_eq!(alloc.max_inferences, 16);
        assert_eq!(alloc.ttl_secs, 45);
    }

    #[test]
    fn test_budget_policy_backs_off_with_pending() {
        let alloc = BudgetPolicy::allocate(UrgencyLevel::High, 3, 0);
        assert_eq!(alloc.max_inferences, 11); // 12 - 1
        assert_eq!(alloc.ttl_secs, 60);
    }

    #[test]
    fn test_budget_policy_pending_never_below_one() {
        let alloc = BudgetPolicy::allocate(UrgencyLevel::Low, 10, 0);
        assert_eq!(alloc.max_inferences, 5); // 6 - 1
    }

    #[test]
    fn test_budget_policy_medium_default_values() {
        let alloc = BudgetPolicy::allocate(UrgencyLevel::Medium, 0, 0);
        assert_eq!(alloc.max_inferences, 8);
        assert_eq!(alloc.ttl_secs, 90);
        assert_eq!(alloc.max_tokens, BudgetAllocation::default().max_tokens);
    }

    #[test]
    fn test_memory_budget_realistic_for_16gb_system() {
        // On a 16GB system with 4 agents, each specialist gets 2GB max (fallback)
        let alloc = BudgetPolicy::allocate(UrgencyLevel::Medium, 0, 0);
        assert_eq!(alloc.max_memory_bytes, 2048 * 1024 * 1024); // 2GB
        // Low urgency gets 1GB — enough for qwen3.5-0.8b (550MB) but not ministral-3b (2.5GB)
        let low = BudgetPolicy::allocate(UrgencyLevel::Low, 0, 0);
        assert_eq!(low.max_memory_bytes, 1024 * 1024 * 1024); // 1GB
    }

    #[test]
    fn test_dynamic_memory_overrides_hardcoded() {
        let per_agent = 33 * 1024 * 1024 * 1024_u64; // 33 GB
        let alloc = BudgetPolicy::allocate(UrgencyLevel::Low, 0, per_agent);
        assert_eq!(alloc.max_memory_bytes, per_agent);
        // Urgency still controls inferences/TTL, not memory
        assert_eq!(alloc.max_inferences, 6);
        assert_eq!(alloc.ttl_secs, 120);
    }

    #[test]
    fn test_fallback_when_per_agent_bytes_zero() {
        let low = BudgetPolicy::allocate(UrgencyLevel::Low, 0, 0);
        assert_eq!(low.max_memory_bytes, 1024 * 1024 * 1024); // 1GB hardcoded
        let med = BudgetPolicy::allocate(UrgencyLevel::Medium, 0, 0);
        assert_eq!(med.max_memory_bytes, 2048 * 1024 * 1024); // 2GB hardcoded
    }

    #[test]
    fn test_memory_exceeds_budget_blocks() {
        let mut budget = AgentComputeBudget::new_passive();
        budget.refresh(&BudgetAllocation {
            max_tokens: 1000,
            max_cost_usd: 0.10,
            max_inferences: 5,
            max_memory_bytes: 1024 * 1024 * 1024, // 1GB
            ttl_secs: 60,
            ..Default::default()
        });
        assert!(budget.has_remaining());

        // Simulate model loading that exceeds budget
        budget.update_memory_usage(2 * 1024 * 1024 * 1024); // 2GB used
        assert!(!budget.has_remaining()); // blocked by memory
    }

    #[test]
    fn test_expired_budget_has_no_remaining() {
        let mut budget = AgentComputeBudget::new_passive();
        budget.refresh(&BudgetAllocation {
            max_tokens: 1000,
            max_cost_usd: 0.10,
            max_inferences: 5,
            ttl_secs: 0, // expires immediately
            ..Default::default()
        });
        // TTL 0 means expires_at = now, so has_remaining should be false
        std::thread::sleep(Duration::from_millis(1));
        assert!(!budget.has_remaining());
    }
}
