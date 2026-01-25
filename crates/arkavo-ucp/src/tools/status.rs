//! Get payment status tool

use super::{UcpState, error_response, success_response};
use arkavo_mcp::ToolSchema;
use arkavo_mcp_tools::server::Tool;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Tool for getting payment status
pub struct GetPaymentStatusTool {
    state: Arc<RwLock<UcpState>>,
    schema: ToolSchema,
}

impl GetPaymentStatusTool {
    pub fn new(state: Arc<RwLock<UcpState>>) -> Self {
        Self {
            state,
            schema: ToolSchema {
                name: "ucp_get_payment_status".to_string(),
                aliases: None,
                description: "Get the status of a payment".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "payment_id": {
                            "type": "string",
                            "description": "The payment ID to check"
                        }
                    },
                    "required": ["payment_id"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for GetPaymentStatusTool {
    async fn execute(&self, params: Value) -> arkavo_mcp_tools::Result<Value> {
        let payment_id_str = params["payment_id"].as_str().unwrap_or("");
        let payment_id = Uuid::parse_str(payment_id_str).map_err(|_| {
            arkavo_mcp_tools::ToolError::InvalidParams(format!(
                "Invalid payment ID: {payment_id_str}"
            ))
        })?;

        let state = self.state.read().await;
        let record = state.tracker.get(payment_id).await;

        match record {
            Some(rec) => Ok(success_response(json!({
                "payment_id": payment_id.to_string(),
                "status": format!("{:?}", rec.status()).to_lowercase(),
                "amount": format!("{}", rec.intent.amount),
                "merchant": rec.intent.merchant.name,
                "created_at": rec.intent.created_at.to_rfc3339(),
                "history_count": rec.history.len(),
            }))),
            None => Ok(error_response(&format!("Payment not found: {payment_id}"))),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
