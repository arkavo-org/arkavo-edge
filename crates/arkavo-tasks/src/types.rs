//! Core types for task management.
//!
//! Task-related types are re-exported from arkavo-protocol for consistency.

pub use arkavo_protocol::types::{Message, MessagePart, TaskProgress, TaskStatus};

// Re-export TaskErrorInfo from arkavo_protocol for data structures
pub use arkavo_protocol::types::TaskError as TaskErrorInfo;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Unique task identifier
pub type TaskId = Uuid;

/// A task with all its metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub status: TaskStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub assigned_agent: Option<String>,
    pub parent_task: Option<Uuid>,
    /// Current progress
    pub progress: Option<TaskProgress>,
    /// Error information if task failed
    pub error: Option<TaskErrorInfo>,
    /// Task priority
    pub priority: TaskPriority,
    /// Task input message
    pub message: Message,
    /// Agent card if assigned
    pub agent_card: Option<AgentCard>,
    /// Task result
    pub result: Option<serde_json::Value>,
}

/// Task priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskPriority {
    Low,
    Normal,
    High,
    Critical,
}

/// Agent card describing capabilities
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCard {
    pub id: String,
    pub name: String,
    pub capabilities: Vec<String>,
    pub endpoint: String,
}

/// Task offer for agent negotiation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskOffer {
    pub task_id: Uuid,
    pub agent_id: String,
    pub price: Option<f64>,
    pub time_estimate_seconds: Option<u64>,
    /// Original intent ID
    pub intent_id: String,
    /// Intent description
    pub intent: String,
    /// Device capabilities
    pub device_caps: DeviceCapabilities,
}

/// Device capabilities for agent
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceCapabilities {
    pub can_execute_code: bool,
    pub can_access_files: bool,
    pub can_make_http_requests: bool,
}

/// Platform-specific device capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DevicePlatform {
    Windows,
    MacOS,
    Linux,
    IOS,
    Android,
    Web,
}

/// Agent capabilities for A2A protocol
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentCapabilities {
    pub name: String,
    pub version: String,
    pub capabilities: Vec<String>,
}

/// AI capability types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AiCapability {
    TextGeneration,
    ImageGeneration,
    CodeExecution,
    DataAnalysis,
}

/// Agent provider information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentProvider {
    pub name: String,
    pub organization: String,
}

/// Agent extension metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentExtension {
    pub name: String,
    pub version: String,
}

/// Agent skill description
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSkill {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// Security scheme for agent authentication
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityScheme {
    pub scheme_type: SecuritySchemeType,
    pub description: String,
}

/// Security scheme types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SecuritySchemeType {
    ApiKey,
    Http,
    OAuth2,
}

/// Security requirement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityRequirement {
    pub scheme: String,
    pub scopes: Vec<String>,
}

/// Sensor types for device capabilities
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SensorType {
    Camera,
    Microphone,
    Gps,
    Accelerometer,
}
