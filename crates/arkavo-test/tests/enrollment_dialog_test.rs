use arkavo_test::mcp::device_manager::DeviceManager;
use arkavo_test::mcp::enrollment_dialog_handler::EnrollmentDialogHandler;
use arkavo_test::mcp::server::Tool;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn test_enrollment_dialog_cancel_coordinates() {
    // Create device manager
    let device_manager = Arc::new(DeviceManager::new());

    // Create enrollment dialog handler
    let handler = EnrollmentDialogHandler::new(device_manager);

    // Test getting cancel coordinates for iPhone 16 Pro Max
    let params = json!({
        "action": "get_cancel_coordinates"
    });

    let result = handler.execute(params).await.unwrap();

    // Check if we got an error (no device available in test environment)
    if result.get("error").is_some() {
        // This is expected in test environment without real devices
        assert!(result["error"]["code"].is_string());
        return;
    }

    // Verify the response structure
    assert!(result["success"].as_bool().unwrap_or(false));
    assert_eq!(result["action"], "get_cancel_coordinates");
    assert!(result["cancel_button"].is_object());
    assert!(result["cancel_button"]["x"].is_number());
    assert!(result["cancel_button"]["y"].is_number());

    // For iPhone 16 Pro Max, coordinates should be around (215, 830)
    if result["device_type"]
        .as_str()
        .unwrap_or("")
        .contains("iPhone-16-Pro-Max")
    {
        let x = result["cancel_button"]["x"].as_f64().unwrap();
        let y = result["cancel_button"]["y"].as_f64().unwrap();
        assert_eq!(x, 215.0);
        assert_eq!(y, 830.0);
    }
}

#[tokio::test]
async fn test_enrollment_dialog_tap_cancel() {
    // Create device manager
    let device_manager = Arc::new(DeviceManager::new());

    // Create enrollment dialog handler
    let handler = EnrollmentDialogHandler::new(device_manager);

    // Test tap_cancel action
    let params = json!({
        "action": "tap_cancel"
    });

    let result = handler.execute(params).await.unwrap();

    // Check if we got an error (no device available in test environment)
    if result.get("error").is_some() {
        // This is expected in test environment without real devices
        assert!(result["error"]["code"].is_string());
        return;
    }

    // Verify the response provides next step
    assert!(result["success"].as_bool().unwrap_or(false));
    assert_eq!(result["action"], "tap_cancel");
    assert!(result["coordinates"].is_object());
    assert!(result["next_step"].is_object());
    assert_eq!(result["next_step"]["tool"], "ui_interaction");
}
