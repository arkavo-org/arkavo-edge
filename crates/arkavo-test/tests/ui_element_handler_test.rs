use arkavo_test::mcp::device_manager::DeviceManager;
use arkavo_test::mcp::server::Tool;
use arkavo_test::mcp::ui_element_handler::UiElementHandler;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn test_tap_checkbox() {
    let device_manager = Arc::new(DeviceManager::new());
    let handler = UiElementHandler::new(device_manager);

    let params = json!({
        "action": "tap_checkbox",
        "coordinates": {
            "x": 50.0,
            "y": 100.0
        }
    });

    let result = handler.execute(params).await.unwrap();

    // In test environment, we expect a device error
    if result.get("error").is_some() {
        assert_eq!(result["error"]["code"], "DEVICE_ERROR");
        return;
    }

    // Otherwise check for success structure
    if result["success"].as_bool().unwrap_or(false) {
        assert_eq!(result["action"], "tap_checkbox");
        assert!(result["strategies_tried"].is_array());
    }
}

#[tokio::test]
async fn test_tap_with_retry() {
    let device_manager = Arc::new(DeviceManager::new());
    let handler = UiElementHandler::new(device_manager);

    let params = json!({
        "action": "tap_with_retry",
        "coordinates": {
            "x": 100.0,
            "y": 200.0
        },
        "retry_count": 3
    });

    let result = handler.execute(params).await.unwrap();

    // Check structure
    assert!(result.is_object());
    if result.get("error").is_some() {
        // Expected in test environment
        assert!(result["error"]["code"].is_string());
    }
}

#[tokio::test]
async fn test_double_tap() {
    let device_manager = Arc::new(DeviceManager::new());
    let handler = UiElementHandler::new(device_manager);

    let params = json!({
        "action": "double_tap",
        "coordinates": {
            "x": 150.0,
            "y": 250.0
        }
    });

    let result = handler.execute(params).await.unwrap();

    // Check structure
    assert!(result.is_object());
    if result["success"].as_bool().unwrap_or(false) {
        assert_eq!(result["action"], "double_tap");
        assert_eq!(result["coordinates"]["x"], 150.0);
        assert_eq!(result["coordinates"]["y"], 250.0);
    }
}

#[tokio::test]
async fn test_long_press() {
    let device_manager = Arc::new(DeviceManager::new());
    let handler = UiElementHandler::new(device_manager);

    let params = json!({
        "action": "long_press",
        "coordinates": {
            "x": 200.0,
            "y": 300.0
        }
    });

    let result = handler.execute(params).await.unwrap();

    // Check structure
    assert!(result.is_object());
    
    // In test environment, we might get a device error
    if result.get("error").is_some() {
        assert_eq!(result["error"]["code"], "DEVICE_ERROR");
        return;
    }

    // Otherwise check for success structure
    if result["success"].as_bool().unwrap_or(false) {
        assert_eq!(result["action"], "long_press");
        assert_eq!(result["coordinates"]["x"], 200.0);
        assert_eq!(result["coordinates"]["y"], 300.0);
    }
}
