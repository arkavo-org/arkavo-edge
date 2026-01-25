//! UCP MCP Tools for AI agent payment operations

mod create;
mod execute;
mod list;
mod status;

pub use create::CreatePaymentIntentTool;
pub use execute::ExecutePaymentTool;
pub use list::ListPaymentsTool;
pub use status::GetPaymentStatusTool;

use crate::handler::PaymentHandler;
use crate::handlers::{BudgetPaymentHandler, EvmPaymentHandler};
use crate::policy::{CommerceLimits, CommercePolicy};
use crate::tracker::PaymentTracker;
use crate::types::Currency;
use arkavo_budget::BudgetTracker;
use arkavo_mcp::Tool;
use arkavo_wallet::EvmKeypair;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Shared UCP state for tools
pub struct UcpState {
    pub tracker: Arc<PaymentTracker>,
    pub policy: Arc<CommercePolicy>,
    pub budget_handler: Arc<BudgetPaymentHandler>,
    pub evm_handler: Option<Arc<EvmPaymentHandler>>,
}

impl UcpState {
    pub fn new(budget_tracker: Arc<BudgetTracker>) -> Self {
        let tracker = Arc::new(PaymentTracker::new());
        let policy = Arc::new(CommercePolicy::new(
            budget_tracker.clone(),
            CommerceLimits::default(),
        ));
        let budget_handler = Arc::new(BudgetPaymentHandler::new(budget_tracker));

        Self {
            tracker,
            policy,
            budget_handler,
            evm_handler: None,
        }
    }

    pub fn with_evm_handler(mut self, keypair: Arc<EvmKeypair>, chain_id: u64) -> Self {
        self.evm_handler = Some(Arc::new(EvmPaymentHandler::new(keypair, chain_id)));
        self
    }

    pub(crate) fn get_handler(&self, currency: Currency) -> Option<Arc<dyn PaymentHandler>> {
        match currency {
            Currency::Usd => Some(self.budget_handler.clone()),
            Currency::Eth => self.evm_handler.clone().map(|h| h as Arc<dyn PaymentHandler>),
            _ => None,
        }
    }
}

pub(crate) fn error_response(message: &str) -> Value {
    json!({
        "success": false,
        "error": message
    })
}

pub(crate) fn success_response(data: Value) -> Value {
    json!({
        "success": true,
        "data": data
    })
}

/// Register all UCP tools with a tool registry
pub fn register_tools<R: ToolRegistry>(registry: &mut R, state: Arc<RwLock<UcpState>>) {
    registry.register(Box::new(CreatePaymentIntentTool::new(state.clone())));
    registry.register(Box::new(ExecutePaymentTool::new(state.clone())));
    registry.register(Box::new(GetPaymentStatusTool::new(state.clone())));
    registry.register(Box::new(ListPaymentsTool::new(state)));
}

/// Trait for tool registries
pub trait ToolRegistry {
    fn register(&mut self, tool: Box<dyn Tool>);
}

#[cfg(test)]
mod tests;
