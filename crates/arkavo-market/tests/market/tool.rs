//! The `market_request` tool: what the model can ask for, and what it reads
//! back.

use std::sync::Arc;

use arkavo_market::{AgentKey, MARKET_REQUEST_TOOL, MarketRequestTool, register_tools};
use arkavo_mcp_tools::server::Tool;
use arkavo_mcp_tools::{ToolError, ToolRegistry};
use serde_json::json;
use wiremock::MockServer;

use super::{OFFER, World, open_request, refusal, world, world_with};

fn tool(world: World) -> (MarketRequestTool, MockServer, MockServer) {
    let World {
        identity,
        market,
        keypair,
        client,
    } = world;
    let bytes = keypair.to_bytes();
    let key: AgentKey = Arc::new(move || {
        arkavo_crypto::AgentKeypair::from_bytes(&bytes).map_err(|e| e.to_string())
    });
    (
        MarketRequestTool::new(Arc::new(client), key),
        identity,
        market,
    )
}

#[test]
fn the_ask_is_the_only_tool_registered() {
    let mut registry = ToolRegistry::empty();

    register_tools(&mut registry);

    let names: Vec<String> = registry.list_tools().into_iter().map(|t| t.name).collect();
    assert_eq!(names, [MARKET_REQUEST_TOOL]);
    let schema = registry.get(MARKET_REQUEST_TOOL).unwrap().schema();
    assert_eq!(schema.parameters["required"], json!(["offer", "term"]));
    assert_eq!(
        schema.parameters["properties"]["term"]["enum"],
        json!([3, 12])
    );
    // Where the request goes is never the model's to choose.
    let properties = schema.parameters["properties"].as_object().unwrap();
    assert_eq!(properties.len(), 2);
}

#[tokio::test]
async fn the_model_gets_the_payment_link_to_hand_to_the_person() {
    let (tool, _identity, market) = tool(world(vec![open_request(3)]).await);

    let answer = tool
        .execute(json!({"offer": OFFER, "term": 3}))
        .await
        .unwrap();

    assert_eq!(answer["success"], true);
    assert_eq!(answer["data"]["url"], "https://buy.stripe.com/test_abc");
    assert_eq!(answer["data"]["request"], "rq_AAAAAAAAAAAAAAAAAAAAAA");
    assert_eq!(answer["data"]["term"], 3);
    assert_eq!(answer["data"]["status"], "open");
    assert_eq!(market.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_argument_naming_another_address_sends_nothing_there() {
    let elsewhere = MockServer::start().await;
    let (tool, _identity, market) = tool(world(vec![open_request(12)]).await);

    let answer = tool
        .execute(json!({
            "offer": OFFER,
            "term": 12,
            "url": elsewhere.uri(),
            "market_url": elsewhere.uri(),
        }))
        .await
        .unwrap();

    assert_eq!(answer["success"], true);
    assert!(elsewhere.received_requests().await.unwrap().is_empty());
    assert_eq!(market.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn arguments_the_market_would_refuse_are_refused_here() {
    let (tool, identity, market) = tool(world(vec![open_request(3)]).await);

    for arguments in [
        json!({"term": 3}),
        json!({"offer": OFFER}),
        json!({"offer": OFFER, "term": 6}),
        json!({"offer": OFFER, "term": "3"}),
        json!({"offer": 7, "term": 3}),
    ] {
        let err = tool.execute(arguments.clone()).await.unwrap_err();
        assert!(matches!(err, ToolError::InvalidParams(_)), "{arguments}");
    }
    assert!(identity.received_requests().await.unwrap().is_empty());
    assert!(market.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_refusal_reaches_the_model_as_words_it_can_pass_on() {
    let w = world_with(&[], vec![open_request(3)]).await;
    let (undelegated, _identity, market) = tool(w);

    let answer = undelegated
        .execute(json!({"offer": OFFER, "term": 3}))
        .await
        .unwrap();

    assert_eq!(answer["success"], false);
    assert!(
        answer["error"]
            .as_str()
            .unwrap()
            .contains("has not let it ask the market"),
        "{answer}"
    );
    assert!(market.received_requests().await.unwrap().is_empty());

    let (unavailable, _identity, _market) = tool(world(vec![refusal(503, "unavailable")]).await);
    let answer = unavailable
        .execute(json!({"offer": OFFER, "term": 3}))
        .await
        .unwrap();
    assert_eq!(answer["success"], false);
    assert!(answer["error"].as_str().unwrap().contains("unavailable"));
}

#[tokio::test]
async fn an_agent_key_that_cannot_be_read_is_reported_and_nothing_is_sent() {
    let w = world(vec![open_request(3)]).await;
    let key: AgentKey = Arc::new(|| Err("keychain is locked".to_string()));
    let tool = MarketRequestTool::new(Arc::new(w.client), key);

    let answer = tool
        .execute(json!({"offer": OFFER, "term": 3}))
        .await
        .unwrap();

    assert_eq!(answer["success"], false);
    assert!(
        answer["error"]
            .as_str()
            .unwrap()
            .contains("keychain is locked")
    );
    assert!(w.identity.received_requests().await.unwrap().is_empty());
    assert!(w.market.received_requests().await.unwrap().is_empty());
}
