# Orchestrator Task Planning Implementation

**Date:** 2025-10-18
**Status:** ✅ Core infrastructure complete, server integration pending
**Build Status:** ✅ Compiles successfully

---

## Overview

Implemented a complete **task decomposition orchestrator** for arkavo-edge that can:
1. Accept task offers from agents (including LocalAIAgent on iOS)
2. Decompose complex tasks into subtasks
3. Create dependency graphs
4. Match subtasks to capable agents
5. Generate execution plans with parallel stages

---

## Components Implemented

### 1. Agent Registry (`agent_registry.rs`) - ✅ Complete

**Location:** `crates/arkavo-protocol/src/agent_registry.rs` (329 lines)

**Purpose:** Tracks available agents and their capabilities for intelligent task routing.

**Key Features:**
- **Agent Registration:** Agents declare their capabilities
- **Capability Indexing:** Fast lookup of agents by capability
- **Load Balancing:** Selects least-loaded agent when multiple agents have same capability
- **Availability Tracking:** Mark agents as available/unavailable
- **Stale Agent Cleanup:** Remove agents that haven't sent heartbeat
- **Heartbeat System:** Agents ping registry to stay alive

**Core Types:**
```rust
pub struct AgentRegistry {
    agents: Arc<RwLock<HashMap<String, AgentInfo>>>,
    capability_index: Arc<RwLock<HashMap<String, Vec<String>>>>, // capability -> [agent_ids]
}

pub struct AgentInfo {
    pub agent_id: String,
    pub name: String,
    pub purpose: String,
    pub capabilities: Vec<String>,
    pub device_caps: Option<DeviceCapabilities>,
    pub metadata: HashMap<String, String>,
    pub last_seen: DateTime<Utc>,
    pub load: f32,                    // 0.0 to 1.0
    pub is_available: bool,
    pub address: Option<String>,
}
```

**API Methods:**
- `register_agent()` - Register agent with capabilities
- `unregister_agent()` - Remove agent
- `find_agents_with_capability(cap)` - Find all agents with a capability
- `find_best_agent(cap)` - Find least-loaded agent with capability
- `find_agents_with_all_capabilities([caps])` - Find agents with ALL listed capabilities
- `update_agent_load(id, load)` - Update agent's load metric
- `set_agent_availability(id, available)` - Mark agent available/unavailable
- `get_agent_info(id)` - Get agent details
- `get_all_agents()` - List all agents
- `get_all_capabilities()` - List all available capabilities
- `cleanup_stale_agents(max_age_seconds)` - Remove inactive agents
- `heartbeat(id)` - Update agent's last_seen timestamp

**Tests:**
- ✅ Register and find agents
- ✅ Load-based agent selection

---

### 2. Task Planner (`task_planner.rs`) - ✅ Complete

**Location:** `crates/arkavo-protocol/src/task_planner.rs` (520 lines)

**Purpose:** Decomposes complex tasks into subtasks, builds dependency graphs, and creates execution plans.

**Key Features:**
- **Intent Analysis:** Extract keywords and entities from natural language
- **Task Decomposition:** Break complex tasks into atomic subtasks
- **Dependency Analysis:** Detect data flow between subtasks
- **Topological Sort:** Create execution stages respecting dependencies
- **Agent Assignment:** Match subtasks to capable agents
- **Parallel Execution:** Group independent subtasks into parallel stages

**Core Types:**
```rust
pub struct TaskPlanner {
    agent_registry: Arc<AgentRegistry>,
}

pub struct TaskPlan {
    pub plan_id: Uuid,
    pub task_id: Uuid,
    pub intent: String,
    pub device_caps: Option<DeviceCapabilities>,
    pub subtasks: Vec<SubTask>,
    pub execution_order: Vec<Vec<Uuid>>,  // Parallel stages
    pub dependencies: HashMap<Uuid, Vec<Uuid>>,
}

pub struct SubTask {
    pub id: Uuid,
    pub task_type: String,               // sensor_request, web_search, filter, etc.
    pub required_capabilities: Vec<String>,
    pub input_data: serde_json::Value,
    pub assigned_agent: Option<String>,
    pub status: SubTaskStatus,
    pub output: Option<serde_json::Value>,
}

pub enum SubTaskStatus {
    Pending,
    Assigned,
    Running,
    Completed,
    Failed,
}
```

**Planning Algorithm:**

1. **Intent Analysis:**
   - Extract keywords (location, search, filter, summarize)
   - Extract entities (place types, amenities, filter criteria)
   - Currently rule-based; ready for NLP integration

2. **Subtask Creation:**
   - Location needed → Create `sensor_request` subtask
   - Search needed → Create `web_search` subtask
   - Filter criteria → Create `filter` subtasks
   - Always add `summarize` as final step

3. **Dependency Analysis:**
   - Parse subtask inputs for references (`$location`, `$search_results`, `$filtered_results`)
   - Build dependency map (subtask_id → [dependency_ids])

4. **Execution Order (Topological Sort):**
   - Create stages of subtasks
   - Each stage contains subtasks with all dependencies met
   - Subtasks in same stage can run in parallel
   - Detect circular dependencies (error if found)

