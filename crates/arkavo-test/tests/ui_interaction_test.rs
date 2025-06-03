#[cfg(test)]
mod ui_interaction_tests {
    use arkavo_test::mcp::{
        device_manager::DeviceManager,
        xctest_enhanced::XCTestEnhanced,
        xctest_unix_bridge::{CommandType, XCTestUnixBridge},
    };
    use std::sync::Arc;

    #[tokio::test]
    async fn test_xctest_tap_functionality() {
        // Create device manager
        let device_manager = Arc::new(DeviceManager::new());

        // Get or boot a simulator
        device_manager.refresh_devices().unwrap();
        let devices = device_manager.get_booted_devices();

        if devices.is_empty() {
            eprintln!("No booted devices found, skipping test");
            return;
        }

        let device = &devices[0];
        let device_id = &device.id;

        // Create enhanced XCTest instance
        let xctest = match XCTestEnhanced::new(device_manager.clone()).await {
            Ok(test) => test,
            Err(e) => {
                eprintln!("Failed to create XCTest instance: {}", e);
                return;
            }
        };

        // Initialize XCTest on the device
        if let Err(e) = xctest.initialize(device_id).await {
            eprintln!("Failed to initialize XCTest: {}", e);
            return;
        }

        // Test coordinate tap
        match xctest.tap(200.0, 400.0).await {
            Ok(response) => {
                assert!(response.success, "Tap should succeed");
                println!("Tap successful: {:?}", response.result);
            }
            Err(e) => {
                eprintln!("Tap failed: {}", e);
            }
        }

        // Test tap by text
        match xctest.tap_by_text("Login", Some(5.0)).await {
            Ok(response) => {
                println!("Text tap result: {:?}", response);
            }
            Err(e) => {
                eprintln!("Text tap failed: {}", e);
            }
        }
    }

    #[tokio::test]
    async fn test_swipe_command() {
        // Create XCTest bridge directly
        let _bridge = XCTestUnixBridge::new();

        // Create swipe command
        let swipe_cmd = XCTestUnixBridge::create_swipe(
            200.0,
            600.0, // Start coordinates
            200.0,
            200.0,     // End coordinates
            Some(0.5), // Duration
        );

        assert_eq!(swipe_cmd.command_type, CommandType::Swipe);
        assert_eq!(swipe_cmd.parameters.x1, Some(200.0));
        assert_eq!(swipe_cmd.parameters.y1, Some(600.0));
        assert_eq!(swipe_cmd.parameters.x2, Some(200.0));
        assert_eq!(swipe_cmd.parameters.y2, Some(200.0));
        assert_eq!(swipe_cmd.parameters.duration, Some(0.5));
    }

    #[tokio::test]
    async fn test_type_text_command() {
        // Create type text command
        let type_cmd = XCTestUnixBridge::create_type_text(
            "Hello, World!".to_string(),
            true, // Clear first
        );

        assert_eq!(type_cmd.command_type, CommandType::TypeText);
        assert_eq!(
            type_cmd.parameters.text_to_type,
            Some("Hello, World!".to_string())
        );
        assert_eq!(type_cmd.parameters.clear_first, Some(true));
    }

    #[test]
    fn test_command_serialization() {
        // Test that commands serialize correctly for Swift
        let tap_cmd = XCTestUnixBridge::create_coordinate_tap(100.0, 200.0);
        let json = serde_json::to_string(&tap_cmd).unwrap();

        assert!(json.contains("\"type\":\"tap\""));
        assert!(json.contains("\"targetType\":\"coordinate\""));
        assert!(json.contains("\"x\":100.0"));
        assert!(json.contains("\"y\":200.0"));

        // Test swipe serialization
        let swipe_cmd = XCTestUnixBridge::create_swipe(100.0, 200.0, 300.0, 400.0, Some(1.0));
        let json = serde_json::to_string(&swipe_cmd).unwrap();

        assert!(json.contains("\"type\":\"swipe\""));
        assert!(json.contains("\"x1\":100.0"));
        assert!(json.contains("\"y1\":200.0"));
        assert!(json.contains("\"x2\":300.0"));
        assert!(json.contains("\"y2\":400.0"));
        assert!(json.contains("\"duration\":1.0"));
    }
}
