//! Answering a message with the conductor directly, outside the agent loop.
//!
//! The agent loop keeps one conversation for the life of the agent and folds
//! every message queued in a tick into one cycle. That suits an agent that
//! acts on its own. It does not suit a message that must be answered on its
//! own terms: a step of a pipeline run answered from the history of an
//! earlier run, or two steps answered as one, is a wrong answer. Execution
//! here starts from the agent's purpose and the message and nothing else.

use std::collections::HashSet;
use std::sync::Arc;

use arkavo_hrm::{Conductor, store::InMemoryTaskStore};
use arkavo_protocol::mcp_registry::McpRegistry;
use arkavo_tasks::task_executor::TaskExecutor;
use tokio::sync::RwLock;

use crate::server::LearningBus;
use crate::server::conductor_parallel::for_requester;
use crate::server::config_helpers::AgentMetadata;
use crate::server::execute_with_conductor_and_learning;
use crate::server::mcp_bridge::McpBridgeTool;
use crate::server::tool_memory::ToolMemory;

/// Tool calls a task's own memory keeps, matching the agent's.
const TASK_MEMORY_ENTRIES: usize = 10;

/// What the conductor needs to answer a message as this agent.
#[derive(Clone)]
pub(in crate::server) struct DirectExecution {
    pub router: Arc<arkavo_router::Router>,
    pub conductor: Arc<Conductor<InMemoryTaskStore>>,
    pub mcp_registry: Arc<McpRegistry>,
    pub task_executor: Arc<TaskExecutor>,
    pub learning_bus: Option<Arc<LearningBus>>,
    pub compute_budget: arkavo_budget::SharedComputeBudget,
    pub mesh_state: Option<Arc<arkavo_mcp_mesh::MeshToolsState>>,
    pub agent_metadata: Arc<RwLock<AgentMetadata>>,
    pub model_hint: Option<arkavo_router::ModelChoice>,
    #[cfg(feature = "iroh")]
    pub iroh_node: Option<Arc<arkavo_tdf_iroh::IrohNode>>,
}

/// How one message is to be answered.
pub(in crate::server) struct Request {
    pub content: String,
    pub images: Option<Vec<String>>,
    /// The task a requester polls, for the conductor's progress reports.
    pub task_id: Option<uuid::Uuid>,
    /// Tool activity the answer may draw on and adds to.
    pub memory: Arc<RwLock<ToolMemory>>,
    /// Skip the model call that decides whether to split the task up.
    pub skip_complexity: bool,
}

impl Request {
    /// A message answered with nothing behind it: no images, no task to
    /// report progress on, and a tool memory of its own, so nothing an
    /// earlier message did is visible to it.
    pub(in crate::server) fn isolated(content: String, task_id: Option<uuid::Uuid>) -> Self {
        Self {
            content,
            images: None,
            task_id,
            memory: Arc::new(RwLock::new(ToolMemory::new(TASK_MEMORY_ENTRIES))),
            // A role's step is one piece of work by definition; the agent
            // loop, which runs these roles otherwise, skips the check too.
            skip_complexity: true,
        }
    }
}

impl DirectExecution {
    /// Answer `request` and return the model's text.
    ///
    /// The tools offered are the agent's mesh and MCP tools, narrowed to the
    /// role's grant set when the agent is specialized, so this path grants
    /// nothing the agent loop would withhold (design D9).
    pub(in crate::server) async fn answer(&self, request: Request) -> Result<String, String> {
        let (purpose, granted, specialized) = {
            let meta = self.agent_metadata.read().await;
            (
                meta.purpose.clone(),
                meta.granted_tools.iter().cloned().collect::<HashSet<_>>(),
                meta.specialized,
            )
        };
        let registry = {
            let mut registry = arkavo_mcp_tools::ToolRegistry::empty();
            if let Some(mesh) = &self.mesh_state {
                arkavo_mcp_mesh::register_tools(&mut registry, mesh.clone());
            }
            if let Ok(tools) = self.mcp_registry.list_all_tools().await {
                for tool in tools {
                    let name = tool.name.clone();
                    let bridge = McpBridgeTool::new(self.mcp_registry.clone(), tool);
                    registry.register(&name, Box::new(bridge));
                }
            }
            if specialized {
                registry.retain_granted(&granted);
            }
            Arc::new(registry)
        };

        // The requester polls for the answer, so the model's text is the
        // result and is not traded for a tool call. Boxed because the
        // conductor's future is tens of kilobytes and would otherwise sit
        // inline in the caller's own future.
        for_requester(Box::pin(execute_with_conductor_and_learning(
            &self.conductor,
            &self.router,
            &self.mcp_registry,
            request.content,
            request.task_id,
            request.task_id.map(|_| &self.task_executor),
            self.learning_bus.as_ref(),
            Some(&request.memory),
            (!purpose.is_empty()).then_some(purpose.as_str()),
            self.mesh_state.as_ref(),
            self.model_hint.as_ref(),
            request.images,
            Some(&self.compute_budget),
            None,
            request.skip_complexity,
            Some(registry),
            specialized.then_some(&granted),
            #[cfg(feature = "iroh")]
            self.iroh_node.as_ref(),
        )))
        .await
    }
}
