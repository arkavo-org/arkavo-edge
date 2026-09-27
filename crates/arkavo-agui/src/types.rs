use arkavo_budget::config::{AgentBudget, BudgetAlert};
use arkavo_budget::tracker::{BudgetStatus, SpendingRecord};
use arkavo_budget::{BudgetConfig, TokenCost};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// Process start time for computing uptime
pub(crate) static PROCESS_START: std::sync::LazyLock<std::time::Instant> =
    std::sync::LazyLock::new(std::time::Instant::now);

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

    // Compute budget telemetry events (per-agent, polled from agents)
    ComputeBudgetUpdate {
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "computeBudget")]
        compute_budget: serde_json::Value,
        timestamp: String,
    },

    // Per-agent process metrics (self-reported by each agent binary)
    AgentSystemMetrics {
        #[serde(rename = "agentId")]
        agent_id: String,
        /// Resident set size in MB
        #[serde(rename = "rssMb")]
        rss_mb: f64,
        /// CPU usage percentage (0-100+, multi-core can exceed 100)
        #[serde(rename = "cpuPercent")]
        cpu_percent: f64,
        /// OS process ID
        pid: u32,
        /// Total system RAM in MB
        #[serde(rename = "totalRamMb", skip_serializing_if = "Option::is_none")]
        total_ram_mb: Option<f64>,
        /// Available (free) system RAM in MB
        #[serde(rename = "availableRamMb", skip_serializing_if = "Option::is_none")]
        available_ram_mb: Option<f64>,
    },

    // Per-agent task counts
    AgentTaskCounts {
        #[serde(rename = "agentId")]
        agent_id: String,
        pending: u32,
        active: u32,
        completed: u32,
        failed: u32,
    },

    /// Router telemetry: process-wide counts of structured `RouterEvent`s
    /// (e.g. `cloud_escalation_blocked`, `local_feasibility`,
    /// `spec_decoding_disabled`), read from the global event counters. Lets the
    /// UI surface how often local-first held and feasibility degraded.
    RouterTelemetry {
        counters: std::collections::HashMap<String, u64>,
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
        /// Per-agent security posture
        #[serde(default)]
        agents: Vec<AgentSecurityInfo>,
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

    // Data plane events (Iroh P2P + TDF share)
    GetDataPlaneStatus,
    DataPlaneStatusUpdate {
        #[serde(rename = "irohActive")]
        iroh_active: bool,
        #[serde(rename = "totalSharesSent")]
        total_shares_sent: u64,
        #[serde(rename = "totalSharesReceived")]
        total_shares_received: u64,
        #[serde(rename = "totalBytesStaged")]
        total_bytes_staged: u64,
        #[serde(rename = "totalBytesFetched")]
        total_bytes_fetched: u64,
        #[serde(rename = "pendingOffers")]
        pending_offers: u32,
        timestamp: String,
    },
    DataPlaneTransfer {
        direction: String,
        #[serde(rename = "peerAgentId")]
        peer_agent_id: String,
        #[serde(rename = "contentHash")]
        content_hash: String,
        #[serde(rename = "sizeBytes")]
        size_bytes: u64,
        status: String,
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
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        lessons: Vec<LessonInfo>,
        #[serde(
            rename = "optimalConfigs",
            default,
            skip_serializing_if = "Vec::is_empty"
        )]
        optimal_configs: Vec<OptimalConfigInfo>,
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

    LessonExtracted {
        #[serde(rename = "agentId")]
        agent_id: String,
        category: String,
        condition: String,
        action: String,
        confidence: f64,
        timestamp: String,
    },

    // Context Topology events
    RequestContextTopology {
        #[serde(rename = "selectedAgent")]
        #[serde(default)]
        selected_agent: Option<String>,
    },
    ContextTopologyUpdate {
        rlm: RlmSnapshot,
        #[serde(rename = "contextStrategies")]
        context_strategies: Vec<ContextStrategySnapshot>,
        #[serde(rename = "toolMemory")]
        tool_memory: ToolMemorySnapshot,
        #[serde(rename = "decisionTraces")]
        decision_traces: Vec<DecisionTraceSnapshot>,
        #[serde(rename = "antiPatterns")]
        anti_patterns: Vec<AntiPatternSnapshot>,
        #[serde(rename = "memoryLifecycle")]
        memory_lifecycle: MemoryLifecycleSnapshot,
        gossip: GossipSnapshot,
        agents: Vec<AgentContextInfo>,
        #[serde(rename = "conversationWindow")]
        #[serde(skip_serializing_if = "Option::is_none")]
        conversation_window: Option<serde_json::Value>,
        timestamp: String,
    },

    // Agent Runtime Policy (ARP) events
    RequestArpStatus,
    ArpStatusUpdate {
        snapshot: ArpStatusSnapshot,
        #[serde(rename = "eventId")]
        event_id: String,
    },
    /// Findings-inbox HITL decision on a tightening proposal.
    RequestArpProposalAction {
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "proposalId")]
        proposal_id: String,
        /// "approve" or "reject".
        action: String,
    },
    ArpProposalActionResult {
        #[serde(rename = "agentId")]
        agent_id: String,
        #[serde(rename = "proposalId")]
        proposal_id: String,
        /// `None` on success; the rejection reason otherwise.
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },

    // MCP-T Published Trust panel — read-only view of what the agent
    // publishes externally via `trust/query` and `trust/history`.
    // `agent_id` selects which connected agent's view to query; when None
    // the gateway falls back to the first live AgentConnection so the
    // initial render works before the user picks one.
    RequestPublishedTrust {
        #[serde(rename = "agentId", skip_serializing_if = "Option::is_none", default)]
        agent_id: Option<String>,
    },
    PublishedTrustUpdate {
        snapshot: PublishedTrustSnapshot,
        #[serde(rename = "eventId")]
        event_id: String,
    },

    /// Operator request to stop an active SwarmFlight. The gateway
    /// deregisters every role of the flight from the ArpHandler and
    /// drops the flight from the registry. Stopping a non-existent or
    /// already-stopped flight is a no-op (`FlightStopped` reports success).
    RequestStopFlight {
        #[serde(rename = "flightId")]
        flight_id: String,
    },
    FlightStopped {
        #[serde(rename = "flightId")]
        flight_id: String,
        /// `None` on success; `Some(message)` if the flight_id failed to
        /// parse as a UUID or some other server-side error occurred.
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
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
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        status: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        source: Option<String>,
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
    #[serde(rename = "tokensGenerated", default)]
    pub tokens_generated: u32,
    #[serde(rename = "tokensPerSec", default)]
    pub tokens_per_sec: f64,
    #[serde(rename = "ttftMs", default)]
    pub ttft_ms: u64,
    #[serde(rename = "inferenceDurationMs", default)]
    pub inference_duration_ms: u64,
    #[serde(rename = "energyWh", default)]
    pub energy_wh: f64,
    #[serde(rename = "qualityScore", skip_serializing_if = "Option::is_none")]
    pub quality_score: Option<f64>,
    /// Context window utilization (0-100%)
    #[serde(
        rename = "contextUtilizationPct",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_utilization_pct: Option<f64>,
    /// Prompt evaluation / prefill time (ms) — from llama.cpp timings
    #[serde(rename = "promptEvalMs", skip_serializing_if = "Option::is_none")]
    pub prompt_eval_ms: Option<u64>,
    /// Token generation time (ms) — from llama.cpp timings
    #[serde(rename = "generationMs", skip_serializing_if = "Option::is_none")]
    pub generation_ms: Option<u64>,
    /// KV cache hit rate (0-100%) — tokens served from cache
    #[serde(rename = "cacheHitPct", skip_serializing_if = "Option::is_none")]
    pub cache_hit_pct: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiPlanPart {
    pub id: String,
    pub name: String,
    pub description: String,
}

/// Per-agent security and data plane posture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSecurityInfo {
    pub id: String,
    #[serde(rename = "kasEnabled")]
    pub kas_enabled: bool,
    #[serde(rename = "keyId")]
    pub key_id: String,
    pub algorithm: String,
    #[serde(rename = "irohActive")]
    pub iroh_active: bool,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub status: String,
    #[serde(rename = "targetAgent", skip_serializing_if = "Option::is_none")]
    pub target_agent: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "completedAt", skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
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
pub struct OptimalConfigInfo {
    pub model: String,
    pub temperature: f32,
    #[serde(rename = "topP")]
    pub top_p: f32,
    #[serde(rename = "thinkingMode")]
    pub thinking_mode: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LessonInfo {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub category: String,
    pub condition: String,
    pub action: String,
    pub confidence: f64,
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

// Context Topology snapshot types

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RlmSnapshot {
    #[serde(rename = "manifestCount")]
    pub manifest_count: usize,
    #[serde(rename = "totalChunks")]
    pub total_chunks: usize,
    #[serde(rename = "totalTokens")]
    pub total_tokens: usize,
    #[serde(rename = "activationThreshold")]
    pub activation_threshold: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextStrategySnapshot {
    pub strategy: String,
    #[serde(rename = "completionRate")]
    pub completion_rate: f64,
    #[serde(rename = "loopAvoidanceRate")]
    pub loop_avoidance_rate: f64,
    #[serde(rename = "avgContextBytes")]
    pub avg_context_bytes: usize,
    #[serde(rename = "compositeScore")]
    pub composite_score: f64,
    #[serde(rename = "burstCount")]
    pub burst_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMemorySnapshot {
    #[serde(rename = "entryCount")]
    pub entry_count: usize,
    #[serde(rename = "maxEntries")]
    pub max_entries: usize,
    #[serde(rename = "errorCount")]
    pub error_count: usize,
    #[serde(rename = "duplicateCount")]
    pub duplicate_count: usize,
    #[serde(rename = "recentActionTypes")]
    pub recent_action_types: Vec<String>,
    #[serde(rename = "consecutiveSameType")]
    pub consecutive_same_type: usize,
    #[serde(rename = "hasObserveData")]
    pub has_observe_data: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionTraceSnapshot {
    #[serde(rename = "traceId")]
    pub trace_id: String,
    #[serde(rename = "agentId", default, skip_serializing_if = "String::is_empty")]
    pub agent_id: String,
    #[serde(rename = "taskCategory")]
    pub task_category: String,
    #[serde(rename = "selectedModel")]
    pub selected_model: String,
    #[serde(rename = "selectionReason")]
    pub selection_reason: String,
    #[serde(rename = "budgetUsagePct")]
    pub budget_usage_pct: f64,
    #[serde(rename = "feasibleCount")]
    pub feasible_count: usize,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AntiPatternSnapshot {
    pub model: String,
    pub category: String,
    #[serde(rename = "failureSignature")]
    pub failure_signature: String,
    #[serde(rename = "failureCount")]
    pub failure_count: u32,
    #[serde(rename = "decayedWeight")]
    pub decayed_weight: f64,
    #[serde(rename = "lastSeen")]
    pub last_seen: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryLifecycleSnapshot {
    pub promoted: usize,
    pub distilled: usize,
    pub expired: usize,
    pub demoted: usize,
    #[serde(rename = "transientTtlDays")]
    pub transient_ttl_days: u32,
    #[serde(rename = "promotionThreshold")]
    pub promotion_threshold: u32,
    #[serde(rename = "canonicalThreshold")]
    pub canonical_threshold: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipSnapshot {
    #[serde(rename = "eventsReceived")]
    pub events_received: u64,
    #[serde(rename = "episodesSynthesized")]
    pub episodes_synthesized: u64,
    #[serde(rename = "lessonsStored")]
    pub lessons_stored: u64,
    #[serde(rename = "gossipPeers")]
    pub gossip_peers: usize,
    #[serde(rename = "lastEventSecsAgo", skip_serializing_if = "Option::is_none")]
    pub last_event_secs_ago: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContextInfo {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(
        rename = "contextUtilizationPct",
        skip_serializing_if = "Option::is_none"
    )]
    pub context_utilization_pct: Option<f64>,
    pub alpha: f64,
    #[serde(rename = "betaParam")]
    pub beta_param: f64,
    #[serde(rename = "expectedValue")]
    pub expected_value: f64,
    #[serde(rename = "totalObservations")]
    pub total_observations: u64,
    #[serde(
        rename = "categoryStats",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub category_stats: Vec<CategoryStat>,
}

/// Mesh-wide ARP snapshot — one entry per agent.
///
/// Each agent in the mesh owns its own Agent Runtime Policy, so the panel
/// is keyed by agent. The gateway includes its locally-loaded document as
/// a synthetic `(local)` agent until per-agent A2A polling is wired.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpStatusSnapshot {
    pub agents: Vec<AgentArpStatus>,
    pub timestamp: String,
    /// Most-recent boot-time SwarmKit auto-launch failures, surfaced
    /// for the operator. Empty in steady state. Populated when
    /// `ARKAVO_SWARMKIT_PATH` is set but loading the manifest or
    /// launching the flight failed.
    #[serde(rename = "swarmkitLaunchErrors", default)]
    pub swarmkit_launch_errors: Vec<SwarmkitLaunchErrorEntry>,
}

/// Read-only snapshot of every MCP-T trust score this agent publishes.
///
/// Each agent acts as its own provider — `provider_did` is the persistent
/// device key that signs all scores. `self_score` is what the agent
/// publishes about itself; `peers` are scores it publishes about other
/// agents it has observed (one entry per peer, signed by the same key).
/// `recent_traces` spans every subject so the panel can show the most
/// recent N tool-execution traces across the swarm at a glance.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedTrustSnapshot {
    pub provider_did: String,
    pub self_subject_id: String,
    pub self_score: Option<TrustScoreView>,
    pub peers: Vec<TrustScoreView>,
    pub recent_traces: Vec<BehaviorTraceView>,
    pub fetched_at: String,
}

/// Display projection of a single MCP-T `TrustScore`. Strips the signature
/// and JCS bookkeeping the panel doesn't render.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustScoreView {
    pub subject_id: String,
    pub composite: u32,
    pub dimensions: Vec<DimensionView>,
    pub validity_expires_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DimensionView {
    pub name: String,
    pub value: u32,
    pub confidence: f64,
    pub evidence_count: u32,
}

/// Display projection of a `behavior.trace` event payload — just the fields
/// the panel renders (subject, contract, fidelity ratio, counts, time).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorTraceView {
    pub trace_id: String,
    pub contract_id: String,
    pub subject_id: String,
    pub timestamp: String,
    pub fidelity_ratio: f64,
    pub total_tool_calls: u32,
    pub undeclared_tool_calls: u32,
}

/// Per-agent ARP state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentArpStatus {
    #[serde(rename = "agentId")]
    pub agent_id: String,
    /// Path the ARP document was loaded from (None if no document configured).
    #[serde(rename = "documentPath", skip_serializing_if = "Option::is_none")]
    pub document_path: Option<String>,
    /// Whether the loaded document parsed and validated successfully.
    #[serde(rename = "documentValid")]
    pub document_valid: bool,
    /// Parse / validation error if loading failed.
    #[serde(rename = "loadError", skip_serializing_if = "Option::is_none")]
    pub load_error: Option<String>,
    /// Document summary (None if no document loaded).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document: Option<ArpDocumentSummary>,
    /// Runtime PolicyCache state (None if cache not instantiated).
    #[serde(rename = "policyCache", skip_serializing_if = "Option::is_none")]
    pub policy_cache: Option<ArpPolicyCacheSnapshot>,
    /// Adaptation engine state (None if engine not instantiated).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adaptation: Option<ArpAdaptationSnapshot>,
    /// Tightening proposals (findings inbox).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub proposals: Vec<ArpProposalSnapshot>,
    /// Most recent decision-trace entries from arkavo-observability.
    #[serde(
        rename = "decisionTraces",
        default,
        skip_serializing_if = "Vec::is_empty"
    )]
    pub decision_traces: Vec<ArpDecisionTraceEntry>,
    /// Set when this agent entry represents a role within a SwarmFlight.
    /// The UI uses this to label the entry with kit + role metadata
    /// rather than the synthetic agent_id used for storage.
    #[serde(rename = "flightContext", skip_serializing_if = "Option::is_none")]
    pub flight_context: Option<FlightContext>,
}

/// Identifies a SwarmFlight role surfaced through the AG-UI ARP panel.
///
/// SwarmKit's `agent_provisioning` block hands off to the companion ARP
/// specification at SwarmFlight start. When the gateway registers a flight,
/// each role becomes addressable in the panel's agent dropdown; this struct
/// carries enough metadata for the UI to render the role meaningfully
/// instead of showing the synthetic per-role agent_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlightContext {
    #[serde(rename = "flightId")]
    pub flight_id: String,
    #[serde(rename = "kitId")]
    pub kit_id: String,
    #[serde(rename = "kitName")]
    pub kit_name: String,
    #[serde(rename = "roleId")]
    pub role_id: String,
    #[serde(rename = "roleType")]
    pub role_type: String,
    /// DID of the mesh agent the orchestrator bound to this role.
    /// `None` for in-process / unbound flights.
    #[serde(rename = "boundAgentDid", skip_serializing_if = "Option::is_none")]
    pub bound_agent_did: Option<String>,
}

/// Single auto-launch failure surfaced to the operator. The gateway
/// records one of these into [`SwarmFlightRegistry`] when an
/// `ARKAVO_SWARMKIT_PATH` boot launch fails, so the UI can render the
/// failure instead of leaving the operator to grep logs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwarmkitLaunchErrorEntry {
    pub path: String,
    /// Discriminator from `AutoLaunchError`: one of `read`, `parse`,
    /// `launch`, `tdf_unwrap`, `tdf_feature_disabled`.
    pub kind: String,
    pub message: String,
    #[serde(rename = "occurredAt")]
    pub occurred_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpDocumentSummary {
    #[serde(rename = "arpSpec")]
    pub arp_spec: String,
    #[serde(rename = "adlUri", skip_serializing_if = "Option::is_none")]
    pub adl_uri: Option<String>,
    #[serde(rename = "adlHash", skip_serializing_if = "Option::is_none")]
    pub adl_hash: Option<String>,
    #[serde(rename = "integritySigned")]
    pub integrity_signed: bool,
    #[serde(rename = "adaptationMethod")]
    pub adaptation_method: String,
    #[serde(rename = "qualityGate", skip_serializing_if = "Option::is_none")]
    pub quality_gate: Option<ArpQualityGateSummary>,
    #[serde(rename = "policyCacheConfig", skip_serializing_if = "Option::is_none")]
    pub policy_cache_config: Option<ArpPolicyCacheConfigSummary>,
    pub budget: ArpBudgetSummary,
    #[serde(rename = "sectionsPresent")]
    pub sections_present: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpQualityGateSummary {
    pub threshold: f64,
    pub metric: String,
    #[serde(rename = "onFailure")]
    pub on_failure: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpPolicyCacheConfigSummary {
    #[serde(rename = "ttlSec")]
    pub ttl_sec: u64,
    #[serde(rename = "decayStrategy")]
    pub decay_strategy: String,
    #[serde(rename = "halfLifeSec", skip_serializing_if = "Option::is_none")]
    pub half_life_sec: Option<u64>,
    #[serde(rename = "humanExempt")]
    pub human_exempt: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpBudgetSummary {
    #[serde(rename = "taskCeilingUsd")]
    pub task_ceiling_usd: f64,
    #[serde(rename = "onExhaustion")]
    pub on_exhaustion: String,
    #[serde(
        rename = "maxSpendPerMinuteUsd",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_spend_per_minute_usd: Option<f64>,
    #[serde(
        rename = "maxToolCallsPerMinute",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_tool_calls_per_minute: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpPolicyCacheSnapshot {
    #[serde(rename = "entryCount")]
    pub entry_count: usize,
    #[serde(rename = "chainValid")]
    pub chain_valid: bool,
    pub entries: Vec<ArpPolicyCacheEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpPolicyCacheEntry {
    pub key: String,
    pub source: String,
    pub influence: f64,
    #[serde(rename = "ageSec")]
    pub age_sec: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpAdaptationSnapshot {
    pub method: String,
    pub entities: Vec<ArpAdaptationEntity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpAdaptationEntity {
    pub id: String,
    pub alpha: f64,
    pub beta: f64,
    pub mean: f64,
    pub observations: u32,
    #[serde(rename = "inWarmup")]
    pub in_warmup: bool,
    /// Live component of the effective prior (prior provenance).
    #[serde(rename = "liveAlpha")]
    pub live_alpha: f64,
    #[serde(rename = "liveBeta")]
    pub live_beta: f64,
    /// Distilled (teacher-provided) mass; absent when none was seeded.
    #[serde(rename = "distilledAlpha", skip_serializing_if = "Option::is_none")]
    pub distilled_alpha: Option<f64>,
    #[serde(rename = "distilledBeta", skip_serializing_if = "Option::is_none")]
    pub distilled_beta: Option<f64>,
    /// Current displacement weight applied to distilled mass.
    #[serde(rename = "distilledWeight", skip_serializing_if = "Option::is_none")]
    pub distilled_weight: Option<f64>,
}

/// One tightening proposal rendered as a findings-inbox card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpProposalSnapshot {
    pub id: String,
    pub origin: String,
    pub state: String,
    #[serde(rename = "effectKind")]
    pub effect_kind: String,
    #[serde(rename = "effectSummary")]
    pub effect_summary: String,
    pub rationale: String,
    #[serde(rename = "blastRadius")]
    pub blast_radius: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<ArpTraceRef>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "appliedAt", skip_serializing_if = "Option::is_none")]
    pub applied_at: Option<String>,
    #[serde(rename = "reviewedBy", skip_serializing_if = "Option::is_none")]
    pub reviewed_by: Option<String>,
    #[serde(rename = "dispositionReason", skip_serializing_if = "Option::is_none")]
    pub disposition_reason: Option<String>,
}

/// Evidence back-link on a proposal card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpTraceRef {
    #[serde(rename = "traceId")]
    pub trace_id: String,
    #[serde(rename = "episodeIds", default, skip_serializing_if = "Vec::is_empty")]
    pub episode_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArpDecisionTraceEntry {
    #[serde(rename = "traceId")]
    pub trace_id: String,
    pub layer: String,
    #[serde(rename = "eventType")]
    pub event_type: String,
    #[serde(rename = "agentId")]
    pub agent_id: String,
    #[serde(rename = "taskId")]
    pub task_id: String,
    #[serde(rename = "chosenEntity", skip_serializing_if = "Option::is_none")]
    pub chosen_entity: Option<String>,
    #[serde(rename = "selectionMethod", skip_serializing_if = "Option::is_none")]
    pub selection_method: Option<String>,
    /// Outcome.success from the trace; `false` indicates an attempted
    /// policy violation that was blocked or denied.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<bool>,
    /// Free-form denial reason or error type, when present.
    #[serde(rename = "deniedReason", skip_serializing_if = "Option::is_none")]
    pub denied_reason: Option<String>,
    pub timestamp: String,
}
