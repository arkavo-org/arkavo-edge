//! A compute budget a caller asks for, held to what the agent was given.
//!
//! A commander refreshes its specialists by sending a `budget_allocation` in
//! the metadata of a `message/send`. The handler cannot tell a commander from
//! any other caller, so the allocation is a request and not an instruction:
//! it may lower the agent's budget or top it back up, and it may never raise
//! any limit past what the agent was configured with.

use arkavo_budget::{AgentComputeBudget, BudgetAllocation};

/// The allocation `metadata` asks for, if it carries a well-formed one.
pub(super) fn requested(metadata: Option<&serde_json::Value>) -> Option<BudgetAllocation> {
    let allocation = metadata?.get("budget_allocation")?;
    serde_json::from_value(allocation.clone()).ok()
}

/// What an agent's compute budget holds when it is first configured.
///
/// Read from a freshly configured budget rather than restated here, so the
/// ceiling cannot drift from the budget the agent actually starts with.
async fn configured() -> BudgetAllocation {
    let fresh = arkavo_budget::new_shared_compute_budget();
    let snapshot = fresh.read().await.snapshot();
    BudgetAllocation {
        max_tokens: snapshot.remaining_tokens,
        max_cost_usd: snapshot.remaining_cost_usd,
        max_inferences: snapshot.remaining_inferences,
        max_memory_bytes: snapshot.max_memory_bytes,
        max_disk_bytes: snapshot.max_disk_bytes,
        max_network_bytes: snapshot.remaining_network_bytes,
        max_io_ops: snapshot.remaining_io_ops,
        max_mcp_calls: snapshot.remaining_mcp_calls,
        // The snapshot reports time left, which is a hair under the window
        // the budget was configured with.
        ttl_secs: snapshot.ttl_remaining_secs.round() as u64,
    }
}

/// `requested` with every limit held to `ceiling`.
fn clamp(requested: &BudgetAllocation, ceiling: &BudgetAllocation) -> BudgetAllocation {
    BudgetAllocation {
        max_tokens: requested.max_tokens.min(ceiling.max_tokens),
        max_cost_usd: requested.max_cost_usd.min(ceiling.max_cost_usd).max(0.0),
        max_inferences: requested.max_inferences.min(ceiling.max_inferences),
        max_memory_bytes: requested.max_memory_bytes.min(ceiling.max_memory_bytes),
        max_disk_bytes: requested.max_disk_bytes.min(ceiling.max_disk_bytes),
        max_network_bytes: requested.max_network_bytes.min(ceiling.max_network_bytes),
        max_io_ops: requested.max_io_ops.min(ceiling.max_io_ops),
        max_mcp_calls: requested.max_mcp_calls.min(ceiling.max_mcp_calls),
        ttl_secs: requested.ttl_secs.min(ceiling.ttl_secs),
    }
}

/// Refresh `budget` with what a caller asked for, within the agent's ceiling.
pub(super) async fn refresh_within_ceiling(
    budget: &mut AgentComputeBudget,
    requested: &BudgetAllocation,
) {
    let granted = clamp(requested, &configured().await);
    if granted.max_inferences < requested.max_inferences
        || granted.max_tokens < requested.max_tokens
        || granted.ttl_secs < requested.ttl_secs
    {
        tracing::warn!(
            requested_inferences = requested.max_inferences,
            granted_inferences = granted.max_inferences,
            requested_tokens = requested.max_tokens,
            granted_tokens = granted.max_tokens,
            requested_ttl_secs = requested.ttl_secs,
            granted_ttl_secs = granted.ttl_secs,
            "Caller asked for more compute budget than this agent is configured with"
        );
    }
    budget.refresh(&granted);
}

