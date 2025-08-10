use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Request to make a task from another agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskRequest {
    /// The ID of the requesting agent
    #[serde(rename = "agent_id")]
    #[schemars(example = "example_agent_id")]
    pub agent_id: String,

    /// The type of task being requested
    #[serde(rename = "task_type")]
    #[schemars(example = "example_task_type")]
    pub task_type: String,

    /// Additional data for the task request
    #[serde(rename = "payload", skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// Response to a task request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskResponse {
    /// Unique identifier for the task
    #[serde(rename = "task_id")]
    pub task_id: Uuid,

    /// Current status of the task
    pub status: TaskStatus,

    /// Additional data in the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Status of a task
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    /// Task has been submitted
    Submitted,
    /// Task is being worked on
    Working,
    /// Task requires input from the requester
    InputRequired,
    /// Task has been completed successfully
    Completed,
    /// Task has been canceled
    Canceled,
    /// Task has failed
    Failed,
    /// Task has been rejected
    Rejected,
    /// Task requires authentication
    AuthRequired,
}

/// Declaration of tasks an agent can fulfill
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskDeclareRequest {
    /// The ID of the declaring agent
    #[serde(rename = "agent_id")]
    pub agent_id: String,

    /// List of tasks the agent can fulfill
    pub tasks: Vec<TaskCapability>,
}

/// A task capability that an agent can fulfill
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskCapability {
    /// Type of task
    #[serde(rename = "type")]
    pub task_type: String,

    /// Constraints on the task
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<serde_json::Value>,

    /// Additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Response to a task declaration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskDeclareResponse {
    /// Whether the declaration was acknowledged
    pub acknowledged: bool,

    /// Timestamp of acknowledgment
    pub timestamp: DateTime<Utc>,
}

/// Request to discover agents
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentDiscoverRequest {
    /// Optional filter criteria
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<AgentDiscoverFilter>,
}

/// Filter criteria for agent discovery
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentDiscoverFilter {
    /// Filter by task types
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_types: Option<Vec<String>>,

    /// Filter by tags
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Information about a discovered agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscoveredAgent {
    /// Unique identifier for the agent
    pub agent_id: Uuid,

    /// Network endpoint for the agent
    pub endpoint: String,

    /// Task types the agent supports
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tasks: Option<Vec<String>>,

    /// Additional metadata about the agent
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// DIDComm Discover Features Query (RFC 0031/0557)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscoverFeaturesQuery {
    /// Feature types to query (protocols, goal-codes, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub queries: Option<Vec<FeatureQuery>>,
}

/// Individual feature query
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeatureQuery {
    /// Feature type to query
    #[serde(rename = "feature-type")]
    pub feature_type: FeatureType,

    /// Optional match pattern (supports wildcards)
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    pub match_pattern: Option<String>,
}

/// DIDComm Discover Features Disclosure (RFC 0031/0557)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiscoverFeaturesDisclose {
    /// List of supported features
    pub disclosures: Vec<FeatureDisclosure>,
}

/// Individual feature disclosure
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FeatureDisclosure {
    /// Feature type
    #[serde(rename = "feature-type")]
    pub feature_type: FeatureType,

    /// Feature identifier (protocol ID, goal code, etc.)
    pub id: String,

    /// Supported roles for this feature (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
}

/// Types of features that can be discovered
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum FeatureType {
    /// DIDComm protocol support
    Protocol,
    /// Goal codes the agent can fulfill
    GoalCode,
    /// Governance frameworks
    Gov,
    /// MCP tools (custom extension)
    McpTool,
    /// MCP servers (custom extension)
    McpServer,
}

/// Agent Card - JSON metadata document describing an agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentCard {
    /// Agent identity information
    pub identity: AgentIdentity,

    /// Agent capabilities
    pub capabilities: Vec<AgentCapability>,

    /// Authentication requirements
    pub authentication: AuthenticationRequirements,

    /// Supported interaction modes
    pub interaction_modes: Vec<InteractionMode>,

    /// Optional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Agent identity information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentIdentity {
    /// Unique agent ID
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Agent description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Agent version
    pub version: String,

    /// Organization or owner
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization: Option<String>,
}

/// Agent capability description
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentCapability {
    /// Skill or capability name
    pub name: String,

    /// Capability description
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Input schema for this capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,

    /// Output schema for this capability
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
}

/// Authentication requirements
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AuthenticationRequirements {
    /// Supported authentication methods
    pub methods: Vec<AuthMethod>,

    /// Whether authentication is required
    pub required: bool,
}

/// Authentication method
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    /// OAuth2 authentication
    OAuth2,
    /// JWT bearer token
    JwtBearer,
    /// API key
    ApiKey,
    /// mTLS client certificate
    Mtls,
    /// Basic authentication
    Basic,
}

/// Request from one agent to query another agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentQueryRequest {
    /// ID of the requesting agent
    pub from_agent_id: String,

    /// ID of the target agent (optional, can be broadcast)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_agent_id: Option<String>,

    /// The query/question being asked
    pub query: String,

    /// Optional context for the query
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,

    /// Domain or capability being queried
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
}

