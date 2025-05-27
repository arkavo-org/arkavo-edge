#[cfg(test)]
mod ios_bridge_tests {
    use arkavo_test::bridge::ios_ffi::RustTestHarness;
    use arkavo_test::mcp::device_manager::DeviceManager;
    use arkavo_test::mcp::ios_tools::{ScreenCaptureKit, UiInteractionKit, UiQueryKit};
    use arkavo_test::mcp::server::Tool;
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn test_ui_interaction_tap() {
        let harness = Arc::new(Mutex::new(RustTestHarness::new()));
        let device_manager = Arc::new(DeviceManager::new());
        let ui_kit = UiInteractionKit::new(harness.clone(), device_manager);

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
        let harness = Arc::new(Mutex::new(RustTestHarness::new()));
        let device_manager = Arc::new(DeviceManager::new());
        let ui_kit = UiInteractionKit::new(harness.clone(), device_manager);

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
        let harness = Arc::new(Mutex::new(RustTestHarness::new()));
        let device_manager = Arc::new(DeviceManager::new());
        let ui_kit = UiInteractionKit::new(harness.clone(), device_manager);

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
        let harness = Arc::new(Mutex::new(RustTestHarness::new()));
        let device_manager = Arc::new(DeviceManager::new());
        let capture_kit = ScreenCaptureKit::new(harness.clone(), device_manager);

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
        let harness = Arc::new(Mutex::new(RustTestHarness::new()));
        let device_manager = Arc::new(DeviceManager::new());
        let query_kit = UiQueryKit::new(harness.clone(), device_manager);

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
    async fn test_harness_state_management() {
        let harness = Arc::new(Mutex::new(RustTestHarness::new()));
        
        // Test checkpoint creation
        {
            let mut h = harness.lock().unwrap();
            let result = h.checkpoint("test_checkpoint");
            assert!(result.is_ok());
        }
        
        // Test branching
        {
            let mut h = harness.lock().unwrap();
            let result = h.branch("test_checkpoint", "new_branch");
            assert!(result.is_ok());
        }
        
        // Test restore
        {
            let mut h = harness.lock().unwrap();
            let result = h.restore("test_checkpoint");
            assert!(result.is_ok());
        }
    }
}