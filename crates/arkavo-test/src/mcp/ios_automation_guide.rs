use super::server::{Tool, ToolSchema};
use crate::Result;
use async_trait::async_trait;
use serde_json::Value;

/// Guide tool to help AI agents understand the correct iOS automation workflow
pub struct IosAutomationGuide {
    schema: ToolSchema,
}

impl IosAutomationGuide {
    pub fn new() -> Self {
        Self {
            schema: ToolSchema {
                name: "ios_automation_guide".to_string(),
                description: "Get the recommended workflow for iOS UI automation. Use this if you're unsure how to automate iOS apps or which tools to use. Returns step-by-step instructions for the most reliable approach.".to_string(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "scenario": {
                            "type": "string",
                            "enum": ["getting_started", "tap_button", "enter_text", "verify_screen", "handle_dialogs", "debug_issues"],
                            "description": "What you're trying to accomplish"
                        }
                    }
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for IosAutomationGuide {
    async fn execute(&self, params: Value) -> Result<Value> {
        let scenario = params
            .get("scenario")
            .and_then(|v| v.as_str())
            .unwrap_or("getting_started");

        let guide = match scenario {
            "getting_started" => serde_json::json!({
                "workflow": "iOS UI Automation Quick Start",
                "overview": "arkavo-edge provides fast, reliable iOS automation using Apple's private AXP APIs",
                "critical": "⚠️ ALWAYS BUILD AXP HARNESS FIRST! Without it, taps take 300ms+ and may fail with IDB errors.",
                "steps": [
                    {
                        "step": 1,
                        "action": "Boot a simulator (if not already running)",
                        "tool": "device_management",
                        "example": {
                            "action": "boot",
                            "device_name": "iPhone 15"
                        },
                        "note": "Skip if simulator is already booted"
                    },
                    {
                        "step": 2,
                        "action": "🚀 BUILD AXP HARNESS FIRST (ONE TIME SETUP)",
                        "tool": "build_test_harness",
                        "example": {
                            "app_bundle_id": "<YOUR_APP_BUNDLE_ID>"
                        },
                        "note": "Replace <YOUR_APP_BUNDLE_ID> with your app's bundle identifier (e.g., com.company.appname)",
                        "critical": "THIS IS REQUIRED! Without AXP harness:",
                        "problems_without_axp": [
                            "❌ Taps take 300ms+ (vs <30ms with AXP)",
                            "❌ IDB may fail with port conflicts",
                            "❌ Fallback methods are unreliable",
                            "❌ Tests will be 10x slower"
                        ],
                        "important": "Run this ONCE per app. It creates fast touch injection that works every time.",
                        "troubleshooting": "If this fails, ensure Xcode command line tools are installed"
                    },
                    {
                        "step": 3,
                        "action": "Launch your app",
                        "tool": "app_launcher",
                        "example": {
                            "bundle_id": "<YOUR_APP_BUNDLE_ID>"
                        }
                    },
                    {
                        "step": 4,
                        "action": "Take screenshot to see UI",
                        "tool": "screen_capture",
                        "example": {}
                    },
                    {
                        "step": 5,
                        "action": "Read screenshot to identify elements",
                        "tool": "Read (built-in)",
                        "example": "Read the .png file from screen_capture"
                    },
                    {
                        "step": 6,
                        "action": "Tap using coordinates",
                        "tool": "ui_interaction",
                        "example": {
                            "action": "tap",
                            "target": {"x": 200, "y": 400}
                        },
                        "result": "With AXP harness: <30ms tap. Without: 300ms+ or failure."
                    }
                ],
                "summary": "1) BUILD_TEST_HARNESS FIRST, 2) Use coordinates from screenshots, 3) Enjoy fast, reliable automation",
                "important": "ALWAYS use coordinates! They're fast and reliable.",
                "avoid": [
                    "DO NOT skip build_test_harness - it prevents IDB failures",
                    "DO NOT use text-based tapping - unreliable and slow",
                    "DO NOT use setup_xcuitest - deprecated"
                ]
            }),

            "tap_button" => serde_json::json!({
                "workflow": "Tapping a Button",
                "steps": [
                    {
                        "step": 1,
                        "action": "Take screenshot",
                        "tool": "screen_capture",
                        "why": "To see current UI state"
                    },
                    {
                        "step": 2,
                        "action": "Read screenshot image",
                        "tool": "Read",
                        "why": "To visually identify button location"
                    },
                    {
                        "step": 3,
                        "action": "Tap at button coordinates",
                        "tool": "ui_interaction",
                        "example": {
                            "action": "tap",
                            "target": {"x": 196, "y": 680}
                        }
                    }
                ],
                "tips": [
                    "Estimate coordinates from visual inspection",
                    "Button centers work best",
                    "For iPhone 15: screen is 393x852 logical points",
                    "AXP harness makes taps instant (<30ms)"
                ]
            }),

            "enter_text" => serde_json::json!({
                "workflow": "Entering Text",
                "steps": [
                    {
                        "step": 1,
                        "action": "Tap the text field first",
                        "tool": "ui_interaction",
                        "example": {
                            "action": "tap",
                            "target": {"x": 200, "y": 300}
                        },
                        "why": "To focus the text field"
                    },
                    {
                        "step": 2,
                        "action": "Clear existing text",
                        "tool": "ui_interaction",
                        "example": {
                            "action": "clear_text"
                        }
                    },
                    {
                        "step": 3,
                        "action": "Type new text",
                        "tool": "ui_interaction",
                        "example": {
                            "action": "type_text",
                            "value": "user@example.com"
                        }
                    }
                ],
                "important": "MUST tap field first to focus it!"
            }),

            "verify_screen" => serde_json::json!({
                "workflow": "Verifying Screen Content",
                "steps": [
                    {
                        "step": 1,
                        "action": "Take screenshot",
                        "tool": "screen_capture"
                    },
                    {
                        "step": 2,
                        "action": "Read screenshot",
                        "tool": "Read",
                        "why": "Use vision to check for expected elements"
                    },
                    {
                        "step": 3,
                        "action": "Analyze what you see",
                        "note": "Look for buttons, text, UI state"
                    }
                ],
                "tip": "Visual verification is more reliable than programmatic queries"
            }),

            "handle_dialogs" => serde_json::json!({
                "workflow": "Handling System Dialogs",
                "options": {
                    "biometric_prompts": {
                        "tool": "biometric_auth",
                        "example": {
                            "action": "authenticate",
                            "success": true
                        }
                    },
                    "permission_dialogs": {
                        "tool": "system_dialog",
                        "example": {
                            "action": "handle_alert",
                            "button": "Allow"
                        }
                    },
                    "custom_alerts": {
                        "approach": "Use screen_capture + coordinates",
                        "tip": "Alert buttons are usually at bottom"
                    }
                }
            }),

            "debug_issues" => serde_json::json!({
                "workflow": "Debugging Automation Issues",
                "common_problems": {
                    "tap_not_working": [
                        "Ensure build_test_harness was run for the app",
                        "Check coordinates are within screen bounds",
                        "Take screenshot to verify UI state",
                        "Try waiting 1-2 seconds after app launch"
                    ],
                    "text_not_typing": [
                        "Make sure to tap the field first",
                        "Use clear_text before typing",
                        "Check if keyboard is showing in screenshot"
                    ],
                    "app_not_launching": [
                        "Verify bundle ID is correct",
                        "Check if app is installed (app_management tool)",
                        "Try booting a fresh simulator"
                    ]
                },
                "diagnostic_tools": {
                    "device_logs": "log_stream tool",
                    "app_state": "app_diagnostic tool",
                    "simulator_state": "device_management with 'list' action"
                },
                "ios26_beta_issues": [
                    "AXP symbols not available - this is expected",
                    "build_test_harness will use minimal mode automatically",
                    "IDB (idb_companion) will be used for touch injection",
                    "Performance: ~100ms taps instead of <30ms",
                    "Solution: Install Xcode 16 beta or use iOS 18 simulator"
                ]
            }),

            _ => serde_json::json!({
                "error": "Unknown scenario",
                "available_scenarios": [
                    "getting_started",
                    "tap_button",
                    "enter_text",
                    "verify_screen",
                    "handle_dialogs",
                    "debug_issues"
                ]
            }),
        };

        Ok(guide)
    }

    fn schema(&self) -> &ToolSchema {
        &self.schema
    }
}