/// Response to an agent query
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentQueryResponse {
    /// ID of the responding agent
    pub from_agent_id: String,

    /// The response to the query
    pub response: String,

    /// Confidence score (0.0 to 1.0)
    pub confidence: f32,

    /// Domain expertise of the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,

    /// Optional supporting evidence or context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<serde_json::Value>,
}

/// Agent status enumeration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Agent is online and available
    Online,
    /// Agent is offline
    Offline,
    /// Agent is busy with tasks
    Busy,
    /// Agent is in maintenance mode
    Maintenance,
    /// Seeking collaboration
    SeekingCollaboration,
}

/// Broadcast message for agent capabilities
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentBroadcast {
    /// ID of the broadcasting agent
    pub agent_id: String,

    /// Type of broadcast
    pub broadcast_type: BroadcastType,

    /// Capabilities or specializations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<Vec<AgentCapability>>,

    /// Optional status update
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentStatus>,

    /// Whether the agent is accepting new tasks
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accepting_tasks: Option<bool>,

    /// Additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Types of broadcasts agents can make
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BroadcastType {
    /// Status update broadcast
    Status,
    /// Capability announcement
    Capability,
    /// Availability change
    Availability,
    /// Agent shutdown notification
    Shutdown,
    /// Custom broadcast type
    Custom(String),
}

/// Interaction mode supported by the agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum InteractionMode {
    /// Synchronous request/response
    Synchronous,
    /// Server-sent events streaming
    Streaming,
    /// Asynchronous with push notifications
    Asynchronous,
}

// Helper function for examples
fn example_agent_id() -> &'static str {
    "550e8400-e29b-41d4-a716-446655440000"
}

fn example_task_type() -> &'static str {
    "data_access"
}

/// Request to open a new chat session
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatOpenRequest {
    /// Optional initial context for the conversation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,

    /// Optional metadata about the chat session
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,

    /// Optional JWT token for authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
}

/// Metrics acknowledgment for back-pressure management
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MetricsAck {
    /// Session ID this acknowledgment is for
    pub session_id: String,
    /// Last sequence number received by the client
    pub last_seq: u64,
    /// Optional metrics about client buffer state
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_metrics: Option<ClientMetrics>,
}

/// Client-side metrics for monitoring
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClientMetrics {
    /// Number of messages in client buffer
    pub buffer_size: usize,
    /// Client processing latency in milliseconds
    pub processing_latency_ms: Option<u64>,
    /// Whether client is ready for more messages
    pub ready_for_more: bool,
}

/// Response from opening a chat session
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatSession {
    /// Unique session ID for this chat
    pub session_id: String,

    /// Agent capabilities for this session
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ChatCapabilities>,

    /// Timestamp when session was created
    pub created_at: DateTime<Utc>,
}

/// Capabilities of the chat session
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatCapabilities {
    /// Maximum context length
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_context_length: Option<u32>,

    /// Supported message types
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supported_message_types: Option<Vec<String>>,

    /// Whether file attachments are supported
    #[serde(default)]
    pub supports_attachments: bool,

    /// Whether tool calls are supported
    #[serde(default)]
    pub supports_tools: bool,
}

/// User message to send within a chat session
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UserMessage {
    /// The user's message text
    pub content: String,

    /// Optional attachments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<Attachment>>,

    /// Optional message metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Attachment for messages
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Attachment {
    /// MIME type of the attachment
    pub mime_type: String,

    /// Base64-encoded content or URL
    pub content: String,

    /// Whether content is a URL (true) or base64 data (false)
    #[serde(default)]
    pub is_url: bool,

    /// Optional filename
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
}

/// Legacy chat request (to be deprecated)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatRequest {
    /// The user's message
    pub message: String,

    /// Optional context for the conversation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,

    /// Optional session ID for multi-turn conversations
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Message delta for streaming responses
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MessageDelta {
    /// Session ID this delta belongs to
    pub session_id: String,

    /// The message ID this delta belongs to
    pub message_id: String,

    /// Sequence number for ordering
    pub sequence: u64,

    /// The delta content
    pub delta: MessageDeltaContent,

    /// Timestamp of the delta
    pub timestamp: DateTime<Utc>,
}

/// Content of a message delta
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MessageDeltaContent {
    /// Text content delta
    Text {
        /// The text to append
        text: String,
    },
    /// Tool call delta with streaming support
    ToolCall {
        /// Tool call ID
        tool_call_id: String,
        /// Tool name (only sent on first delta)
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// JSON fragment of arguments being streamed
        args_json_fragment: String,
        /// Whether this completes the tool call
        done: bool,
    },
    /// Stream ended
    StreamEnd {
        /// Reason for ending
        reason: StreamEndReason,
    },
    /// Error occurred
    Error {
        /// Error code
        code: String,
        /// Error message
        message: String,
    },
}

/// Reason for stream ending
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StreamEndReason {
    /// Completed normally
    Complete,
    /// Reached max tokens
    MaxTokens,
    /// User requested abort
    UserAbort,
    /// Error occurred
    Error,
    /// Session closed
    SessionClosed,
}

// A2A Protocol Types

/// Message send request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MessageSendRequest {
    /// The message content
    pub message: Message,

    /// Optional task context
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

/// Message for A2A communication
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Message {
    /// Message parts
    pub parts: Vec<MessagePart>,

    /// Optional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Part of a message
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessagePart {
    /// Text content
    Text {
        /// The text content
        content: String,
    },
    /// File content
    File {
        /// File name
        name: String,
        /// MIME type
        mime_type: String,
        /// File data (base64 encoded) or URI
        data: String,
        /// Whether data is a URI (true) or base64 (false)
        #[serde(default)]
        is_uri: bool,
    },
    /// Structured data
    Data {
        /// Data schema identifier
        schema: String,
        /// The actual data
        content: serde_json::Value,
    },
}

/// Message send response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MessageSendResponse {
    /// Task ID for this message
    pub task_id: String,

    /// Initial task status
    pub status: TaskStatus,

    /// Optional immediate response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<Message>,
}

/// Task get request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskGetRequest {
    /// Task ID to retrieve
    pub task_id: String,
}

