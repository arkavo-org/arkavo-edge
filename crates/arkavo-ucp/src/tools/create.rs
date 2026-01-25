//! Create payment intent tool

use super::{error_response, success_response, UcpState};
use crate::types::{Currency, Merchant, PaymentAmount, PaymentIntent};
use arkavo_mcp::ToolSchema;
use arkavo_mcp_tools::server::Tool;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Tool for creating payment intents
pub struct CreatePaymentIntentTool {
    state: Arc<RwLock<UcpState>>,
    schema: ToolSchema,
}

impl CreatePaymentIntentTool {
    pub fn new(state: Arc<RwLock<UcpState>>) -> Self {
        Self {
            state,
            schema: ToolSchema {
                name: "ucp_create_payment".to_string(),
                aliases: None,
                description: "Create a payment intent for UCP commerce".to_string(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "amount": {
                            "type": "number",
                            "description": "Amount in cents (for USD) or wei (for ETH)"
                        },
                        "currency": {
                            "type": "string",
                            "enum": ["USD", "ETH", "USDC"],
                            "description": "Payment currency"
                        },
                        "merchant_id": {
                            "type": "string",
                            "description": "Merchant identifier"
                        },
                        "merchant_name": {
                            "type": "string",
                            "description": "Merchant display name"
                        },
                        "merchant_address": {
                            "type": "string",
                            "description": "Merchant crypto address (for ETH payments)"
                        },
                        "description": {
                            "type": "string",
                            "description": "Payment description"
                        },
                        "agent_id": {
                            "type": "string",
                            "description": "Agent making the payment"
                        }
                    },
                    "required": ["amount", "currency", "merchant_id", "agent_id"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for CreatePaymentIntentTool {
    async fn execute(&self, params: Value) -> arkavo_mcp_tools::Result<Value> {
        let amount_value = params["amount"].as_u64().unwrap_or(0);
        let currency_str = params["currency"].as_str().unwrap_or("USD");
        let merchant_id = params["merchant_id"].as_str().unwrap_or("");
        let merchant_name = params["merchant_name"].as_str().unwrap_or(merchant_id);
        let merchant_address = params["merchant_address"].as_str();
        let description = params["description"].as_str();
        let agent_id = params["agent_id"].as_str().unwrap_or("default");

        let currency = match currency_str.to_uppercase().as_str() {
            "USD" => Currency::Usd,
            "ETH" => Currency::Eth,
            "USDC" => Currency::Usdc,
            other => return Ok(error_response(&format!("Unknown currency: {other}"))),
        };

        let amount = PaymentAmount {
            value: amount_value,
            currency,
        };

        let mut merchant = Merchant::new(merchant_id, merchant_name);
        if let Some(addr) = merchant_address {
            merchant = merchant.with_crypto_address(addr);
        }

        let mut intent = PaymentIntent::new(agent_id, amount, merchant);
        if let Some(desc) = description {
            intent = intent.with_description(desc);
        }

        let state = self.state.read().await;
        let decision = state.policy.evaluate(&intent).await;

        if !decision.allowed {
            drop(state);
            return Ok(error_response(&format!(
                "Policy violation: {}",
                decision.reason.unwrap_or_default()
            )));
        }

        let id = state.tracker.create(intent.clone()).await;
        drop(state);

        Ok(success_response(json!({
            "payment_id": id.to_string(),
            "status": "pending",
            "requires_confirmation": decision.requires_confirmation,
            "confirmation_reason": decision.reason,
            "amount": format!("{}", intent.amount),
        })))
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
