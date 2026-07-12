use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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

/// A2A Protocol Agent Card - JSON metadata document at /.well-known/agent.json
/// Conforms to A2A Protocol Specification v0.3+
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentCard {
    /// Human-readable name of the agent
    pub name: String,

    /// Description of the agent's purpose and capabilities
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// The base URL for the agent's A2A endpoint
    pub url: String,

    /// Information about the agent provider/organization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AgentProvider>,

    /// Version of this agent implementation
    pub version: String,

    /// A2A protocol versions this agent supports
    #[serde(default)]
    pub protocol_versions: Vec<String>,

    /// Default supported input content types
    #[serde(default)]
    pub default_input_modes: Vec<String>,

    /// Default supported output content types
    #[serde(default)]
    pub default_output_modes: Vec<String>,

    /// Agent capability flags
    pub capabilities: AgentCapabilities,

    /// Skills this agent can perform
    #[serde(default)]
    pub skills: Vec<AgentSkill>,

    /// Authentication schemes available
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security_schemes: Vec<SecurityScheme>,

    /// Required authentication configuration
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security: Vec<SecurityRequirement>,

    /// Optional extensions
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extensions: Vec<AgentExtension>,

    /// Optional signature for card verification (A2A v0.3+)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<AgentCardSignature>,
}

/// Information about the agent provider/organization
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentProvider {
    /// Name of the organization providing this agent
    pub organization: String,

    /// URL of the organization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Agent capability flags per A2A spec
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    /// Whether the agent supports streaming responses
    #[serde(default)]
    pub streaming: bool,

    /// Whether the agent supports push notifications
    #[serde(default)]
    pub push_notifications: bool,

    /// Whether the agent supports state/session persistence
    #[serde(default)]
    pub state_transition_history: bool,
}

/// A skill the agent can perform
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentSkill {
    /// Unique identifier for this skill
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Description of what this skill does
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Tags for categorizing the skill
    #[serde(default)]
    pub tags: Vec<String>,

    /// Example prompts that trigger this skill
    #[serde(default)]
    pub examples: Vec<String>,

    /// Input modes this skill accepts (overrides agent defaults)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub input_modes: Vec<String>,

    /// Output modes this skill produces (overrides agent defaults)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modes: Vec<String>,
}

/// Security scheme definition (OpenAPI-style)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SecurityScheme {
    /// Scheme identifier
    pub name: String,

    /// Type of security scheme
    #[serde(rename = "type")]
    pub scheme_type: SecuritySchemeType,

    /// Description of the scheme
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// OAuth2 flows (if type is oauth2)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flows: Option<serde_json::Value>,
}

/// Type of security scheme
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum SecuritySchemeType {
    /// OAuth2 authentication
    Oauth2,
    /// HTTP authentication (Bearer, Basic, etc.)
    Http,
    /// API key authentication
    ApiKey,
    /// OpenID Connect
    OpenIdConnect,
    /// Mutual TLS
    MutualTls,
}

/// Security requirement (which schemes are required)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SecurityRequirement {
    /// Scheme name to scopes mapping
    #[serde(flatten)]
    pub requirements: std::collections::HashMap<String, Vec<String>>,
}

/// Agent extension declaration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentExtension {
    /// Extension identifier URI
    pub uri: String,

    /// Extension-specific data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Signature for Agent Card verification (A2A v0.3+)
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct AgentCardSignature {
    /// Signature algorithm used
    pub algorithm: String,

    /// Base64-encoded signature
    pub value: String,

    /// Key ID for verification
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
}

