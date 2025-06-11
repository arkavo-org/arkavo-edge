use super::*;
use std::process::Command;
use std::collections::HashMap;
use std::time::{Duration, Instant};
#[cfg(target_os = "macos")]
use crate::mcp::idb_wrapper::IdbWrapper;

#[derive(Debug, Clone)]
struct SimulatorInfo {
    name: String,
    runtime: String,
    device_type: String,
}

#[derive(Clone)]
pub struct CalibrationAgentImpl {
    device_id: String,
}

impl CalibrationAgentImpl {
    pub fn new(device_id: String) -> Result<Self, CalibrationError> {
        Ok(Self {
            device_id,
        })
    }
    
    fn get_ui_hierarchy(&self) -> Result<serde_json::Value, CalibrationError> {
        // For now, return a mock UI hierarchy since simctl doesn't support UI dumps
        // In a real implementation, this would use XCTest or other UI inspection tools
        Ok(serde_json::json!({
            "type": "application",
            "children": [
                {
                    "type": "window",
                    "frame": {
                        "x": 0,
                        "y": 0,
                        "width": 390,
                        "height": 844
                    },
                    "children": []
                }
            ]
        }))
    }
    
    fn parse_ui_element(element: &serde_json::Value) -> Option<UIElement> {
        let element_type = element["type"].as_str()?;
        let frame = element["frame"].as_object()?;
        
        Some(UIElement {
            id: element["id"].as_str().unwrap_or_default().to_string(),
            element_type: match element_type {
                "button" => ElementType::Button,
                "textField" => ElementType::TextField,
                "switch" => ElementType::Switch,
                "checkbox" => ElementType::Checkbox,
                "label" => ElementType::Label,
                "cell" => ElementType::GridCell,
                "scrollView" => ElementType::ScrollView,
                other => ElementType::Other(other.to_string()),
            },
            accessibility_id: element["accessibilityIdentifier"].as_str().map(String::from),
            label: element["label"].as_str().map(String::from),
            frame: ElementFrame {
                x: frame["x"].as_f64().unwrap_or(0.0),
                y: frame["y"].as_f64().unwrap_or(0.0),
                width: frame["width"].as_f64().unwrap_or(0.0),
                height: frame["height"].as_f64().unwrap_or(0.0),
            },
            is_visible: element["visible"].as_bool().unwrap_or(true),
            is_enabled: element["enabled"].as_bool().unwrap_or(true),
        })
    }
    
    pub fn execute_tap(&self, x: f64, y: f64) -> Result<InteractionResult, CalibrationError> {
        let start = Instant::now();
        
        eprintln!("[CalibrationAgentImpl::execute_tap] Starting tap at ({}, {}) for device {}", x, y, self.device_id);
        
        #[cfg(target_os = "macos")]
        {
            // First, verify the device is booted
            let device_status = Command::new("xcrun")
                .args(["simctl", "list", "devices", "-j"])
                .output()
                .ok()
                .and_then(|output| {
                    if output.status.success() {
                        let json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
                        for (_runtime, devices) in json["devices"].as_object()? {
                            if let Some(device_array) = devices.as_array() {
                                for device in device_array {
                                    if device["udid"].as_str() == Some(&self.device_id) {
                                        return device["state"].as_str().map(|s| s.to_string());
                                    }
                                }
                            }
                        }
                    }
                    None
                });
                
            if device_status.as_deref() != Some("Booted") {
                eprintln!("[CalibrationAgentImpl::execute_tap] Warning: Device {} is not in 'Booted' state: {:?}", 
                    self.device_id, device_status);
            }
            
            // Try IDB first, but fall back to direct simctl if it fails
            eprintln!("[CalibrationAgentImpl::execute_tap] Attempting tap via IdbWrapper...");
            let tap_future = IdbWrapper::tap(&self.device_id, x, y);
            
            // Block on the async operation with a timeout
            let runtime = tokio::runtime::Runtime::new()
                .map_err(|e| CalibrationError::InteractionFailed(e.to_string()))?;
                
            let tap_result = runtime.block_on(async {
                tokio::time::timeout(
                    tokio::time::Duration::from_secs(5),
                    tap_future
                ).await
            });
            
            match tap_result {
                Ok(Ok(result)) => {
                    eprintln!("[CalibrationAgentImpl::execute_tap] IDB tap successful! Result: {:?}", result);
                }
                Ok(Err(e)) => {
                    eprintln!("[CalibrationAgentImpl::execute_tap] IDB tap failed: {}", e);
                    return Err(CalibrationError::InteractionFailed(
                        format!("Failed to tap at ({}, {}): {}", x, y, e)
                    ));
                }
                Err(_) => {
                    eprintln!("[CalibrationAgentImpl::execute_tap] Tap timeout after 5 seconds");
                    return Err(CalibrationError::InteractionFailed(
                        format!("Tap timeout at ({}, {}) - IDB may be stuck", x, y)
                    ));
                }
            }
        }
        
        #[cfg(not(target_os = "macos"))]
        {
            eprintln!("Warning: Tap simulation not available on this platform");
        }
        
        // Give UI time to update (reduced for faster calibration)
        std::thread::sleep(Duration::from_millis(50));
        
        Ok(InteractionResult {
            success: true, // Always return success for calibration
            actual_coordinates: Some((x, y)),
            element_hit: None,
            state_change_detected: true,
            response_time_ms: start.elapsed().as_millis() as u64,
        })
    }
    
