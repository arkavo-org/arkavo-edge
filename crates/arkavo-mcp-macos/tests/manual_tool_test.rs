//! Manual test for all new MCP tools
//! Run with: cargo test -p arkavo-mcp-macos --test manual_tool_test -- --nocapture

#![allow(
    clippy::disallowed_methods,
    clippy::ignore_without_reason,
    clippy::uninlined_format_args
)]

use arkavo_mcp_macos::mcp::server::Tool;
use serde_json::json;

// Device Workflow Tools
use arkavo_mcp_macos::mcp::device_workflow::{
    DeviceAppPathTool, DeviceBuildTool, DeviceCleanTool, DeviceInstallTool, DeviceLaunchTool,
    DeviceListTool, DeviceLogStartTool, DeviceLogStopTool, DeviceStopTool, DeviceTestTool,
};

// macOS Workflow Tools
use arkavo_mcp_macos::mcp::macos_workflow::{
    MacosAppPathTool, MacosBuildRunTool, MacosBuildSettingsTool, MacosBuildTool, MacosBundleIdTool,
    MacosCleanTool, MacosDiscoverProjectsTool, MacosLaunchTool, MacosListSchemesTool,
    MacosStopTool, MacosTestTool,
};

// Swift Package Workflow Tools
use arkavo_mcp_macos::mcp::swift_package_workflow::{
    SwiftPackageBuildTool, SwiftPackageCleanTool, SwiftPackageInitTool, SwiftPackageListTool,
    SwiftPackageRunTool, SwiftPackageStopTool, SwiftPackageTestTool,
};

// Project Discovery Tools
use arkavo_mcp_macos::mcp::project_discovery::{
    DiscoverProjectsTool, GetAppBundleIdTool, GetProjectInfoTool, ListSchemesTool,
    ShowBuildSettingsTool,
};

// Debugger Tools
use arkavo_mcp_macos::mcp::debugger_tools::{
    DebugAttachTool, DebugBreakpointAddTool, DebugCommandTool, DebugContinueTool, DebugDetachTool,
    DebugStackTool, DebugStepTool, DebugVariablesTool,
};

// Simulator Extended Tools
use arkavo_mcp_macos::mcp::simulator_extended::{
    SimBootTool, SimEraseTool, SimListTool, SimOpenTool, SimResetLocationTool, SimScreenshotTool,
    SimSetAppearanceTool, SimSetLocationTool, SimShutdownTool,
};

// UI Automation Tools
use arkavo_mcp_macos::mcp::ui_automation_tools::{
    UiDescribeTool, UiKeyPressTool, UiLongPressTool, UiScreenshotTool, UiSwipeTool, UiTapTool,
    UiTypeTextTool,
};

async fn test_tool<T: Tool>(tool: T, params: serde_json::Value, name: &str) {
    println!("\n============================================================");
    println!("Testing: {}", name);
    println!("Schema: {}", tool.schema().name);
    println!("Params: {}", params);
    println!("------------------------------------------------------------");

    match tool.execute(params).await {
        Ok(result) => {
            let result_str = serde_json::to_string_pretty(&result).unwrap_or_default();
            if result_str.len() > 500 {
                println!("Result: {}...", &result_str[..500]);
            } else {
                println!("Result: {}", result_str);
            }
        }
        Err(e) => {
            println!("Error: {:?}", e);
        }
    }
}