/// Agent capability description (used in broadcasts)
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
    /// Tool execution result
    ToolResult {
        /// Tool call ID this result corresponds to
        tool_call_id: String,
        /// JSON content of the result
        content: String,
        /// Whether the tool execution resulted in an error
        is_error: bool,
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
    /// Internal metadata (routing decisions, quality feedback)
    Metadata {
        /// Metadata type (e.g. "model_selected", "quality_feedback")
        key: String,
        /// Structured metadata payload
        value: serde_json::Value,
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
    /// Message parts (must contain at least one)
    #[serde(deserialize_with = "deserialize_non_empty_parts")]
    pub parts: Vec<MessagePart>,

    /// Optional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

fn deserialize_non_empty_parts<'de, D>(deserializer: D) -> Result<Vec<MessagePart>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let parts = Vec::<MessagePart>::deserialize(deserializer)?;
    if parts.is_empty() {
        return Err(serde::de::Error::custom(
            "message must have at least one part",
        ));
    }
    Ok(parts)
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
    /// RLM Context Manifest - decomposed large context
    ContextManifest {
        /// Unique manifest identifier
        manifest_id: String,
        /// Summary of the decomposed context
        summary: String,
        /// Number of chunks in the manifest
        chunk_count: usize,
        /// Total tokens across all chunks
        total_tokens: usize,
        /// Chunk previews with hints for search
        #[serde(default)]
        chunk_previews: Vec<ChunkPreviewInfo>,
    },
}

/// Preview information for a chunk in an RLM manifest.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChunkPreviewInfo {
    /// Chunk index (0-based)
    pub index: usize,
    /// Token count for this chunk
    pub tokens: usize,
    /// Preview text (first ~100 chars)
    pub preview: String,
    /// Semantic hints/keywords for searching
    #[serde(default)]
    pub hints: Vec<String>,
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

/// Request to list recent HRM tasks from an agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskListRequest {
    /// Only return tasks updated after this timestamp (ISO 8601)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub since: Option<String>,
    /// Maximum number of tasks to return (default: 50)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Response containing a list of HRM tasks
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskListResponse {
    pub tasks: Vec<HrmTaskSummary>,
    pub total_count: u32,
}

/// Summary of an HRM task for the dashboard
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HrmTaskSummary {
    pub id: String,
    pub objective: String,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
    pub progress: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<HrmToolCallSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_text: Option<String>,
}

/// A tool call within an HRM task
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HrmToolCallSummary {
    pub name: String,
    pub success: bool,
    pub timestamp: String,
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

/// Response containing full agent capabilities for onboarding
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentCapabilitiesGetResponse {
    /// Unique agent identifier
    pub agent_id: String,

    /// Human-readable agent name
    pub name: String,

    /// Agent's declared purpose
    pub purpose: String,

    /// LLM model being used
    pub model: String,

    /// List of capabilities this agent provides
    pub capabilities: Vec<String>,

    /// MCP tools available to this agent
    pub mcp_tools: Vec<McpToolInfo>,

    /// Current load (0.0 to 1.0)
    pub load: f32,

    /// Whether the agent is accepting new tasks
    pub accepting_tasks: bool,

    /// Base64-encoded ECDSA P-256 public key for TDF encryption
    pub public_key: String,

    /// Agent version
    pub version: String,

    /// Supported interaction modes
    #[serde(default)]
    pub interaction_modes: Vec<InteractionMode>,
}

/// Information about an MCP tool
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct McpToolInfo {
    /// Tool name
    pub name: String,

    /// Tool description
    pub description: String,

    /// Tool server/provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server: Option<String>,

    /// Input schema for the tool
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
}

/// Request to specialize an agent with encrypted configuration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentSpecializeRequest {
    /// Orchestrator/requester agent ID
    pub requester_id: String,

    /// TDF-encrypted ConfigurationBundle (base64-encoded)
    pub encrypted_bundle: String,

    /// Task context for specialization
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_context: Option<String>,

    /// Session ID for tracking
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,

    /// Iroh blob ticket pointing at the TDF-wrapped bundle on the data
    /// plane. When set, the handler fetches the bundle bytes via the
    /// agent's Iroh node instead of decoding `encrypted_bundle`. Mesh
    /// shipping (WS-D) uses this; the inline base64 path remains for
    /// callers without an Iroh node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
}

/// Response to agent specialization request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AgentSpecializeResponse {
    /// Session ID for the specialization
    pub session_id: String,

    /// Whether specialization was accepted
    pub accepted: bool,

    /// Message with additional details
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,

    /// Capabilities activated by this specialization
    #[serde(default)]
    pub activated_capabilities: Vec<String>,
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
    /// Current SwarmKit kit YAML content. The `agent_config_*` RPC method
    /// names predate the S6 AGENTS.md→SwarmKit cutover and were kept for
    /// wire compatibility; the payload itself has been kit YAML since then.
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
    /// New SwarmKit kit YAML content (see [`AgentConfigGetResponse::content`])
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

/// Task offer from LocalAIAgent to Orchestrator when user triggers intent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskOffer {
    /// Unique intent identifier
    pub intent_id: String,
    /// Hints about what capabilities might be needed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub capabilities_hint: Option<Vec<String>>,
    /// Device capabilities available
    pub device_caps: DeviceCapabilities,
    /// The user's intent or goal
    pub intent: String,
    /// Optional context from the app
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

/// Device capabilities available on the local device
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeviceCapabilities {
    /// On-device AI capabilities
    pub ai_capabilities: Vec<AiCapability>,
    /// Available sensors
    pub sensors: Vec<SensorType>,
    /// Device platform
    pub platform: DevicePlatform,
    /// OS version
    pub os_version: String,
}

/// On-device AI capability
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AiCapability {
    /// Foundation Models (structured generation, tool calling)
    FoundationModels,
    /// Writing Tools (text refinement, proofreading)
    WritingTools,
    /// Image Playground (on-device image synthesis)
    ImagePlayground,
    /// Speech recognition
    SpeechRecognition,
    /// Text-to-speech
    TextToSpeech,
}

/// Device platform
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DevicePlatform {
    Ios,
    Macos,
    Tvos,
    Watchos,
}

/// Type of sensor available on device
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SensorType {
    /// GPS location
    Location,
    /// Camera
    Camera,
    /// Microphone
    Microphone,
    /// Motion sensors (accelerometer, gyroscope)
    Motion,
    /// Nearby devices (BLE, WiFi, mDNS)
    NearbyDevices,
    /// Compass
    Compass,
    /// Ambient light sensor
    AmbientLight,
    /// Barometer
    Barometer,
}

/// Request for sensor data from LocalAIAgent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SensorRequest {
    /// Task ID this sensor request is part of
    pub task_id: String,
    /// Type of sensor to access
    pub sensor: SensorType,
    /// Level of detail required
    pub scope: DataScope,
    /// How long to retain the data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retention: Option<u64>,
    /// Sampling rate in Hz (samples per second)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate: Option<f64>,
    /// Policy tag for audit trail
    pub policy_tag: String,
}

