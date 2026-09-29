//! `message/send` and the compute budget, through the handler itself.

use std::sync::Arc;

use arkavo_budget::BudgetAllocation;
use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_protocol::metrics::MetricsCollector;
use arkavo_protocol::rate_limit::{RateLimitConfig, RateLimiter};
use arkavo_protocol::types::{Message, MessagePart, MessageSendRequest};
use arkavo_tasks::task_executor::{TaskExecutor, TaskExecutorConfig};
use arkavo_tasks::task_store::{SqliteTaskStore, TaskStore};

use super::handle_message_send;
use crate::server::config_helpers::AgentMetadata;
use crate::server::tool_memory::ToolMemory;

/// Send one `message/send` carrying `allocation` to an agent that starts with
/// `budget`. No router is configured, so nothing is executed: what the handler
/// does to the budget is the only effect.
async fn send_with_allocation(
    budget: &arkavo_budget::SharedComputeBudget,
    allocation: &BudgetAllocation,
) {
    let store: Arc<dyn TaskStore> = Arc::new(
        SqliteTaskStore::new_in_memory()
            .await
            .expect("in-memory task store"),
    );
    let executor = Arc::new(TaskExecutor::new(
        store.clone(),
        TaskExecutorConfig::default(),
    ));
    let request = MessageSendRequest {
        message: Message {
            parts: vec![MessagePart::Text {
                content: "Summarise the quarter.".to_string(),
            }],
            metadata: Some(serde_json::json!({ "budget_allocation": allocation })),
        },
        task_id: None,
    };

    handle_message_send(
        &Arc::new(MetricsCollector::new(false)),
        &RateLimiter::new(RateLimitConfig::default()),
        &executor,
        &store,
        &Arc::new(McpRegistry::new()),
        &Arc::new(Conductor::new(InMemoryTaskStore::new())),
        None,
        None,
        None,
        None,
        budget,
        None,
        &Arc::new(tokio::sync::RwLock::new(AgentMetadata::default())),
        &Arc::new(tokio::sync::RwLock::new(ToolMemory::new(10))),
        Arc::new(tokio::sync::Mutex::new(None)),
        #[cfg(feature = "iroh")]
        None,
        request,
    )
    .await
    .expect("message accepted");
}

/// Regression: `metadata.budget_allocation` was applied as sent, so any caller
/// could give the agent whatever compute budget it liked.
#[tokio::test]
async fn a_caller_cannot_raise_the_budget_past_the_configured_one() {
    let budget = arkavo_budget::new_shared_compute_budget();
    let configured = budget.read().await.snapshot();

    send_with_allocation(
        &budget,
        &BudgetAllocation {
            max_tokens: 50_000_000,
            max_cost_usd: 10_000.0,
            max_inferences: 1_000_000,
            max_memory_bytes: u64::MAX,
            max_disk_bytes: u64::MAX,
            max_network_bytes: u64::MAX,
            max_io_ops: u64::MAX,
            max_mcp_calls: 1_000_000,
            ttl_secs: 365 * 24 * 3600,
        },
    )
    .await;

    let after = budget.read().await.snapshot();
    assert_eq!(after.remaining_inferences, configured.remaining_inferences);
    assert_eq!(after.remaining_tokens, configured.remaining_tokens);
    assert_eq!(after.remaining_mcp_calls, configured.remaining_mcp_calls);
    assert_eq!(after.remaining_io_ops, configured.remaining_io_ops);
    assert_eq!(
        after.remaining_network_bytes,
        configured.remaining_network_bytes
    );
    assert_eq!(after.max_memory_bytes, configured.max_memory_bytes);
    assert_eq!(after.max_disk_bytes, configured.max_disk_bytes);
    assert!(after.remaining_cost_usd <= configured.remaining_cost_usd);
    assert!(after.ttl_remaining_secs <= configured.ttl_remaining_secs + 1.0);
}

/// A commander pacing its specialist sends less than the ceiling, and that
/// has to arrive as sent.
#[tokio::test]
async fn a_smaller_allocation_is_applied_as_sent() {
    let budget = arkavo_budget::new_shared_compute_budget();
    let allocation = BudgetAllocation {
        max_inferences: 6,
        max_tokens: 4_000,
        ttl_secs: 120,
        ..BudgetAllocation::default()
    };

    send_with_allocation(&budget, &allocation).await;

    let after = budget.read().await.snapshot();
    assert_eq!(after.remaining_inferences, 6);
    assert_eq!(after.remaining_tokens, 4_000);
    assert!(after.ttl_remaining_secs <= 120.0);
    assert!(after.ttl_remaining_secs > 110.0);
}