/// Task get response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskGetResponse {
    /// Task ID
    pub task_id: String,

    /// Current task status
    pub status: TaskStatus,

    /// Task result if completed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Message>,

    /// Error details if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<TaskError>,

    /// Progress information
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<TaskProgress>,
}

/// Task error information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskError {
    /// Error code
    pub code: String,

    /// Error message
    pub message: String,

    /// Additional error details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

/// Task progress information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskProgress {
    /// Progress percentage (0-100)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub percentage: Option<u8>,

    /// Progress message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Estimated time remaining in seconds
    #[serde(skip_serializing_if = "Option::is_none")]
    pub eta_seconds: Option<u64>,
}

/// Task cancel request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskCancelRequest {
    /// Task ID to cancel
    pub task_id: String,

    /// Reason for cancellation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Task cancel response
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskCancelResponse {
    /// Whether the cancellation was successful
    pub success: bool,

    /// Final task status
    pub status: TaskStatus,

    /// Optional message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// Configuration management error types
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigError {
    /// Agent is offline or unreachable
    AgentOffline,
    /// Filesystem is read-only or write protected
    ReadOnlyFilesystem,
    /// Configuration validation failed
    ValidationFailed { details: String },
    /// Concurrent modification conflict
    Conflict { current_version: String },
    /// Unauthorized to modify configuration
    Unauthorized,
}

/// Request to get agent configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfigGetRequest {
    /// Agent ID requesting the configuration
    pub agent_id: String,
    /// Include backup versions
    #[serde(default)]
    pub include_backups: bool,
}

/// Response with agent configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfigGetResponse {
    /// Current AGENTS.md content
    pub content: String,
    /// Configuration version/hash
    pub version: String,
    /// Available backup versions
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backups: Option<Vec<ConfigBackup>>,
    /// Whether configuration is writable
    pub writable: bool,
}

/// Configuration backup information
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigBackup {
    /// Backup filename
    pub filename: String,
    /// Timestamp when backup was created
    pub timestamp: DateTime<Utc>,
    /// Size in bytes
    pub size: u64,
    /// Version/hash of this backup
    pub version: String,
}

/// Request to update agent configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfigUpdateRequest {
    /// Agent ID requesting the update
    pub agent_id: String,
    /// New AGENTS.md content
    pub content: String,
    /// Expected version for optimistic locking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_version: Option<String>,
    /// Whether to create a backup
    #[serde(default = "default_create_backup")]
    pub create_backup: bool,
}

fn default_create_backup() -> bool {
    true
}

/// Response to configuration update
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfigUpdateResponse {
    /// Whether update was successful
    pub success: bool,
    /// New configuration version
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_version: Option<String>,
    /// Path to backup if created
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup_path: Option<String>,
    /// Error if update failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ConfigError>,
    /// Whether agent reload is required
    #[serde(default)]
    pub reload_required: bool,
}

/// Request to validate configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfigValidateRequest {
    /// Agent ID requesting validation
    pub agent_id: String,
    /// Configuration content to validate
    pub content: String,
}

/// Response to configuration validation
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfigValidateResponse {
    /// Whether configuration is valid
    pub valid: bool,
    /// Validation errors if any
    #[serde(default)]
    pub errors: Vec<String>,
    /// Validation warnings
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// Request to restore configuration from backup
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfigRestoreRequest {
    /// Agent ID requesting restore
    pub agent_id: String,
    /// Backup filename to restore
    pub backup_filename: String,
}

/// Response to configuration restore
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentConfigRestoreResponse {
    /// Whether restore was successful
    pub success: bool,
    /// New configuration version after restore
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_version: Option<String>,
    /// Error if restore failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<ConfigError>,
}
