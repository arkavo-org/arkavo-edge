use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::message::{Message, ToolCall};

/// Context entry supplied with a run request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Context {
    pub description: String,
    pub value: String,
}

/// Tool definition exposed to the agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tool {
    pub name: String,
    pub description: String,
    /// JSON Schema describing the tool's parameters.
    pub parameters: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// Lifecycle status of a resume entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResumeStatus {
    Resolved,
    Cancelled,
}

/// A previously resolved or cancelled interrupt carried forward in a run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResumeEntry {
    pub interrupt_id: String,
    pub status: ResumeStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

/// A human-in-the-loop interrupt surfaced in `RunFinished`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Interrupt {
    pub id: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_schema: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// Input payload for a single agent run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunAgentInput {
    pub thread_id: String,
    pub run_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_run_id: Option<String>,
    pub state: Value,
    pub messages: Vec<Message>,
    pub tools: Vec<Tool>,
    pub context: Vec<Context>,
    pub forwarded_props: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resume: Option<Vec<ResumeEntry>>,
}

/// Outcome variant carried by `RunFinished`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RunOutcome {
    Success,
    Interrupt { interrupts: Vec<Interrupt> },
}

/// Capability flags advertised by an agent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCapabilities {
    pub identity: bool,
    pub transport: TransportCapabilities,
    pub tools: ToolCapabilities,
    pub output: OutputCapabilities,
    pub state: StateCapabilities,
    pub multi_agent: bool,
    pub reasoning: bool,
    pub multimodal: bool,
    pub execution: bool,
    pub human_in_the_loop: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransportCapabilities {
    pub sse: bool,
    pub websocket: bool,
    pub binary: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCapabilities {
    pub enabled: bool,
    pub streaming: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputCapabilities {
    pub structured: bool,
    pub streaming: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateCapabilities {
    pub snapshots: bool,
    pub deltas: bool,
    pub memory: bool,
}

impl AgentCapabilities {
    /// Default capabilities for the Arkavo AG-UI gateway.
    pub const fn arkavo_default() -> Self {
        Self {
            identity: true,
            transport: TransportCapabilities {
                sse: true,
                websocket: true,
                binary: false,
            },
            tools: ToolCapabilities {
                enabled: true,
                streaming: true,
            },
            output: OutputCapabilities {
                structured: true,
                streaming: true,
            },
            state: StateCapabilities {
                snapshots: true,
                deltas: true,
                memory: true,
            },
            multi_agent: true,
            reasoning: true,
            multimodal: false,
            execution: false,
            human_in_the_loop: false,
        }
    }
}

impl RunAgentInput {
    /// Convenience constructor for tests and examples.
    pub fn new(thread_id: impl Into<String>, run_id: impl Into<String>) -> Self {
        Self {
            thread_id: thread_id.into(),
            run_id: run_id.into(),
            parent_run_id: None,
            state: Value::Null,
            messages: Vec::new(),
            tools: Vec::new(),
            context: Vec::new(),
            forwarded_props: Value::Null,
            resume: None,
        }
    }
}

impl Tool {
    /// Convenience constructor for tests and examples.
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: Value) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            parameters,
            metadata: None,
        }
    }
}

impl Message {
    /// Extract the `id` field regardless of role.
    pub fn id(&self) -> &str {
        match self {
            Self::Developer(m) => &m.id,
            Self::System(m) => &m.id,
            Self::Assistant(m) => &m.id,
            Self::User(m) => &m.id,
            Self::Tool(m) => &m.id,
            Self::Activity(m) => &m.id,
            Self::Reasoning(m) => &m.id,
        }
    }

    /// Return the tool calls on an `AssistantMessage`, if any.
    pub fn tool_calls(&self) -> Option<&[ToolCall]> {
        match self {
            Self::Assistant(m) => m.tool_calls.as_deref(),
            _ => None,
        }
    }
}
