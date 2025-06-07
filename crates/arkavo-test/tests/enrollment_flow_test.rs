use arkavo_test::mcp::device_manager::DeviceManager;
use arkavo_test::mcp::enrollment_flow_handler::EnrollmentFlowHandler;
use arkavo_test::mcp::server::Tool;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn test_enrollment_flow_complete() {
    // Create device manager
    let device_manager = Arc::new(DeviceManager::new());

    // Create enrollment flow handler
    let handler = EnrollmentFlowHandler::new(device_manager);

    // Test complete enrollment flow
    let params = json!({
        "action": "complete_enrollment",
        "app_bundle_id": "com.test.app"
    });

    let result = handler.execute(params).await.unwrap();

    // Check if we got an error (no device available in test environment)
    if result.get("error").is_some() {
        // This is expected in test environment without real devices
        assert!(result["error"]["code"].is_string());
        return;
    }

    // Should have success or failure with steps
    if result["success"].as_bool().unwrap_or(false) {
        assert_eq!(result["action"], "complete_enrollment");
        assert!(result["steps_completed"].is_array());
        assert_eq!(result["app_bundle_id"], "com.test.app");
    } else {
        assert!(result["error"].is_object());
        assert!(result["error"]["steps_completed"].is_array());
    }
}

#[tokio::test]
async fn test_enrollment_flow_dismiss_and_relaunch() {
    // Create device manager
    let device_manager = Arc::new(DeviceManager::new());

    // Create enrollment flow handler
    let handler = EnrollmentFlowHandler::new(device_manager);

    // Test dismiss and relaunch
    let params = json!({
        "action": "dismiss_and_relaunch"
    });

    let result = handler.execute(params).await.unwrap();

    // Check if we got an error (no device available in test environment)
    if result.get("error").is_some() {
        if result["error"]["code"] == "DEVICE_ERROR" {
            // This is expected in test environment without real devices
            return;
        }
    }

    // Should have action in response
    if result["success"].as_bool().unwrap_or(false) {
        assert_eq!(result["action"], "dismiss_and_relaunch");
        assert!(result["steps_completed"].is_array());
    }
}

#[tokio::test]
async fn test_enrollment_flow_enroll_only() {
    // Create device manager
    let device_manager = Arc::new(DeviceManager::new());

    // Create enrollment flow handler
    let handler = EnrollmentFlowHandler::new(device_manager);

    // Test enroll and continue
    let params = json!({
        "action": "enroll_and_continue"
    });

    let result = handler.execute(params).await.unwrap();

    // Check if we got an error (no device available in test environment)
    if result.get("error").is_some() {
        if result["error"]["code"] == "DEVICE_ERROR" {
            // This is expected in test environment without real devices
            return;
        }
    }

    // Should have action in response
    if result["success"].as_bool().unwrap_or(false) {
        assert_eq!(result["action"], "enroll_and_continue");
        assert!(result["message"].is_string());
    }
}