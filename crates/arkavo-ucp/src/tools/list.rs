//! List payments tool

use super::{UcpState, success_response};
use crate::types::PaymentStatus;
use arkavo_mcp::ToolSchema;
use arkavo_mcp_tools::server::Tool;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Tool for listing payments
pub struct ListPaymentsTool {
    state: Arc<RwLock<UcpState>>,
    schema: ToolSchema,
}

impl ListPaymentsTool {
    pub fn new(state: Arc<RwLock<UcpState>>) -> Self {
        Self {
            state,
            schema: ToolSchema {
                name: "ucp_list_payments".to_string(),
                aliases: None,
                description: "List payments for an agent".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "agent_id": {
                            "type": "string",
                            "description": "The agent ID to list payments for"
                        },
                        "status": {
                            "type": "string",
                            "enum": ["pending", "completed", "failed", "all"],
                            "description": "Filter by status",
                            "default": "all"
                        }
                    },
                    "required": ["agent_id"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for ListPaymentsTool {
    async fn execute(&self, params: Value) -> arkavo_mcp_tools::Result<Value> {
        let agent_id = params["agent_id"].as_str().unwrap_or("default");
        let status_filter = params["status"].as_str().unwrap_or("all");

        let state = self.state.read().await;
        let payments = state.tracker.get_by_agent(agent_id).await;

        let filtered: Vec<_> = payments
            .iter()
            .filter(|p| match status_filter {
                "pending" => {
                    matches!(
                        p.status(),
                        PaymentStatus::Pending | PaymentStatus::Processing
                    )
                }
                "completed" => matches!(p.status(), PaymentStatus::Completed),
                "failed" => matches!(p.status(), PaymentStatus::Failed),
                _ => true,
            })
            .map(|p| {
                json!({
                    "payment_id": p.intent.id.to_string(),
                    "status": format!("{:?}", p.status()).to_lowercase(),
                    "amount": format!("{}", p.intent.amount),
                    "merchant": p.intent.merchant.name,
                    "created_at": p.intent.created_at.to_rfc3339(),
                })
            })
            .collect();

        let stats = state.tracker.get_stats(agent_id).await;

        Ok(success_response(json!({
            "payments": filtered,
            "total": filtered.len(),
            "stats": {
                "pending": stats.pending,
                "completed": stats.completed,
                "failed": stats.failed,
                "total_spent_cents": stats.total_spent,
            }
        })))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
