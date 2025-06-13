use super::*;
use serde_json;
use std::process::Command;
use std::thread;
use std::time::Duration;

#[derive(Clone)]
pub struct ReferenceAppInterface {
    device_id: String,
    bundle_id: String,
    diagnostic_endpoint: Option<String>,
    deep_link_scheme: String,
}

impl ReferenceAppInterface {
    pub fn new(device_id: String) -> Self {
        Self {
            device_id,
            bundle_id: "com.arkavo.reference".to_string(), // Correct bundle ID from Info.plist
            diagnostic_endpoint: None,
            deep_link_scheme: "arkavo-edge".to_string(),
        }
    }

    pub fn with_bundle_id(mut self, bundle_id: String) -> Self {
        self.bundle_id = bundle_id;
        self
    }

    pub fn with_diagnostic_endpoint(mut self, endpoint: String) -> Self {
        self.diagnostic_endpoint = Some(endpoint);
        self
    }

    fn get_url_dialog_coordinates(&self) -> (f64, f64) {
        // Default coordinates for "Open" button in URL confirmation dialog
        // This is typically centered on most iPhone models
        (195.0, 490.0)
    }

    pub fn launch(&self) -> Result<(), CalibrationError> {
        // Check if reference app is available
        let check_output = Command::new("xcrun")
            .args([
                "simctl",
                "get_app_container",
                &self.device_id,
                &self.bundle_id,
            ])
            .output()?;

        if !check_output.status.success() {
            // Reference app not installed - this is a critical error
            return Err(CalibrationError::ConfigurationError(format!(
                "ArkavoReference app '{}' is not installed. Cannot proceed with calibration.",
                self.bundle_id
            )));
        }

        // Terminate if already running
        let _ = Command::new("xcrun")
            .args(["simctl", "terminate", &self.device_id, &self.bundle_id])
            .output();

        thread::sleep(Duration::from_millis(500));

        // Launch the app normally - it will start in calibration mode by default
        eprintln!("Launching reference app (default view is calibration)...");
        eprintln!("Device ID: {}", self.device_id);
        eprintln!("Bundle ID: {}", self.bundle_id);

        let output = Command::new("xcrun")
            .args(["simctl", "launch", &self.device_id, &self.bundle_id])
            .output()?;

        if !output.status.success() {
            return Err(CalibrationError::ConfigurationError(format!(
                "Failed to launch app: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        eprintln!("Successfully launched reference app - calibration view will be shown");

        // Wait for app to fully launch
        thread::sleep(Duration::from_secs(2));

        // Verify app is running using launchctl
        let verify_output = Command::new("xcrun")
            .args(["simctl", "spawn", &self.device_id, "launchctl", "list"])
            .output()?;

        if verify_output.status.success() {
            let output_str = String::from_utf8_lossy(&verify_output.stdout);
            let is_running = output_str
                .lines()
                .any(|line| line.contains(&format!("UIKitApplication:{}", self.bundle_id)));

            if is_running {
                eprintln!("App verified as running after launch");
            } else {
                eprintln!("Warning: App may not be running after launch attempt");
            }
        }

        Ok(())
    }

    pub fn is_available(&self) -> bool {
        Command::new("xcrun")
            .args([
                "simctl",
                "get_app_container",
                &self.device_id,
                &self.bundle_id,
            ])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    pub fn navigate_to_test_screen(&self, screen_name: &str) -> Result<(), CalibrationError> {
        // Use deep linking if available
        let deep_link = format!("{}://test/{}", self.deep_link_scheme, screen_name);

        let output = Command::new("xcrun")
            .args(["simctl", "openurl", &self.device_id, &deep_link])
            .output()?;

        if !output.status.success() {
            return Err(CalibrationError::InteractionFailed(format!(
                "Failed to navigate via deep link: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }

        // Handle URL confirmation dialog
        thread::sleep(Duration::from_millis(1500));
        let (x, y) = self.get_url_dialog_coordinates();
        let _ = Command::new("xcrun")
            .args([
                "simctl",
                "io",
                &self.device_id,
                "tap",
                &x.to_string(),
                &y.to_string(),
            ])
            .output();

        thread::sleep(Duration::from_millis(500));
        Ok(())
    }

    pub fn get_calibration_script(&self) -> CalibrationScript {
        // Check if reference app is available
        let check_output = Command::new("xcrun")
            .args([
                "simctl",
                "get_app_container",
                &self.device_id,
                &self.bundle_id,
            ])
            .output();

        let has_reference_app = check_output.map(|o| o.status.success()).unwrap_or(false);

        if has_reference_app {
            // Full calibration with reference app
            CalibrationScript {
                name: "Reference App Full Calibration".to_string(),
                description: "Complete calibration sequence for UI automation".to_string(),
                steps: vec![
                    CalibrationStep {
                        name: "Launch and Initialize".to_string(),
                        actions: vec![CalibrationAction {
                            action_type: ActionType::Tap,
                            target: ActionTarget::AccessibilityId("sign_in_button".to_string()),
                            parameters: HashMap::new(),
                        }],
                        validation: Some(ValidationCheck {
                            check_type: CheckType::ElementVisible("main_menu".to_string()),
                            timeout_ms: 5000,
                        }),
                    },
                    CalibrationStep {
                        name: "Grid Calibration".to_string(),
                        actions: Self::generate_grid_calibration_actions(),
                        validation: None,
                    },
                ],
            }
        } else {
            // Basic calibration without reference app
            CalibrationScript {
                name: "Basic Device Calibration".to_string(),
                description: "Basic calibration for device coordinate mapping".to_string(),
                steps: vec![
                    CalibrationStep {
                        name: "Screen Boundary Test".to_string(),
                        actions: vec![
                            // Test corners
                            CalibrationAction {
                                action_type: ActionType::Tap,
                                target: ActionTarget::Coordinates { x: 10.0, y: 10.0 },
                                parameters: HashMap::new(),
                            },
                            CalibrationAction {
                                action_type: ActionType::Tap,
                                target: ActionTarget::Coordinates { x: 380.0, y: 10.0 },
                                parameters: HashMap::new(),
                            },
                            CalibrationAction {
                                action_type: ActionType::Tap,
                                target: ActionTarget::Coordinates { x: 10.0, y: 834.0 },
                                parameters: HashMap::new(),
                            },
                            CalibrationAction {
                                action_type: ActionType::Tap,
                                target: ActionTarget::Coordinates { x: 380.0, y: 834.0 },
                                parameters: HashMap::new(),
                            },
                        ],
                        validation: None,
                    },
                    CalibrationStep {
                        name: "Grid Calibration".to_string(),
                        actions: Self::generate_grid_calibration_actions(),
                        validation: None,
                    },
                ],
            }
        }
    }

    fn generate_grid_calibration_actions() -> Vec<CalibrationAction> {
        let mut actions = vec![];

        // Create a smaller grid for faster calibration
        let grid_points = vec![
            (0.2, 0.2), // Top-left
            (0.8, 0.2), // Top-right
            (0.5, 0.5), // Center
            (0.2, 0.8), // Bottom-left
            (0.8, 0.8), // Bottom-right
        ];

        for (x_percent, y_percent) in grid_points {
            actions.push(CalibrationAction {
                action_type: ActionType::Tap,
                target: ActionTarget::Coordinates {
                    x: x_percent * 390.0, // Assuming iPhone 14 dimensions
                    y: y_percent * 844.0,
                },
                parameters: HashMap::new(),
            });
        }

        actions
    }

    pub fn read_diagnostic_data(&self) -> Result<DiagnosticData, CalibrationError> {
        if let Some(_endpoint) = &self.diagnostic_endpoint {
            // In a real implementation, this would make an HTTP request
            // For now, return mock data
            Ok(DiagnosticData {
                timestamp: chrono::Utc::now(),
                ui_state: serde_json::json!({
                    "current_screen": "test_components",
                    "checkboxes": {
                        "checkbox_1": true,
                        "checkbox_2": false,
                        // ... etc
                    }
                }),
                interaction_log: vec![],
                performance_metrics: PerformanceMetrics {
                    avg_response_time_ms: 50,
                    frame_rate: 60.0,
                    memory_usage_mb: 128.5,
                },
            })
        } else {
            Err(CalibrationError::ConfigurationError(
                "No diagnostic endpoint configured".to_string(),
            ))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationScript {
    pub name: String,
    pub description: String,
    pub steps: Vec<CalibrationStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationStep {
    pub name: String,
    pub actions: Vec<CalibrationAction>,
    pub validation: Option<ValidationCheck>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationCheck {
    pub check_type: CheckType,
    pub timeout_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CheckType {
    ElementVisible(String),
    ElementValue { id: String, expected: String },
    ScreenContains(String),
    Custom(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticData {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub ui_state: serde_json::Value,
    pub interaction_log: Vec<InteractionLogEntry>,
    pub performance_metrics: PerformanceMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionLogEntry {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub action: String,
    pub target: String,
    pub result: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceMetrics {
    pub avg_response_time_ms: u64,
    pub frame_rate: f64,
    pub memory_usage_mb: f64,
}
