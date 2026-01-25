//! Execute payment tool

use super::{error_response, success_response, UcpState};
use crate::types::PaymentStatus;
use arkavo_mcp::ToolSchema;
use arkavo_mcp_tools::server::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Tool for executing a payment
pub struct ExecutePaymentTool {
    state: Arc<RwLock<UcpState>>,
    schema: ToolSchema,
}

impl ExecutePaymentTool {
    pub fn new(state: Arc<RwLock<UcpState>>) -> Self {
        Self {
            state,
            schema: ToolSchema {
                name: "ucp_execute_payment".to_string(),
                aliases: None,
                description: "Execute a previously created payment intent".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "payment_id": {
                            "type": "string",
                            "description": "The payment intent ID to execute"
                        },
                        "confirmed": {
                            "type": "boolean",
                            "description": "Confirmation for payments above threshold",
                            "default": false
                        }
                    },
                    "required": ["payment_id"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for ExecutePaymentTool {
    async fn execute(&self, params: Value) -> arkavo_mcp_tools::Result<Value> {
        let payment_id_str = params["payment_id"].as_str().unwrap_or("");
        let payment_id = Uuid::parse_str(payment_id_str)
            .map_err(|_| arkavo_mcp_tools::ToolError::InvalidParams(format!("Invalid payment ID: {payment_id_str}")))?;

        let state = self.state.read().await;

        let record = state
            .tracker
            .get(payment_id)
            .await
            .ok_or_else(|| arkavo_mcp_tools::ToolError::Execution(format!("Payment not found: {payment_id}")))?;

        if record.status().is_terminal() {
            return Ok(error_response(&format!(
                "Payment already in terminal state: {:?}",
                record.status()
            )));
        }

        let intent = &record.intent;
        let currency = intent.amount.currency;

        let handler = state
            .get_handler(currency)
            .ok_or_else(|| arkavo_mcp_tools::ToolError::Execution(format!("No handler for currency: {currency}")))?;

        state
            .tracker
            .update_status(payment_id, PaymentStatus::Processing, None)
            .await
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(e.to_string()))?;

        let result = handler
            .execute(intent)
            .await
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(e.to_string()))?;

        state
            .tracker
            .complete(payment_id, result.clone())
            .await
            .map_err(|e| arkavo_mcp_tools::ToolError::Execution(e.to_string()))?;

        Ok(success_response(json!({
            "payment_id": payment_id.to_string(),
            "status": format!("{:?}", result.status).to_lowercase(),
            "tx_hash": result.tx_hash,
            "signed_tx": result.signed_tx.map(|b| hex::encode(&b)),
        })))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