#[tokio::test]
#[ignore] // Manual test only - requires Xcode and device tools
async fn test_all_tools() {
    println!("\n\n############################################################");
    println!("# DEVICE WORKFLOW TOOLS");
    println!("############################################################");

    test_tool(DeviceListTool::new(), json!({}), "device_list").await;
    test_tool(
        DeviceBuildTool::new(),
        json!({"scheme": "TestScheme"}),
        "device_build",
    )
    .await;
    test_tool(
        DeviceInstallTool::new(),
        json!({"device_id": "test-udid", "app_path": "/tmp/Test.app"}),
        "device_install",
    )
    .await;
    test_tool(
        DeviceLaunchTool::new(),
        json!({"device_id": "test-udid", "bundle_id": "com.test.app"}),
        "device_launch",
    )
    .await;
    test_tool(
        DeviceStopTool::new(),
        json!({"device_id": "test-udid", "bundle_id": "com.test.app"}),
        "device_stop",
    )
    .await;
    test_tool(
        DeviceAppPathTool::new(),
        json!({"device_id": "test-udid", "bundle_id": "com.test.app"}),
        "device_app_path",
    )
    .await;
    test_tool(
        DeviceLogStartTool::new(),
        json!({"device_id": "test-udid"}),
        "device_log_start",
    )
    .await;
    test_tool(
        DeviceLogStopTool::new(),
        json!({"session_id": "fake-session"}),
        "device_log_stop",
    )
    .await;
    test_tool(
        DeviceTestTool::new(),
        json!({"scheme": "TestScheme", "device_id": "test-udid"}),
        "device_test",
    )
    .await;
    test_tool(
        DeviceCleanTool::new(),
        json!({"scheme": "TestScheme"}),
        "device_clean",
    )
    .await;

    println!("\n\n############################################################");
    println!("# MACOS WORKFLOW TOOLS");
    println!("############################################################");

    test_tool(
        MacosBuildTool::new(),
        json!({"scheme": "TestScheme"}),
        "macos_build",
    )
    .await;
    test_tool(
        MacosBuildRunTool::new(),
        json!({"scheme": "TestScheme"}),
        "macos_build_run",
    )
    .await;
    test_tool(
        MacosTestTool::new(),
        json!({"scheme": "TestScheme"}),
        "macos_test",
    )
    .await;
    test_tool(
        MacosLaunchTool::new(),
        json!({"app_path": "/Applications/Calculator.app"}),
        "macos_launch",
    )
    .await;
    test_tool(
        MacosStopTool::new(),
        json!({"app_name": "Calculator"}),
        "macos_stop",
    )
    .await;
    test_tool(
        MacosAppPathTool::new(),
        json!({"scheme": "TestScheme"}),
        "macos_app_path",
    )
    .await;
    test_tool(
        MacosCleanTool::new(),
        json!({"scheme": "TestScheme"}),
        "macos_clean",
    )
    .await;
    test_tool(
        MacosBundleIdTool::new(),
        json!({"app_path": "/Applications/Calculator.app"}),
        "macos_bundle_id",
    )
    .await;
    test_tool(
        MacosBuildSettingsTool::new(),
        json!({"scheme": "TestScheme"}),
        "macos_build_settings",
    )
    .await;
    test_tool(MacosListSchemesTool::new(), json!({}), "macos_list_schemes").await;
    test_tool(
        MacosDiscoverProjectsTool::new(),
        json!({"path": "."}),
        "macos_discover_projects",
    )
    .await;

    println!("\n\n############################################################");
    println!("# SWIFT PACKAGE WORKFLOW TOOLS");
    println!("############################################################");

    test_tool(
        SwiftPackageBuildTool::new(),
        json!({"package_path": "/tmp/nonexistent"}),
        "swift_package_build",
    )
    .await;
    test_tool(
        SwiftPackageRunTool::new(),
        json!({"package_path": "/tmp/nonexistent"}),
        "swift_package_run",
    )
    .await;
    test_tool(
        SwiftPackageTestTool::new(),
        json!({"package_path": "/tmp/nonexistent"}),
        "swift_package_test",
    )
    .await;
    test_tool(
        SwiftPackageCleanTool::new(),
        json!({"package_path": "/tmp/nonexistent"}),
        "swift_package_clean",
    )
    .await;
    test_tool(
        SwiftPackageStopTool::new(),
        json!({"pid": 99999}),
        "swift_package_stop",
    )
    .await;
    test_tool(SwiftPackageListTool::new(), json!({}), "swift_package_list").await;
    test_tool(
        SwiftPackageInitTool::new(),
        json!({"path": "/tmp/test-spm-init"}),
        "swift_package_init",
    )
    .await;

    println!("\n\n############################################################");
    println!("# PROJECT DISCOVERY TOOLS");
    println!("############################################################");

    test_tool(
        DiscoverProjectsTool::new(),
        json!({"path": "."}),
        "discover_projects",
    )
    .await;
    test_tool(
        GetAppBundleIdTool::new(),
        json!({"app_path": "/Applications/Calculator.app"}),
        "get_app_bundle_id",
    )
    .await;
    test_tool(ListSchemesTool::new(), json!({}), "list_schemes").await;
    test_tool(
        ShowBuildSettingsTool::new(),
        json!({"scheme": "TestScheme"}),
        "show_build_settings",
    )
    .await;
    test_tool(GetProjectInfoTool::new(), json!({}), "get_project_info").await;

    println!("\n\n############################################################");
    println!("# DEBUGGER TOOLS");
    println!("############################################################");

    test_tool(DebugAttachTool::new(), json!({"pid": 1}), "debug_attach").await;
    test_tool(
        DebugBreakpointAddTool::new(),
        json!({"session_id": "fake", "location": "main"}),
        "debug_breakpoint",
    )
    .await;
    test_tool(
        DebugContinueTool::new(),
        json!({"session_id": "fake"}),
        "debug_continue",
    )
    .await;
    test_tool(
        DebugStepTool::new(),
        json!({"session_id": "fake"}),
        "debug_step",
    )
    .await;
    test_tool(
        DebugStackTool::new(),
        json!({"session_id": "fake"}),
        "debug_stack",
    )
    .await;
    test_tool(
        DebugVariablesTool::new(),
        json!({"session_id": "fake"}),
        "debug_variables",
    )
    .await;
    test_tool(
        DebugDetachTool::new(),
        json!({"session_id": "fake"}),
        "debug_detach",
    )
    .await;
    test_tool(
        DebugCommandTool::new(),
        json!({"session_id": "fake", "command": "help"}),
        "debug_command",
    )
    .await;

    println!("\n\n############################################################");
    println!("# SIMULATOR EXTENDED TOOLS");
    println!("############################################################");

    test_tool(SimListTool::new(), json!({}), "sim_list").await;
    test_tool(SimBootTool::new(), json!({"udid": "fake-udid"}), "sim_boot").await;
    test_tool(
        SimShutdownTool::new(),
        json!({"udid": "fake-udid"}),
        "sim_shutdown",
    )
    .await;
    test_tool(
        SimScreenshotTool::new(),
        json!({"udid": "fake-udid", "output_path": "/tmp/screenshot.png"}),
        "sim_screenshot",
    )
    .await;
    test_tool(
        SimSetLocationTool::new(),
        json!({"udid": "fake-udid", "latitude": 37.7749, "longitude": -122.4194}),
        "sim_set_location",
    )
    .await;
    test_tool(
        SimResetLocationTool::new(),
        json!({"udid": "fake-udid"}),
        "sim_reset_location",
    )
    .await;
    test_tool(
        SimSetAppearanceTool::new(),
        json!({"udid": "fake-udid", "appearance": "dark"}),
        "sim_set_appearance",
    )
    .await;
    test_tool(
        SimEraseTool::new(),
        json!({"udid": "fake-udid"}),
        "sim_erase",
    )
    .await;
    test_tool(
        SimOpenTool::new(),
        json!({"url": "https://apple.com"}),
        "sim_open",
    )
    .await;

    println!("\n\n############################################################");
    println!("# UI AUTOMATION TOOLS");
    println!("############################################################");

    test_tool(UiTapTool::new(), json!({"x": 100, "y": 100}), "ui_tap").await;
    test_tool(
        UiSwipeTool::new(),
        json!({"start_x": 100, "start_y": 100, "end_x": 200, "end_y": 200}),
        "ui_swipe",
    )
    .await;
    test_tool(
        UiLongPressTool::new(),
        json!({"x": 100, "y": 100}),
        "ui_long_press",
    )
    .await;
    test_tool(
        UiTypeTextTool::new(),
        json!({"text": "Hello World"}),
        "ui_type_text",
    )
    .await;
    test_tool(
        UiKeyPressTool::new(),
        json!({"key": "enter"}),
        "ui_key_press",
    )
    .await;
    test_tool(UiDescribeTool::new(), json!({}), "ui_describe").await;
    test_tool(
        UiScreenshotTool::new(),
        json!({"output_path": "/tmp/ui_screenshot.png"}),
        "ui_screenshot",
    )
    .await;

    println!("\n\n============================================================");
    println!("ALL TESTS COMPLETED");
    println!("============================================================\n");
}
