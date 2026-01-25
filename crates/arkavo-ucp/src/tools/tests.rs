use super::*;
use arkavo_budget::BudgetConfig;
use arkavo_mcp_tools::server::Tool;

async fn create_test_state() -> Arc<RwLock<UcpState>> {
    let config = BudgetConfig::default();
    let tracker = BudgetTracker::new(config).await.unwrap();
    let state = UcpState::new(Arc::new(tracker));
    Arc::new(RwLock::new(state))
}

#[tokio::test]
async fn test_create_payment_tool() {
    let state = create_test_state().await;
    let tool = CreatePaymentIntentTool::new(state);

    let params = json!({
        "amount": 100,
        "currency": "USD",
        "merchant_id": "test-merchant",
        "agent_id": "agent-1"
    });

    let result = tool.execute(params).await.unwrap();
    assert!(result["success"].as_bool().unwrap());
    assert!(result["data"]["payment_id"].is_string());
}

#[tokio::test]
async fn test_execute_payment_tool() {
    let state = create_test_state().await;

    let create_tool = CreatePaymentIntentTool::new(state.clone());
    let create_result = create_tool
        .execute(json!({
            "amount": 100,
            "currency": "USD",
            "merchant_id": "test",
            "agent_id": "agent-1"
        }))
        .await
        .unwrap();

    let payment_id = create_result["data"]["payment_id"].as_str().unwrap();

    let execute_tool = ExecutePaymentTool::new(state);
    let result = execute_tool
        .execute(json!({
            "payment_id": payment_id
        }))
        .await
        .unwrap();

    assert!(result["success"].as_bool().unwrap());
    assert_eq!(result["data"]["status"].as_str().unwrap(), "completed");
}

#[tokio::test]
async fn test_get_payment_status_tool() {
    let state = create_test_state().await;

    let create_tool = CreatePaymentIntentTool::new(state.clone());
    let create_result = create_tool
        .execute(json!({
            "amount": 100,
            "currency": "USD",
            "merchant_id": "test",
            "agent_id": "agent-1"
        }))
        .await
        .unwrap();

    let payment_id = create_result["data"]["payment_id"].as_str().unwrap();

    let status_tool = GetPaymentStatusTool::new(state);
    let result = status_tool
        .execute(json!({
            "payment_id": payment_id
        }))
        .await
        .unwrap();

    assert!(result["success"].as_bool().unwrap());
    assert_eq!(result["data"]["status"].as_str().unwrap(), "pending");
}

#[tokio::test]
async fn test_list_payments_tool() {
    let state = create_test_state().await;

    let create_tool = CreatePaymentIntentTool::new(state.clone());
    create_tool
        .execute(json!({
            "amount": 100,
            "currency": "USD",
            "merchant_id": "test1",
            "agent_id": "agent-1"
        }))
        .await
        .unwrap();

    create_tool
        .execute(json!({
            "amount": 150,
            "currency": "USD",
            "merchant_id": "test2",
            "agent_id": "agent-1"
        }))
        .await
        .unwrap();

    let list_tool = ListPaymentsTool::new(state);
    let result = list_tool
        .execute(json!({
            "agent_id": "agent-1"
        }))
        .await
        .unwrap();

    assert!(result["success"].as_bool().unwrap());
    assert_eq!(result["data"]["total"].as_u64().unwrap(), 2);
}

#[tokio::test]
async fn test_policy_violation() {
    let state = create_test_state().await;
    let tool = CreatePaymentIntentTool::new(state);

    let params = json!({
        "amount": 100000,
        "currency": "USD",
        "merchant_id": "test-merchant",
        "agent_id": "agent-1"
    });

    let result = tool.execute(params).await.unwrap();
    assert!(!result["success"].as_bool().unwrap());
    assert!(
        result["error"].as_str().unwrap().contains("exceeds")
            || result["error"].as_str().unwrap().contains("budget")
    );
}

#[tokio::test]
async fn test_invalid_payment_id() {
    let state = create_test_state().await;
    let tool = GetPaymentStatusTool::new(state);

    let result = tool
        .execute(json!({
            "payment_id": "invalid-uuid"
        }))
        .await;

    assert!(result.is_err());
}

#[tokio::test]
async fn test_tool_schemas() {
    let state = create_test_state().await;

    let create_tool = CreatePaymentIntentTool::new(state.clone());
    assert_eq!(create_tool.schema().name, "ucp_create_payment");

    let execute_tool = ExecutePaymentTool::new(state.clone());
    assert_eq!(execute_tool.schema().name, "ucp_execute_payment");

    let status_tool = GetPaymentStatusTool::new(state.clone());
    assert_eq!(status_tool.schema().name, "ucp_get_payment_status");

    let list_tool = ListPaymentsTool::new(state);
    assert_eq!(list_tool.schema().name, "ucp_list_payments");
}