#[cfg(test)]
// `#[tokio::test]` expands to `Runtime::block_on`, which the crate's lint set
// disallows in library code. Same waiver as the other async test modules here.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;

    fn greedy() -> BudgetAllocation {
        BudgetAllocation {
            max_tokens: u64::MAX,
            max_cost_usd: 1_000_000.0,
            max_inferences: u32::MAX,
            max_memory_bytes: u64::MAX,
            max_disk_bytes: u64::MAX,
            max_network_bytes: u64::MAX,
            max_io_ops: u64::MAX,
            max_mcp_calls: u32::MAX,
            ttl_secs: u64::MAX,
        }
    }

    /// The ceiling is the budget the server hands a new agent. If the two
    /// ever differ, a caller can either exceed the configuration or be denied
    /// a refresh the configuration allows.
    #[tokio::test]
    async fn the_ceiling_is_the_configured_budget() {
        let ceiling = configured().await;
        let mut refreshed = AgentComputeBudget::new_passive();
        refreshed.refresh(&ceiling);
        let refreshed = refreshed.snapshot();
        let fresh = arkavo_budget::new_shared_compute_budget()
            .read()
            .await
            .snapshot();

        assert_eq!(refreshed.remaining_inferences, fresh.remaining_inferences);
        assert_eq!(refreshed.remaining_tokens, fresh.remaining_tokens);
        assert_eq!(refreshed.remaining_mcp_calls, fresh.remaining_mcp_calls);
        assert_eq!(refreshed.remaining_io_ops, fresh.remaining_io_ops);
        assert_eq!(refreshed.max_memory_bytes, fresh.max_memory_bytes);
        assert_eq!(refreshed.max_disk_bytes, fresh.max_disk_bytes);
        assert!((refreshed.remaining_cost_usd - fresh.remaining_cost_usd).abs() < f64::EPSILON);
        assert!((refreshed.ttl_remaining_secs - fresh.ttl_remaining_secs).abs() < 1.0);
    }

    #[tokio::test]
    async fn every_limit_is_held_to_the_ceiling() {
        let ceiling = configured().await;
        let granted = clamp(&greedy(), &ceiling);

        assert_eq!(granted.max_tokens, ceiling.max_tokens);
        assert_eq!(granted.max_inferences, ceiling.max_inferences);
        assert_eq!(granted.max_memory_bytes, ceiling.max_memory_bytes);
        assert_eq!(granted.max_disk_bytes, ceiling.max_disk_bytes);
        assert_eq!(granted.max_network_bytes, ceiling.max_network_bytes);
        assert_eq!(granted.max_io_ops, ceiling.max_io_ops);
        assert_eq!(granted.max_mcp_calls, ceiling.max_mcp_calls);
        assert_eq!(granted.ttl_secs, ceiling.ttl_secs);
        assert!(granted.max_cost_usd <= ceiling.max_cost_usd);
    }

    /// A commander's ordinary refresh is smaller than the ceiling and has to
    /// arrive as sent, or clamping would change how specialists are paced.
    #[tokio::test]
    async fn a_request_within_the_ceiling_is_granted_as_asked() {
        let modest = arkavo_budget::BudgetPolicy::allocate(
            arkavo_budget::UrgencyLevel::Low,
            0,
            512 * 1024 * 1024,
        );
        let granted = clamp(&modest, &configured().await);

        assert_eq!(granted.max_inferences, modest.max_inferences);
        assert_eq!(granted.max_tokens, modest.max_tokens);
        assert_eq!(granted.ttl_secs, modest.ttl_secs);
        assert_eq!(granted.max_memory_bytes, modest.max_memory_bytes);
    }

    #[test]
    fn a_negative_cost_grants_nothing() {
        let mut request = greedy();
        request.max_cost_usd = -5.0;
        let granted = clamp(&request, &BudgetAllocation::default());
        assert!(granted.max_cost_usd.abs() < f64::EPSILON);
    }

    #[test]
    fn metadata_without_a_well_formed_allocation_asks_for_nothing() {
        assert!(requested(None).is_none());
        assert!(requested(Some(&serde_json::json!({"source": "chat"}))).is_none());
        assert!(requested(Some(&serde_json::json!({"budget_allocation": "lots"}))).is_none());
    }
}
