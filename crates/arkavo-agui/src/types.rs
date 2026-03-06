use arkavo_budget::config::{AgentBudget, BudgetAlert};
use arkavo_budget::tracker::{BudgetStatus, SpendingRecord};
use arkavo_budget::{BudgetConfig, TokenCost};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// AG-UI Event types as per specification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgUiEvent {
    // Outbound events (frontend → agent)
    Connect {
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "aguiVersion")]
        agui_version: String,
        #[serde(rename = "sinceEventId", skip_serializing_if = "Option::is_none")]
        since_event_id: Option<String>,
    },
    UserMessage {
        #[serde(rename = "agentId")]
        agent_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<Attachment>>,
    },
    ChatOpen {
        #[serde(rename = "agentId")]
        agent_id: String,
    },
    ChatClose {
        #[serde(rename = "agentId")]
        agent_id: String,
    },
    ToolResult {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        result: Value,
    },
    UiAction {
        action: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        params: Option<Value>,
    },

    // Inbound events (agent → frontend)
    StateSnapshot {
        state: Value,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    MessagesSnapshot {
        messages: Vec<Message>,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    MessageDelta {
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "messageId")]
        message_id: String,
        delta: MessageDeltaContent,
    },
    ToolCall {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        arguments: Value,
    },
    StateDelta {
        patch: Vec<JsonPatch>,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    TypingStart {
        #[serde(rename = "messageId")]
        message_id: String,
    },
    TypingStop {
        #[serde(rename = "messageId")]
        message_id: String,
    },

    // Lifecycle events
    #[serde(rename = "lifecycle.start")]
    LifecycleStart {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "lifecycle.end")]
    LifecycleEnd {
        reason: String,
    },
    #[serde(rename = "lifecycle.error")]
    LifecycleError {
        code: String,
        message: String,
    },

    // Error event
    Error {
        code: String,
        message: String,
    },

    // Configuration management events
    GetAgentConfig {
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "includeBackups", default)]
        include_backups: bool,
    },
    AgentConfigSnapshot {
        content: String,
        version: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        backups: Option<Vec<ConfigBackupInfo>>,
        writable: bool,
    },
    UpdateAgentConfig {
        #[serde(rename = "agentId")]
        agent_id: String,
        content: String,
        #[serde(rename = "expectedVersion", skip_serializing_if = "Option::is_none")]
        expected_version: Option<String>,
        #[serde(rename = "createBackup", default = "default_true")]
        create_backup: bool,
    },
    ConfigUpdateResult {
        success: bool,
        #[serde(rename = "newVersion", skip_serializing_if = "Option::is_none")]
        new_version: Option<String>,
        #[serde(rename = "backupPath", skip_serializing_if = "Option::is_none")]
        backup_path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ConfigErrorInfo>,
        #[serde(rename = "reloadRequired", default)]
        reload_required: bool,
    },
    ValidateAgentConfig {
        #[serde(rename = "agentId")]
        agent_id: String,
        content: String,
    },
    ConfigValidationResult {
        valid: bool,
        #[serde(default)]
        errors: Vec<String>,
        #[serde(default)]
        warnings: Vec<String>,
    },
    RestoreAgentConfig {
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "backupFilename")]
        backup_filename: String,
    },
    ConfigRestoreResult {
        success: bool,
        #[serde(rename = "newVersion", skip_serializing_if = "Option::is_none")]
        new_version: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ConfigErrorInfo>,
    },

    // Budget events
    BudgetStatusUpdate {
        #[serde(rename = "agentId")]
        agent_id: Option<String>,
        status: BudgetStatus,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    BudgetAlert {
        alert: BudgetAlert,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    SpendingRecorded {
        record: SpendingRecord,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    BudgetConfigUpdate {
        config: BudgetConfig,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    ModelSelected {
        #[serde(rename = "agentId")]
        agent_id: String,
        provider: String,
        model: String,
        #[serde(rename = "estimatedCost")]
        estimated_cost: TokenCost,
        reason: String,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    GetBudgetStatus {
        #[serde(rename = "agentId")]
        agent_id: Option<String>,
    },
    SetAgentBudget {
        #[serde(rename = "agentId")]
        agent_id: String,
        budget: AgentBudget,
    },
    ResetBudgetWindow {
        window: String,
    },

    // Cost orchestrator events
    GetCostMetrics {
        #[serde(rename = "timeRange")]
        time_range: String,
    },
    CostMetricsUpdate {
        metrics: crate::roi_metrics::CostMetrics,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    GetROIDashboard,
    ROIDashboardUpdate {
        dashboard: crate::roi_metrics::ROIDashboard,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    GetCostPrediction {
        tasks: Vec<String>,
    },
    CostPredictionUpdate {
        prediction: arkavo_router::WorkflowCostPrediction,
        #[serde(rename = "eventId")]
        event_id: String,
    },

    // UI Generation events
    SubmitPrompt {
        text: String,
    },
    Plan {
        parts: Vec<UiPlanPart>,
    },
    RequestStatus,
    StatusUpdate {
        system: SystemStatus,
        #[serde(rename = "mcpTools")]
        mcp_tools: McpToolsStatus,
        llms: Vec<LlmStatus>,
        health: HealthData,
        timestamp: String,
    },
    SystemNotification {
        message: String,
        severity: NotificationSeverity,
    },
    PartStream {
        #[serde(rename = "partId")]
        part_id: String,
        #[serde(rename = "chunkType")]
        chunk_type: String,
        content: String,
        done: bool,
    },
    ApplyPart {
        #[serde(rename = "partId")]
        part_id: String,
    },
    AppliedPart {
        #[serde(rename = "partId")]
        part_id: String,
        #[serde(rename = "versionId")]
        version_id: String,
    },
    CancelGeneration,
    Undo,
    Redo,
    UndoAvailable {
        #[serde(rename = "canUndo")]
        can_undo: bool,
        #[serde(rename = "canRedo")]
        can_redo: bool,
    },
    UserEdit {
        selector: String,
        action: String,
        before: String,
        after: String,
    },
    SaveSession {
        name: String,
    },

    // Mesh dashboard events
    RequestMeshStatus,
    MeshStatus {
        agents: Vec<MeshAgentInfo>,
        timestamp: String,
    },
    AgentDiscovered {
        #[serde(rename = "agentId")]
        agent_id: String,
        endpoint: String,
        purpose: String,
        model: String,
        timestamp: String,
    },
    AgentLost {
        #[serde(rename = "agentId")]
        agent_id: String,
        reason: String,
        timestamp: String,
    },
    #[serde(rename = "a2aMessage")]
    A2AMessage {
        #[serde(rename = "fromAgent")]
        from_agent: String,
        #[serde(rename = "toAgent")]
        to_agent: String,
        method: String,
        direction: String,
        timestamp: String,
    },
    TelemetryEvent {
        #[serde(rename = "eventType")]
        event_type: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        details: Value,
        timestamp: String,
    },

    // Security / TDF audit events
    GetSecurityStatus,
    SecurityStatusUpdate {
        #[serde(rename = "kasEnabled")]
        kas_enabled: bool,
        #[serde(rename = "kasUrl")]
        kas_url: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "keyId")]
        key_id: String,
        #[serde(rename = "encryptionAlgorithm")]
        encryption_algorithm: String,
        #[serde(rename = "auditCount")]
        audit_count: u64,
        #[serde(rename = "preflightEnabled")]
        preflight_enabled: bool,
        #[serde(rename = "preflightPolicies")]
        preflight_policies: u32,
        timestamp: String,
    },
    TdfAuditEvent {
        #[serde(rename = "messageIndex")]
        message_index: usize,
        model: String,
        #[serde(rename = "manifestVersion")]
        manifest_version: String,
        algorithm: String,
        #[serde(rename = "ciphertextBytes")]
        ciphertext_bytes: usize,
        #[serde(rename = "policyAttributes")]
        policy_attributes: Vec<String>,
        timestamp: String,
    },
    PolicyApplied {
        #[serde(rename = "policyId")]
        policy_id: String,
        action: String,
        target: String,
        #[serde(rename = "attributeCount")]
        attribute_count: usize,
        timestamp: String,
    },

    // Learning panel events
    RequestLearningStatus,
    LearningStatusUpdate {
        agents: Vec<AgentLearningInfo>,
        #[serde(rename = "routingHistory")]
        routing_history: Vec<RoutingRecord>,
        #[serde(
            rename = "qualityTrends",
            default,
            skip_serializing_if = "Vec::is_empty"
        )]
        quality_trends: Vec<QualityTrend>,
        #[serde(rename = "lessonCount", default)]
        lesson_count: usize,
        timestamp: String,
    },
    RoutingEvaluation {
        #[serde(rename = "taskId")]
        task_id: String,
        #[serde(rename = "taskDescription")]
        task_description: String,
        candidates: Vec<RoutingCandidate>,
        #[serde(rename = "selectedAgent")]
        selected_agent: String,
        #[serde(rename = "wasExploration")]
        was_exploration: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        category: Option<String>,
        timestamp: String,
    },
    RoutingOutcome {
        #[serde(rename = "taskId")]
        task_id: String,
        #[serde(rename = "agentId")]
        agent_id: String,
        success: bool,
        #[serde(rename = "qualityScore")]
        quality_score: f64,
        #[serde(rename = "qualityIssues")]
        quality_issues: Vec<String>,
        timestamp: String,
    },

    // Task management events
    RequestTaskList,
    TaskList {
        tasks: Vec<TaskInfo>,
        timestamp: String,
    },
    SubmitTask {
        description: String,
        #[serde(rename = "targetAgent", skip_serializing_if = "Option::is_none")]
        target_agent: Option<String>,
    },
    TaskSubmitted {
        #[serde(rename = "taskId")]
        task_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        description: Option<String>,
        status: String,
        timestamp: String,
    },
    TaskStatusChanged {
        #[serde(rename = "taskId")]
        task_id: String,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        progress: Option<f32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        metrics: Option<TaskMetrics>,
        timestamp: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskMetrics {
    #[serde(rename = "tokensGenerated")]
    pub tokens_generated: u32,
    #[serde(rename = "tokensPerSec")]
    pub tokens_per_sec: f64,
    #[serde(rename = "ttftMs")]
    pub ttft_ms: u64,
    #[serde(rename = "inferenceDurationMs")]
    pub inference_duration_ms: u64,
    #[serde(rename = "energyWh")]
    pub energy_wh: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiPlanPart {
    pub id: String,
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshAgentInfo {
    pub id: String,
    pub endpoint: String,
    pub purpose: String,
    pub model: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub id: String,
    pub description: String,
    pub status: String,
    #[serde(rename = "targetAgent", skip_serializing_if = "Option::is_none")]
    pub target_agent: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "completedAt", skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityTrend {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub category: String,
    pub scores: Vec<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingCandidate {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub score: f64,
    pub alpha: f64,
    #[serde(rename = "betaParam")]
    pub beta_param: f64,
    pub observations: u64,
    #[serde(rename = "successRate")]
    pub success_rate: f64,
    pub probationary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryStat {
    pub category: String,
    pub alpha: f64,
    #[serde(rename = "betaParam")]
    pub beta_param: f64,
    #[serde(rename = "expectedValue")]
    pub expected_value: f64,
    pub observations: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentLearningInfo {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub alpha: f64,
    #[serde(rename = "betaParam")]
    pub beta_param: f64,
    #[serde(rename = "expectedValue")]
    pub expected_value: f64,
    #[serde(rename = "stdDev")]
    pub std_dev: f64,
    #[serde(rename = "totalObservations")]
    pub total_observations: u64,
    #[serde(rename = "successRate")]
    pub success_rate: f64,
    pub probationary: bool,
    #[serde(
        rename = "categoryStats",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub category_stats: Vec<CategoryStat>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingRecord {
    #[serde(rename = "taskId")]
    pub task_id: String,
    #[serde(rename = "selectedAgent")]
    pub selected_agent: String,
    #[serde(rename = "wasExploration")]
    pub was_exploration: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(rename = "qualityScore", skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f64>,
    #[serde(
        rename = "qualityIssues",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub quality_issues: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub role: MessageRole,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, Value>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    #[serde(rename = "type")]
    pub attachment_type: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MessageDeltaContent {
    Text {
        text: String,
    },
    ToolCall {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(rename = "argsJsonFragment")]
        args_json_fragment: String,
        done: bool,
    },
    ToolResult {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        content: String,
        #[serde(rename = "isError")]
        is_error: bool,
    },
    /// Metadata (selectively forwarded to frontend, e.g. teaching_intent)
    Metadata {
        key: String,
        value: serde_json::Value,
    },
}

/// JSON Patch operation as per RFC 6902
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum JsonPatch {
    Add { path: String, value: Value },
    Remove { path: String },
    Replace { path: String, value: Value },
    Move { from: String, path: String },
    Copy { from: String, path: String },
    Test { path: String, value: Value },
}

/// Chat request payload for streaming chat
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// Handle for managing active subscriptions
#[derive(Debug)]
pub struct SubscriptionHandle {
    pub id: String,
    pub cancel_tx: Option<tokio::sync::oneshot::Sender<()>>,
}

impl SubscriptionHandle {
    pub fn new(id: String) -> (Self, tokio::sync::oneshot::Receiver<()>) {
        let (tx, rx) = tokio::sync::oneshot::channel();
        (
            Self {
                id,
                cancel_tx: Some(tx),
            },
            rx,
        )
    }

    pub fn cancel(&mut self) {
        if let Some(tx) = self.cancel_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl Drop for SubscriptionHandle {
    fn drop(&mut self) {
        self.cancel();
    }
}

/// Configuration backup information for AG-UI
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigBackupInfo {
    pub filename: String,
    pub timestamp: DateTime<Utc>,
    pub size: u64,
    pub version: String,
}

/// Configuration error information for AG-UI
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConfigErrorInfo {
    AgentOffline,
    ReadOnlyFilesystem,
    ValidationFailed { details: String },
    Conflict { current_version: String },
    Unauthorized,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemStatus {
    pub uptime: String,
    pub memory_usage: String,
    pub active_connections: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolsStatus {
    pub browser_available: bool,
    pub tools_count: usize,
    pub last_used: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmStatus {
    pub name: String,
    pub provider: String,
    pub connected: bool,
    pub model: String,
    #[serde(rename = "requestsToday")]
    pub requests_today: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthData {
    pub status: String,
    pub components: Vec<ComponentHealth>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentHealth {
    pub component: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationSeverity {
    Info,
    Warning,
    Error,
}
