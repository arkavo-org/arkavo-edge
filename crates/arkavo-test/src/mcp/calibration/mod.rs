pub mod agent;
pub mod data;
pub mod reference_app;
pub mod server;
pub mod validation;
pub mod verification;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationConfig {
    pub device_id: String,
    pub device_type: String,
    pub screen_size: ScreenSize,
    pub safe_area: SafeArea,
    pub scale_factor: f64,
    pub calibration_version: String,
    pub last_calibrated: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenSize {
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafeArea {
    pub top: f64,
    pub bottom: f64,
    pub left: f64,
    pub right: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationResult {
    pub success: bool,
    pub device_profile: DeviceProfile,
    pub interaction_adjustments: HashMap<String, InteractionAdjustment>,
    pub edge_cases: Vec<EdgeCase>,
    pub validation_report: ValidationReport,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub device_id: String,
    pub device_name: String,
    pub os_version: String,
    pub screen_resolution: ScreenSize,
    pub pixel_density: f64,
    pub coordinate_mapping: CoordinateMapping,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoordinateMapping {
    pub logical_to_physical_x: f64,
    pub logical_to_physical_y: f64,
    pub offset_x: f64,
    pub offset_y: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionAdjustment {
    pub element_type: String,
    pub tap_offset: Option<(f64, f64)>,
    pub requires_double_tap: bool,
    pub requires_long_press: bool,
    pub custom_delay_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeCase {
    pub element_id: String,
    pub issue_type: String,
    pub solution: String,
    pub coordinates: Option<(f64, f64)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationReport {
    pub total_interactions: usize,
    pub successful_interactions: usize,
    pub failed_interactions: usize,
    pub accuracy_percentage: f64,
    pub issues: Vec<ValidationIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidationIssue {
    pub element_id: String,
    pub expected_result: String,
    pub actual_result: String,
    pub severity: IssueSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum IssueSeverity {
    Critical,
    Major,
    Minor,
}

pub trait CalibrationAgent {
    fn discover_ui_elements(&self) -> Result<Vec<UIElement>, CalibrationError>;
    fn get_device_parameters(&self) -> Result<DeviceProfile, CalibrationError>;
    fn execute_interaction(&self, action: &CalibrationAction) -> Result<InteractionResult, CalibrationError>;
    fn capture_ground_truth(&self) -> Result<GroundTruth, CalibrationError>;
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UIElement {
    pub id: String,
    pub element_type: ElementType,
    pub accessibility_id: Option<String>,
    pub label: Option<String>,
    pub frame: ElementFrame,
    pub is_visible: bool,
    pub is_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ElementType {
    Button,
    TextField,
    Switch,
    Checkbox,
    Label,
    GridCell,
    ScrollView,
    Other(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ElementFrame {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalibrationAction {
    pub action_type: ActionType,
    pub target: ActionTarget,
    pub parameters: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionType {
    Tap,
    DoubleTap,
    LongPress,
    Swipe,
    Scroll,
    TypeText,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ActionTarget {
    Coordinates { x: f64, y: f64 },
    ElementId(String),
    AccessibilityId(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractionResult {
    pub success: bool,
    pub actual_coordinates: Option<(f64, f64)>,
    pub element_hit: Option<String>,
    pub state_change_detected: bool,
    pub response_time_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroundTruth {
    pub ui_tree: serde_json::Value,
    pub element_map: HashMap<String, UIElement>,
    pub interaction_expectations: HashMap<String, ExpectedResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedResult {
    pub element_id: String,
    pub action: ActionType,
    pub expected_state_change: StateChange,
    pub validation_criteria: Vec<ValidationCriterion>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StateChange {
    ValueChange { from: String, to: String },
    VisibilityChange { visible: bool },
    EnabledChange { enabled: bool },
    NavigationChange { to_screen: String },
    None,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValidationCriterion {
    ElementExists(String),
    ElementValue { id: String, expected_value: String },
    ScreenContains(String),
    Custom(String),
}

#[derive(Debug, thiserror::Error)]
pub enum CalibrationError {
    #[error("Device not found: {0}")]
    DeviceNotFound(String),
    
    #[error("UI discovery failed: {0}")]
    UIDiscoveryFailed(String),
    
    #[error("Interaction failed: {0}")]
    InteractionFailed(String),
    
    #[error("Validation error: {0}")]
    ValidationError(String),
    
    #[error("Configuration error: {0}")]
    ConfigurationError(String),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),
}