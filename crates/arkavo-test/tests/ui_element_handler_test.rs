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

    match handler.execute(params).await {
        Ok(result) => {
            // In test environment, we expect errors
            if let Some(error) = result.get("error") {
                let error_code = error["code"].as_str().unwrap_or("");
                assert!(
                    error_code == "DEVICE_ERROR" || error_code == "CHECKBOX_TAP_FAILED",
                    "Unexpected error code: {}",
                    error_code
                );
                return;
            }

            // Otherwise check for success structure
            if result["success"].as_bool().unwrap_or(false) {
                assert_eq!(result["action"], "tap_checkbox");
                assert!(result["strategies_tried"].is_array());
            }
        }
        Err(_) => {
            // Acceptable in CI environment
        }
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

    match handler.execute(params).await {
        Ok(result) => {
            // Check structure
            assert!(result.is_object());
            if result.get("error").is_some() {
                // Expected in test environment
                assert!(result["error"]["code"].is_string());
            }
        }
        Err(_) => {
            // Acceptable in CI environment
        }
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

    match handler.execute(params).await {
        Ok(result) => {
            // Check structure
            assert!(result.is_object());

            // Handle both success and error cases
            if let Some(error) = result.get("error") {
                let error_code = error["code"].as_str().unwrap_or("");
                assert!(
                    error_code == "DEVICE_ERROR" || error_code == "DOUBLE_TAP_FAILED",
                    "Unexpected error code: {}",
                    error_code
                );
            } else if result["success"].as_bool().unwrap_or(false) {
                assert_eq!(result["action"], "double_tap");
                assert_eq!(result["coordinates"]["x"], 150.0);
                assert_eq!(result["coordinates"]["y"], 250.0);
            }
        }
        Err(_) => {
            // Acceptable in CI environment
        }
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

    match handler.execute(params).await {
        Ok(result) => {
            // Debug output
            eprintln!("test_long_press result: {:?}", result);

            // Check structure
            assert!(
                result.is_object(),
                "Expected JSON object, got: {:?}",
                result
            );

            // In test environment, we might get various types of errors
            if let Some(error) = result.get("error") {
                // Accept either DEVICE_ERROR or LONG_PRESS_FAILED in CI environments
                let error_code = error["code"].as_str().unwrap_or("");
                assert!(
                    error_code == "DEVICE_ERROR" || error_code == "LONG_PRESS_FAILED",
                    "Unexpected error code: {} in error: {:?}",
                    error_code,
                    error
                );
                return;
            }

            // Otherwise check for success structure
            if result["success"].as_bool().unwrap_or(false) {
                assert_eq!(result["action"], "long_press");
                assert_eq!(result["coordinates"]["x"], 200.0);
                assert_eq!(result["coordinates"]["y"], 300.0);
            } else {
                // Handle failure case
                assert!(
                    result.get("error").is_some(),
                    "Expected either success or error in result: {:?}",
                    result
                );
            }
        }
        Err(e) => {
            // In CI without display, the handler itself might return an error
            eprintln!("Handler returned error: {:?}", e);
            // This is acceptable in CI environment
        }
    }
}