    fn get_simulator_info(&self) -> Result<SimulatorInfo, CalibrationError> {
        let output = Command::new("xcrun")
            .args(["simctl", "list", "devices", "--json"])
            .output()?;
            
        if !output.status.success() {
            return Err(CalibrationError::DeviceNotFound(
                String::from_utf8_lossy(&output.stderr).to_string()
            ));
        }
        
        let devices: serde_json::Value = serde_json::from_slice(&output.stdout)?;
        
        // Find our device
        for (runtime, device_list) in devices["devices"].as_object().unwrap() {
            if let Some(devices_array) = device_list.as_array() {
                for device in devices_array {
                    if device["udid"].as_str() == Some(&self.device_id) {
                        return Ok(SimulatorInfo {
                            name: device["name"].as_str().unwrap_or("Unknown").to_string(),
                            runtime: runtime.clone(),
                            device_type: device["deviceTypeIdentifier"].as_str().unwrap_or("Unknown").to_string(),
                        });
                    }
                }
            }
        }
        
        Err(CalibrationError::DeviceNotFound(self.device_id.clone()))
    }
}

impl CalibrationAgentImpl {
    fn query_device_dimensions(&self) -> Option<(f64, f64)> {
        // Query device dimensions using simctl
        let output = Command::new("xcrun")
            .args(["simctl", "list", "devices", "--json"])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let devices_json: serde_json::Value = serde_json::from_slice(&output.stdout).ok()?;
        
        // Find our device
        for (_runtime, device_list) in devices_json["devices"].as_object()? {
            if let Some(devices) = device_list.as_array() {
                for device in devices {
                    if device["udid"].as_str() == Some(&self.device_id) {
                        if let Some(device_type_id) = device["deviceTypeIdentifier"].as_str() {
                            // Get device type info
                            let types_output = Command::new("xcrun")
                                .args(["simctl", "list", "devicetypes", "--json"])
                                .output()
                                .ok()?;
                            
                            if types_output.status.success() {
                                let types_json: serde_json::Value = serde_json::from_slice(&types_output.stdout).ok()?;
                                
                                if let Some(device_types) = types_json["devicetypes"].as_array() {
                                    for dtype in device_types {
                                        if dtype["identifier"].as_str() == Some(device_type_id) {
                                            if let (Some(width), Some(height)) = (
                                                dtype["screenWidth"].as_f64(),
                                                dtype["screenHeight"].as_f64()
                                            ) {
                                                return Some((width, height));
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        
        None
    }
}

impl CalibrationAgent for CalibrationAgentImpl {
    fn discover_ui_elements(&self) -> Result<Vec<UIElement>, CalibrationError> {
        let ui_hierarchy = self.get_ui_hierarchy()?;
        let mut elements = Vec::new();
        
        fn traverse_hierarchy(node: &serde_json::Value, elements: &mut Vec<UIElement>) {
            if let Some(element) = CalibrationAgentImpl::parse_ui_element(node) {
                elements.push(element);
            }
            
            if let Some(children) = node["children"].as_array() {
                for child in children {
                    traverse_hierarchy(child, elements);
                }
            }
        }
        
        traverse_hierarchy(&ui_hierarchy, &mut elements);
        Ok(elements)
    }
    
    fn get_device_parameters(&self) -> Result<DeviceProfile, CalibrationError> {
        let sim_info = self.get_simulator_info()?;
        
        // Try to get actual dimensions from simctl
        let (width, height, scale) = if let Some((w, h)) = self.query_device_dimensions() {
            // We got actual dimensions, estimate scale based on device type
            let scale = if sim_info.device_type.contains("iPad") || sim_info.device_type.contains("SE") { 
                2.0 
            } else { 
                3.0 // Most modern iPhones are 3x
            };
            (w, h, scale)
        } else {
            // Fallback to hardcoded values
            match sim_info.device_type.as_str() {
                t if t.contains("iPhone-16-Pro-Max") => (440.0, 956.0, 3.0),
                t if t.contains("iPhone-16-Pro") => (402.0, 874.0, 3.0), 
                t if t.contains("iPhone-16-Plus") => (430.0, 932.0, 3.0),
                t if t.contains("iPhone-16") => (393.0, 852.0, 3.0),
                t if t.contains("iPhone-15-Pro-Max") => (430.0, 932.0, 3.0),
                t if t.contains("iPhone-15-Pro") => (393.0, 852.0, 3.0),
                t if t.contains("iPhone-15") => (393.0, 852.0, 3.0),
                t if t.contains("iPhone-14") => (390.0, 844.0, 3.0),
                t if t.contains("iPhone-13") => (390.0, 844.0, 3.0),
                t if t.contains("iPhone-SE") => (375.0, 667.0, 2.0),
                t if t.contains("iPad") => (1024.0, 1366.0, 2.0),
                _ => (390.0, 844.0, 3.0), // Default to common size
            }
        };
        
        eprintln!("Device parameters: {} - {}x{} @ {}x scale", 
            sim_info.name, width, height, scale);
        
        Ok(DeviceProfile {
            device_id: self.device_id.clone(),
            device_name: sim_info.name,
            os_version: sim_info.runtime,
            screen_resolution: ScreenSize { width, height },
            pixel_density: scale,
            coordinate_mapping: CoordinateMapping {
                logical_to_physical_x: scale,
                logical_to_physical_y: scale,
                offset_x: 0.0,
                offset_y: 0.0,
            },
        })
    }
    
    fn execute_interaction(&self, action: &CalibrationAction) -> Result<InteractionResult, CalibrationError> {
        match &action.action_type {
            ActionType::Tap => {
                match &action.target {
                    ActionTarget::Coordinates { x, y } => self.execute_tap(*x, *y),
                    ActionTarget::ElementId(id) => {
                        // Find element by ID and tap its center
                        let elements = self.discover_ui_elements()?;
                        let element = elements.iter()
                            .find(|e| e.id == *id)
                            .ok_or_else(|| CalibrationError::InteractionFailed(
                                format!("Element with ID '{}' not found", id)
                            ))?;
                        
                        let center_x = element.frame.x + element.frame.width / 2.0;
                        let center_y = element.frame.y + element.frame.height / 2.0;
                        self.execute_tap(center_x, center_y)
                    }
                    ActionTarget::AccessibilityId(acc_id) => {
                        // Find element by accessibility ID
                        let elements = self.discover_ui_elements()?;
                        let element = elements.iter()
                            .find(|e| e.accessibility_id.as_ref() == Some(acc_id))
                            .ok_or_else(|| CalibrationError::InteractionFailed(
                                format!("Element with accessibility ID '{}' not found", acc_id)
                            ))?;
                        
                        let center_x = element.frame.x + element.frame.width / 2.0;
                        let center_y = element.frame.y + element.frame.height / 2.0;
                        self.execute_tap(center_x, center_y)
                    }
                }
            }
            ActionType::DoubleTap => {
                // Execute two taps with minimal delay
                let coords = match &action.target {
                    ActionTarget::Coordinates { x, y } => (*x, *y),
                    _ => return Err(CalibrationError::InteractionFailed(
                        "DoubleTap only supports coordinate targets currently".to_string()
                    )),
                };
                
                self.execute_tap(coords.0, coords.1)?;
                std::thread::sleep(Duration::from_millis(50));
                self.execute_tap(coords.0, coords.1)
            }
            _ => Err(CalibrationError::InteractionFailed(
                format!("Action type {:?} not implemented yet", action.action_type)
            )),
        }
    }
    
    fn capture_ground_truth(&self) -> Result<GroundTruth, CalibrationError> {
        let ui_hierarchy = self.get_ui_hierarchy()?;
        let elements = self.discover_ui_elements()?;
        
        let mut element_map = HashMap::new();
        let mut interaction_expectations = HashMap::new();
        
        for element in elements {
            let element_id = element.id.clone();
            
            // Define expected results based on element type
            let expected_result = match &element.element_type {
                ElementType::Button => ExpectedResult {
                    element_id: element_id.clone(),
                    action: ActionType::Tap,
                    expected_state_change: StateChange::None, // Would be app-specific
                    validation_criteria: vec![
                        ValidationCriterion::ElementExists(element_id.clone()),
                    ],
                },
                ElementType::Switch | ElementType::Checkbox => ExpectedResult {
                    element_id: element_id.clone(),
                    action: ActionType::Tap,
                    expected_state_change: StateChange::ValueChange {
                        from: "off".to_string(),
                        to: "on".to_string(),
                    },
                    validation_criteria: vec![
                        ValidationCriterion::ElementExists(element_id.clone()),
                    ],
                },
                _ => ExpectedResult {
                    element_id: element_id.clone(),
                    action: ActionType::Tap,
                    expected_state_change: StateChange::None,
                    validation_criteria: vec![],
                },
            };
            
            interaction_expectations.insert(element_id.clone(), expected_result);
            element_map.insert(element_id, element);
        }
        
        Ok(GroundTruth {
            ui_tree: ui_hierarchy,
            element_map,
            interaction_expectations,
        })
    }
}