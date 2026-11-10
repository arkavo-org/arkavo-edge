//! The tool an agent's model asks the market with.
//!
//! Only the ask is a tool. Withdrawing a request, like asking for a
//! different term, kills a link the owner may already have shared, so the
//! model is not handed a way to do just that; `arkavo market withdraw` is
//! for the person. Where the request goes is fixed too: a model that could
//! name the address could send the agent's token anywhere.

use std::sync::Arc;

use arkavo_crypto::AgentKeypair;
use arkavo_mcp::ToolSchema;
use arkavo_mcp_tools::server::Tool;
use arkavo_mcp_tools::{ToolError, ToolRegistry};
use async_trait::async_trait;
use serde_json::{Value, json};

use crate::{MARKET_URL, MarketClient, Term};

/// The tool's name in a registry, and in a SwarmKit role's grants.
pub const MARKET_REQUEST_TOOL: &str = "market_request";

/// Where the tool gets the agent's key each time it is called, so a key
/// replaced while the agent runs is the one the next request is signed with.
pub type AgentKey = Arc<dyn Fn() -> Result<AgentKeypair, String> + Send + Sync>;

/// Asks the market for a prepaid term on the owner's behalf.
pub struct MarketRequestTool {
    /// `None` is the market in production, reached with a client made when
    /// the tool is called. An agent builds its registry for every message it
    /// answers, and nearly none of them ask the market, so a registered tool
    /// holds no connection pool or TLS configuration of its own.
    client: Option<Arc<MarketClient>>,
    key: AgentKey,
    schema: ToolSchema,
}

impl MarketRequestTool {
    /// A tool that asks through `client`.
    pub fn new(client: Arc<MarketClient>, key: AgentKey) -> Self {
        Self::build(Some(client), key)
    }

    fn build(client: Option<Arc<MarketClient>>, key: AgentKey) -> Self {
        Self {
            client,
            key,
            schema: ToolSchema {
                name: MARKET_REQUEST_TOOL.to_string(),
                aliases: None,
                description: format!(
                    "Ask the Arkavo market for a prepaid term of a subscription for the person \
                     this agent acts for. Returns a payment link to give to that person: they, \
                     or anyone they share it with, pay it. This agent cannot pay. Asking again \
                     for the same term returns the same link; asking for a different term \
                     replaces it and the earlier link stops working. Offer IDs are listed at \
                     {MARKET_URL}/api (Arkavo Game Pass is `arkavo-online-play`)."
                ),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "offer": {
                            "type": "string",
                            "description": "The offer ID from the market's listing"
                        },
                        "term": {
                            "type": "integer",
                            "enum": [3, 12],
                            "description": "Months the prepaid term covers"
                        }
                    },
                    "required": ["offer", "term"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for MarketRequestTool {
    async fn execute(&self, params: Value) -> arkavo_mcp_tools::Result<Value> {
        let offer = params["offer"]
            .as_str()
            .ok_or_else(|| ToolError::InvalidParams("`offer` is the offer ID, a string".into()))?;
        let term = params["term"]
            .as_u64()
            .ok_or_else(|| ToolError::InvalidParams("`term` is 3 or 12, a number".into()))
            .and_then(|months| {
                Term::try_from(months).map_err(|e| ToolError::InvalidParams(e.to_string()))
            })?;

        let keypair = match (self.key)() {
            Ok(keypair) => keypair,
            Err(e) => return Ok(failed(&format!("The agent's key could not be read: {e}"))),
        };
        let client = match &self.client {
            Some(client) => Arc::clone(client),
            None => match MarketClient::new() {
                Ok(client) => Arc::new(client),
                Err(e) => return Ok(failed(&e.to_string())),
            },
        };
        match client.ask(&keypair, offer, term).await {
            Ok(request) => Ok(json!({
                "success": true,
                "data": {
                    "request": request.request,
                    "url": request.url,
                    "term": request.term,
                    "status": request.status,
                }
            })),
            // A refusal is something to tell the person, not a tool failure:
            // the words say what they have to do next.
            Err(e) => Ok(failed(&e.to_string())),
        }
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}

fn failed(message: &str) -> Value {
    json!({ "success": false, "error": message })
}

/// Register the market tool, signing as this machine's agent identity.
pub fn register_tools(registry: &mut ToolRegistry) {
    let key: AgentKey =
        Arc::new(|| arkavo_registration::load_or_create_agent_keypair().map_err(|e| e.to_string()));
    registry.register(
        MARKET_REQUEST_TOOL,
        Box::new(MarketRequestTool::build(None, key)),
    );
}