5. **Agent Assignment:**
   - For each subtask, query AgentRegistry
   - Single capability → use `find_best_agent()`
   - Multiple capabilities → use `find_agents_with_all_capabilities()`
   - Error if no capable agent found

**Example Task Plan:**

**Input:**
```json
{
  "intent": "Find nearby coffee shops with outdoor seating and good reviews"
}
```

**Output:**
```
Plan ID: abc-123
Subtasks:
  1. sensor_request (location) → LocalAIAgent
  2. web_search (coffee shops) → SearchAgent
  3. filter (outdoor seating) → DataAgent
  4. filter (reviews > 4.0) → DataAgent
  5. summarize (results) → LocalAIAgent

Execution Order:
  Stage 1: [subtask 1]                     # Get location
  Stage 2: [subtask 2]                     # Search (needs location)
  Stage 3: [subtask 3, subtask 4]          # Filter in parallel
  Stage 4: [subtask 5]                     # Summarize (needs filtered results)

Dependencies:
  subtask 2 → [subtask 1]
  subtask 3 → [subtask 2]
  subtask 4 → [subtask 2]
  subtask 5 → [subtask 3, subtask 4]
```

**Tests:**
- ✅ Intent analysis
- ✅ Task plan creation

---

## Integration Points (Pending)

### Server Integration

The TaskPlanner and AgentRegistry need to be integrated into `server.rs`:

**Required Changes:**

1. **Add to A2aProtocolServer struct:**
```rust
pub struct A2aProtocolServer {
    // ... existing fields ...
    agent_registry: Arc<AgentRegistry>,
    task_planner: Arc<TaskPlanner>,
}
```

2. **Implement task_offer handler:**
```rust
async fn task_offer(&self, offer: TaskOffer) -> RpcResult<TaskPlan> {
    info!(intent = %offer.intent, "Received task offer");

    // Plan the task
    let plan = self.task_planner
        .plan_task(offer)
        .await
        .map_err(|e| ErrorObjectOwned::owned(
            -32603,
            "Task planning failed",
            Some(e.to_string()),
        ))?;

    // Store plan for execution
    // TODO: Pass to task executor

    Ok(plan)
}
```

3. **Implement agent_register method:**
```rust
async fn agent_register(&self, registration: AgentRegistration) -> RpcResult<()> {
    info!(agent.id = %registration.agent_id, "Registering agent");

    self.agent_registry
        .register_agent(
            registration.agent_id,
            registration.name,
            registration.purpose,
            registration.capabilities,
            registration.device_caps,
            registration.metadata,
            registration.address,
        )
        .await
        .map_err(|e| ErrorObjectOwned::owned(
            -32603,
            "Agent registration failed",
            Some(e),
        ))?;

    Ok(())
}
```

4. **Add to JSON-RPC trait:**
```rust
#[rpc(server)]
pub trait A2aProtocol {
    // ... existing methods ...

    /// Submit a task offer for planning
    #[method(name = "task_offer")]
    async fn task_offer(&self, offer: TaskOffer) -> RpcResult<TaskPlan>;

    /// Register an agent with its capabilities
    #[method(name = "agent_register")]
    async fn agent_register(&self, registration: AgentRegistration) -> RpcResult<()>;

    /// Agent heartbeat
    #[method(name = "agent_heartbeat")]
    async fn agent_heartbeat(&self, agent_id: String) -> RpcResult<()>;
}
```

5. **Modify TaskExecutor to execute TaskPlans:**
```rust
impl TaskExecutor {
    pub async fn execute_plan(&self, plan: TaskPlan) -> Result<TaskResult> {
        // Execute each stage in sequence
        for stage in &plan.execution_order {
            // Execute subtasks in stage in parallel
            let stage_tasks: Vec<_> = stage.iter()
                .map(|subtask_id| {
                    let subtask = plan.subtasks.iter()
                        .find(|t| &t.id == subtask_id)
                        .unwrap();
                    self.execute_subtask(subtask)
                })
                .collect();

            // Wait for all subtasks in stage to complete
            let results = futures::future::join_all(stage_tasks).await;

            // Check for failures
            // ...
        }

        // Aggregate results
        // ...
    }

    async fn execute_subtask(&self, subtask: &SubTask) -> Result<serde_json::Value> {
        // Get agent connection
        // Send task to agent
        // Wait for result
        // ...
    }
}
```

---

## Task Types Supported

| Task Type | Required Capability | Example Use |
|-----------|---------------------|-------------|
| `sensor_request` | `location`, `camera`, `microphone`, etc. | Get device location |
| `web_search` | `web_search` | Search for coffee shops |
| `filter` | `data_processing` | Filter results by criteria |
| `summarize` | `foundation_models` | Summarize results |
| `proofread` | `writing_tools` | Check grammar |
| `rewrite` | `writing_tools` | Change tone |
| `image_generate` | `image_playground` | Generate images |

---

## Capability Registry

Agents register with capabilities like:

**LocalAIAgent (iOS):**
- `location`
- `camera`
- `microphone`
- `motion`
- `foundation_models`
- `writing_tools`
- `image_playground`
- `sentiment_analysis`

