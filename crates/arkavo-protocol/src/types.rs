use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Request to make a promise from another agent
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PromiseRequest {
    /// The ID of the requesting agent
    #[serde(rename = "agent_id")]
    #[schemars(example = "example_agent_id")]
    pub agent_id: String,

    /// The type of promise being requested
    #[serde(rename = "promise_type")]
    #[schemars(example = "example_promise_type")]
    pub promise_type: String,

    /// Additional data for the promise request
    #[serde(rename = "payload", skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
}

/// Response to a promise request
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PromiseResponse {
    /// Unique identifier for the promise
    #[serde(rename = "promise_id")]
    pub promise_id: Uuid,

    /// Current status of the promise
    pub status: PromiseStatus,

    /// Additional data in the response
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<serde_json::Value>,
}

/// Status of a promise
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum PromiseStatus {
    /// Promise has been accepted
    Accepted,
    /// Promise has been rejected
    Rejected,
    /// Promise is pending
    Pending,
    /// Promise has been fulfilled
    Fulfilled,
    /// Promise has been broken
    Broken,
}

/// Declaration of promises an agent can fulfill
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PromiseDeclareRequest {
    /// The ID of the declaring agent
    #[serde(rename = "agent_id")]
    pub agent_id: String,

    /// List of promises the agent can fulfill
    pub promises: Vec<PromiseCapability>,
}

/// A promise capability that an agent can fulfill
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PromiseCapability {
    /// Type of promise
    #[serde(rename = "type")]
    pub promise_type: String,

    /// Constraints on the promise
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<serde_json::Value>,

    /// Additional metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Response to a promise declaration
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PromiseDeclareResponse {
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
    /// Filter by promise types
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promise_types: Option<Vec<String>>,

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

    /// Promise types the agent supports
    #[serde(skip_serializing_if = "Option::is_none")]
    pub promises: Option<Vec<String>>,

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

// Helper function for examples
fn example_agent_id() -> &'static str {
    "550e8400-e29b-41d4-a716-446655440000"
}

fn example_promise_type() -> &'static str {
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
    /// Tool call delta
    ToolCall {
        /// Tool call ID
        tool_call_id: String,
        /// Delta content for the tool call
        delta: String,
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