/// Level of detail for sensor data
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DataScope {
    /// Minimal data (e.g., city-level location)
    Minimal,
    /// Standard data (e.g., street-level location)
    Standard,
    /// Detailed data (e.g., precise GPS coordinates)
    Detailed,
}

/// Response with sensor data
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SensorResponse {
    /// Task ID this response is for
    pub task_id: String,
    /// The sensor data payload
    pub payload: serde_json::Value,
    /// List of redactions applied
    #[serde(default)]
    pub redactions: Vec<String>,
    /// Timestamp of sensor reading
    pub timestamp: DateTime<Utc>,
}

/// Tool call with locality specification
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolCall {
    /// Tool call ID for correlation
    pub tool_call_id: String,
    /// Name of the tool to invoke
    pub name: String,
    /// Arguments for the tool
    pub args: serde_json::Value,
    /// Where the tool should execute
    pub locality: Locality,
}

/// Where a tool should execute
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Locality {
    /// Execute on local device (on-device AI, sensors)
    Local,
    /// Execute remotely (cloud APIs, web services)
    Remote,
}

/// Tool call result
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ToolCallResult {
    /// Tool call ID this result is for
    pub tool_call_id: String,
    /// Whether the call succeeded
    pub success: bool,
    /// Result data if successful
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Request for human assistance
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HumanAssistRequest {
    /// Agent requesting human assistance
    pub agent_id: String,
    /// Reason for requesting human help
    pub reason: String,
    /// Context handle for the conversation
    pub context_handle: String,
    /// Optional suggested questions for the human
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_questions: Option<Vec<String>>,
}

/// Task result with artifacts and citations
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TaskResult {
    /// Task ID this result is for
    pub task_id: String,
    /// Result artifacts
    pub artifacts: Vec<Artifact>,
    /// Citations for sources used
    #[serde(default)]
    pub citations: Vec<Citation>,
    /// Policy tag for audit trail
    pub policy_tag: String,
    /// Timestamp of completion
    pub timestamp: DateTime<Utc>,
}

/// Result artifact
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Artifact {
    /// Artifact type
    pub artifact_type: ArtifactType,
    /// Artifact content or reference
    pub content: serde_json::Value,
    /// Optional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Type of artifact
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactType {
    /// Text content
    Text,
    /// Image content
    Image,
    /// Audio content
    Audio,
    /// Video content
    Video,
    /// Structured data
    Data,
    /// File reference
    File,
}

/// Citation for sources
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Citation {
    /// Source identifier
    pub source: String,
    /// Citation URL if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Timestamp of source
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<DateTime<Utc>>,
    /// Additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Policy binding for TDF key access
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KasPolicyBinding {
    /// HMAC algorithm (typically "HS256")
    pub alg: String,
    /// HMAC hash of policy bound to key
    pub hash: String,
}

/// Request to rewrap a TDF encryption key
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KasRewrapRequest {
    /// Base64-encoded wrapped key from the TDF manifest
    pub wrapped_key: String,
    /// Policy binding from the TDF manifest
    pub policy_binding: KasPolicyBinding,
    /// Base64-encoded policy JSON from the TDF manifest
    pub policy: String,
    /// NTDF delegation token chain (JSON)
    pub delegation_token: String,
    /// Client's public key in PEM format for rewrapping
    pub client_public_key: String,
}

/// Response containing the rewrapped key
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KasRewrapResponse {
    /// Key rewrapped for the client's public key
    pub entity_wrapped_key: String,
}

/// Request to retrieve the KAS public key
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct KasPublicKeyRequest {
    /// Requested algorithm (e.g., "RSA-OAEP")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub algorithm: Option<String>,
}

/// Response containing the KAS public key
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct KasPublicKeyResponse {
    /// PEM-encoded public key
    pub public_key: String,
    /// Key identifier
    pub key_id: String,
    /// Algorithm this key supports
    pub algorithm: String,
}

// TDF share types — canonical definitions live in arkavo-tdf::a2a_types
#[cfg(feature = "kas")]
pub use arkavo_tdf::{
    TdfOffer, TdfOffersRequest, TdfOffersResponse, TdfShareRequest, TdfShareResponse,
};

#[cfg(test)]
mod agent_specialize_request_tests {
    use super::AgentSpecializeRequest;

    #[test]
    fn deserializes_legacy_inline_request_without_ticket() {
        // Existing callers send no `ticket` — must still parse, ticket = None.
        let json = r#"{"requester_id":"did:web:orch","encrypted_bundle":"YmFzZTY0"}"#;
        let req: AgentSpecializeRequest = serde_json::from_str(json).expect("legacy parse");
        assert_eq!(req.encrypted_bundle, "YmFzZTY0");
        assert!(req.ticket.is_none());
    }

    #[test]
    fn deserializes_ticket_request() {
        let json = r#"{"requester_id":"did:web:orch","encrypted_bundle":"","ticket":"blobABC"}"#;
        let req: AgentSpecializeRequest = serde_json::from_str(json).expect("ticket parse");
        assert_eq!(req.ticket.as_deref(), Some("blobABC"));
    }

    #[test]
    fn ticket_is_omitted_from_wire_when_none() {
        let req = AgentSpecializeRequest {
            requester_id: "did:web:orch".into(),
            encrypted_bundle: "x".into(),
            task_context: None,
            session_id: None,
            ticket: None,
        };
        let wire = serde_json::to_string(&req).expect("serialize");
        assert!(
            !wire.contains("ticket"),
            "ticket must be skipped when None: {wire}"
        );
    }
}
