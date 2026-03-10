use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Per-agent compute budget allocation, managed by the commander.
///
/// Specialists check this before each orchestrator tick.
/// When exhausted or expired, they enter passive mode (sleep longer).
/// Refreshed when the commander delegates a task or broadcasts state.
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
}

impl AgentComputeBudget {
    pub fn new_passive() -> Self {
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
            expires_at: Instant::now(),
        }
    }

    pub fn has_remaining(&self) -> bool {
        self.remaining_inferences > 0
            && self.remaining_tokens > 0
            && self.remaining_cost_usd > 0.0
            && self.remaining_network_bytes > 0
            && self.remaining_io_ops > 0
            && self.remaining_mcp_calls > 0
            && self.used_memory_bytes <= self.max_memory_bytes
            && self.used_disk_bytes <= self.max_disk_bytes
            && Instant::now() < self.expires_at
    }

    pub fn consume_inference(&mut self, tokens: u64, cost: f64) {
        self.remaining_inferences = self.remaining_inferences.saturating_sub(1);
        self.remaining_tokens = self.remaining_tokens.saturating_sub(tokens);
        self.remaining_cost_usd = (self.remaining_cost_usd - cost).max(0.0);
    }

    pub fn consume_mcp_call(&mut self) {
        self.remaining_mcp_calls = self.remaining_mcp_calls.saturating_sub(1);
    }

    pub fn consume_network(&mut self, bytes: u64) {
        self.remaining_network_bytes = self.remaining_network_bytes.saturating_sub(bytes);
    }

    pub fn consume_io_ops(&mut self, ops: u64) {
        self.remaining_io_ops = self.remaining_io_ops.saturating_sub(ops);
    }

    pub fn update_memory_usage(&mut self, bytes: u64) {
        self.used_memory_bytes = bytes;
    }

    pub fn update_disk_usage(&mut self, bytes: u64) {
        self.used_disk_bytes = bytes;
    }

    pub fn refresh(&mut self, allocation: &BudgetAllocation) {
        self.remaining_tokens = allocation.max_tokens;
        self.remaining_cost_usd = allocation.max_cost_usd;
        self.remaining_inferences = allocation.max_inferences;
        self.max_memory_bytes = allocation.max_memory_bytes;
        self.max_disk_bytes = allocation.max_disk_bytes;
        self.remaining_network_bytes = allocation.max_network_bytes;
        self.remaining_io_ops = allocation.max_io_ops;
        self.remaining_mcp_calls = allocation.max_mcp_calls;
        self.expires_at = Instant::now() + Duration::from_secs(allocation.ttl_secs);
    }

    pub fn status_label(&self) -> &'static str {
        if !self.has_remaining() {
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
        let ttl_remaining = self
            .expires_at
            .checked_duration_since(Instant::now())
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
            has_remaining: self.has_remaining(),
            status: self.status_label().to_string(),
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

/// Shared compute budget handle for thread-safe access.
pub type SharedComputeBudget = Arc<RwLock<AgentComputeBudget>>;

pub fn new_shared_compute_budget() -> SharedComputeBudget {
    let mut budget = AgentComputeBudget::new_passive();
    budget.refresh(&BudgetPolicy::allocate(UrgencyLevel::Medium, 0, 0));
    Arc::new(RwLock::new(budget))
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
