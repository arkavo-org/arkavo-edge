#[cfg(test)]
mod ios_bridge_tests {
    use arkavo_test::mcp::device_manager::DeviceManager;
    use arkavo_test::mcp::ios_tools::{ScreenCaptureKit, UiInteractionKit, UiQueryKit};
    use arkavo_test::mcp::server::Tool;
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn test_ui_interaction_tap() {
        let device_manager = Arc::new(DeviceManager::new());
        let ui_kit = UiInteractionKit::new(device_manager);

        // Test tap with coordinates
        let params = json!({
            "action": "tap",
            "target": {
                "x": 100,
                "y": 200
            }
        });

        let result = ui_kit.execute(params).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        // Check for either success or simulated response
        assert!(response.is_object());
    }

    #[tokio::test]
    async fn test_ui_interaction_type_text() {
        let device_manager = Arc::new(DeviceManager::new());
        let ui_kit = UiInteractionKit::new(device_manager);

        let params = json!({
            "action": "type_text",
            "value": "Hello, iOS!"
        });

        let result = ui_kit.execute(params).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        // Check for either success or simulated response
        assert!(response.is_object());
    }

    #[tokio::test]
    async fn test_ui_interaction_swipe() {
        let device_manager = Arc::new(DeviceManager::new());
        let ui_kit = UiInteractionKit::new(device_manager);

        let params = json!({
            "action": "swipe",
            "swipe": {
                "x1": 100,
                "y1": 300,
                "x2": 100,
                "y2": 100,
                "duration": 0.5
            }
        });

        let result = ui_kit.execute(params).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        // Check for either success or simulated response
        assert!(response.is_object());
    }

    #[tokio::test]
    async fn test_screen_capture() {
        let device_manager = Arc::new(DeviceManager::new());
        let capture_kit = ScreenCaptureKit::new(device_manager);

        let params = json!({
            "name": "test_screenshot"
        });

        let result = capture_kit.execute(params).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        // Check for either success or simulated response
        assert!(response.is_object());
        if let Some(path) = response["path"].as_str() {
            assert!(path.contains("test_screenshot"));
        }
    }

    #[tokio::test]
    async fn test_ui_query() {
        let device_manager = Arc::new(DeviceManager::new());
        let query_kit = UiQueryKit::new(device_manager);

        let params = json!({
            "query_type": "accessibility_tree"
        });

        let result = query_kit.execute(params).await;
        assert!(result.is_ok());

        let response = result.unwrap();
        // Check for either success or simulated response
        assert!(response.is_object());
    }

    #[tokio::test]
    async fn test_device_management() {
        let device_manager = Arc::new(DeviceManager::new());

        // Test listing devices
        let devices = device_manager.get_all_devices();
        println!("Found {} devices", devices.len());

        // Test getting booted devices
        let booted = device_manager.get_booted_devices();
        println!("Found {} booted devices", booted.len());

        // Test active device management
        if let Some(first_booted) = booted.first() {
            let result = device_manager.set_active_device(&first_booted.id);
            assert!(result.is_ok());

            let active = device_manager.get_active_device();
            assert!(active.is_some());
            assert_eq!(active.unwrap().id, first_booted.id);
        }
    }
}