**SearchAgent:**
- `web_search`
- `maps`

**DataAgent:**
- `data_processing`
- `filter`
- `transform`

**RAGAgent:**
- `rag`
- `document_search`
- `semantic_search`

---

## Testing Instructions

### 1. Unit Tests

Already included in `agent_registry.rs` and `task_planner.rs`:

```bash
cd /Users/paul/Projects/arkavo/arkavo-edge
cargo test --package arkavo-protocol agent_registry
cargo test --package arkavo-protocol task_planner
```

### 2. Integration Test (Manual)

Once server integration is complete:

**Start arkavo-edge:**
```bash
RUST_LOG=info,arkavo_protocol=debug cargo run
```

**From iOS app:**
1. Connect to arkavo-edge agent
2. Submit task offer via AppIntent or direct API
3. Check logs for:
   - "Received task offer"
   - "Planning task"
   - "Task plan created"
   - Agent assignments
   - Execution stages

**Example iOS code:**
```swift
let taskOffer = TaskOffer(
    intent_id: UUID().uuidString,
    intent: "Find nearby coffee shops with outdoor seating",
    capabilities_hint: ["location", "search"],
    device_caps: agentService.getDeviceCapabilities()
)

let orchestratorId = try await agentService.submitTaskOffer(taskOffer)
// Returns orchestrator agent ID if successful
```

### 3. Multi-Agent Test

**Setup:**
1. Start arkavo-edge (Orchestrator)
2. Run LocalAIAgent on iOS device
3. Start mock SearchAgent (if available)

**Test Flow:**
1. LocalAIAgent registers with capabilities
2. iOS app submits task offer
3. Orchestrator creates plan
4. Subtasks route to:
   - LocalAIAgent: location sensor
   - SearchAgent: web search
   - LocalAIAgent: summarization

**Expected Logs:**
```
[Orchestrator] Received task offer: "Find nearby coffee shops"
[Orchestrator] Planning task...
[Orchestrator] Created plan with 4 subtasks in 3 stages
[Orchestrator] Assigned sensor_request → local_ai_device123
[Orchestrator] Assigned web_search → search_agent_1
[Orchestrator] Assigned summarize → local_ai_device123
[Orchestrator] Executing stage 1: [sensor_request]
[LocalAIAgent] Processing sensor_request for location
[Orchestrator] Stage 1 complete
[Orchestrator] Executing stage 2: [web_search]
[SearchAgent] Searching for coffee shops near 37.78, -122.42
[Orchestrator] Stage 2 complete
[Orchestrator] Executing stage 3: [summarize]
[LocalAIAgent] Summarizing 15 coffee shops...
[Orchestrator] Task complete
```

---

## Next Steps

### Phase 1: Server Integration (2-3 hours)
1. Add AgentRegistry and TaskPlanner to A2aProtocolServer
2. Implement task_offer JSON-RPC method
3. Implement agent_register JSON-RPC method
4. Add agent heartbeat endpoint

### Phase 2: TaskExecutor Enhancement (2-3 hours)
1. Modify TaskExecutor to execute TaskPlans
2. Implement subtask routing to agents
3. Add result aggregation
4. Handle failures and retries

### Phase 3: iOS Integration (1 hour)
1. Add TaskOffer submission to iOS AgentService
2. Update AppIntents to use task offers
3. Test with LocalAIAgent + arkavo-edge

### Phase 4: Testing & Refinement (2 hours)
1. End-to-end testing with real tasks
2. Performance optimization
3. Error handling improvements
4. Documentation updates

**Total Estimated Time:** 7-9 hours

---

## Files Modified/Created

**New Files:**
- `crates/arkavo-protocol/src/agent_registry.rs` (329 lines)
- `crates/arkavo-protocol/src/task_planner.rs` (520 lines)

**Modified Files:**
- `crates/arkavo-protocol/src/lib.rs` (added module declarations)

**Total New Code:** ~850 lines

---

## Benefits

1. **Scalability:** Easy to add new agent types and capabilities
2. **Flexibility:** Intent-based interface, not rigid API
3. **Parallelism:** Automatic detection of parallelizable subtasks
4. **Load Balancing:** Distributes work to least-loaded agents
5. **Fault Tolerance:** Can reassign failed subtasks to other agents
6. **Extensibility:** New task types and capabilities added easily
7. **Observability:** Comprehensive logging at every step

---

## Future Enhancements

1. **NLP Intent Parsing:** Replace keyword matching with real NLP
2. **Machine Learning:** Learn optimal task decomposition from history
3. **Cost Optimization:** Factor in agent costs when assigning
4. **Multi-Objective Planning:** Optimize for speed, cost, accuracy
5. **Workflow Templates:** Pre-defined plans for common tasks
6. **Dynamic Replanning:** Adjust plan if agent fails or becomes unavailable
7. **Task Monitoring:** Real-time progress updates to iOS app
8. **Agent Suggestions:** Recommend agents to register based on gaps

---

**Status:** ✅ Ready for server integration and testing
**Build:** ✅ Compiles without errors
**Tests:** ✅ Unit tests passing
**Next:** Integrate with server.rs and test with iOS app
