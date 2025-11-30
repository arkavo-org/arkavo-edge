# Mesh Tools Test Plan

Test plan for `arkavo-mesh-tools` crate - MCP tools for agent mesh orchestration.

## Prerequisites

- Arkavo Edge built with `mcp-tools` feature enabled
- Network access for mDNS discovery
- Multiple terminal sessions for multi-agent testing

## Unit Tests

Run existing unit tests:

```bash
cargo test -p arkavo-mesh-tools
```

Expected: All schema tests pass (4 tests)

## Integration Tests

### Test 1: Tool Registration

**Objective**: Verify mesh tools register correctly in the tool registry.

**Steps**:
1. Start arkavo chat or task command
2. Query available tools

```bash
cargo run -p arkavo -- chat --prompt "What tools do you have for agent coordination?"
```

**Expected**: Response mentions `list_agents`, `agent_query`, `send_task`, `get_task_status`

### Test 2: List Agents (Empty State)

**Objective**: Verify `list_agents` works with no agents running.

**Steps**:
1. Ensure no other arkavo agents are running
2. Execute list_agents tool

```bash
cargo run -p arkavo -- task "Use the list_agents tool to show all available agents"
```

**Expected**: Returns `{"success": true, "agent_count": 0, "agents": []}`

### Test 3: List Agents with Refresh

**Objective**: Verify mDNS discovery triggers correctly.

**Steps**:
1. Execute list_agents with refresh=true

```bash
cargo run -p arkavo -- task "Use list_agents with refresh=true to discover agents"
```

**Expected**: Tool attempts mDNS discovery on `_a2a._tcp.local.`, returns agent list (may be empty if no agents advertising)

### Test 4: Agent Query (No Match)

**Objective**: Verify agent_query handles no matches gracefully.

**Steps**:
1. Query for a non-existent capability

```bash
cargo run -p arkavo -- task "Use agent_query to find agents with capability 'quantum_computing'"
```

**Expected**: Returns `{"success": true, "match_count": 0, "agents": []}`

### Test 5: Agent Query (Validation)

**Objective**: Verify agent_query requires at least one filter.

**Steps**:
1. Call agent_query with no parameters

```bash
cargo run -p arkavo -- task "Use agent_query with no filters"
```

**Expected**: Returns error "At least one of 'capability' or 'purpose_contains' must be provided"

### Test 6: Send Task (Agent Not Found)

**Objective**: Verify send_task handles missing agent gracefully.

**Steps**:
1. Attempt to send task to non-existent agent

```bash
cargo run -p arkavo -- task "Use send_task to send 'hello' to agent_id 'nonexistent-agent'"
```

**Expected**: Returns error "Agent 'nonexistent-agent' not found. Try list_agents with refresh=true first."

### Test 7: Get Task Status (Agent Not Found)

**Objective**: Verify get_task_status handles missing agent gracefully.

**Steps**:
1. Attempt to get status for non-existent agent

```bash
cargo run -p arkavo -- task "Use get_task_status for agent_id 'fake-agent' and task_id 'task-123'"
```

**Expected**: Returns error "Agent 'fake-agent' not found"

## End-to-End Tests

### Test 8: Multi-Agent Discovery

**Objective**: Test mDNS discovery with multiple agents.

**Setup** (Terminal 1 - Agent A):
```bash
cargo run -p arkavo -- agent run examples/orchestrator-agent/security-agent.md
```

**Setup** (Terminal 2 - Agent B):
```bash
cargo run -p arkavo -- agent run examples/orchestrator-agent/testing-agent.md
```

**Test** (Terminal 3):
```bash
cargo run -p arkavo -- task "Use list_agents with refresh=true and show me what agents are available"
```

**Expected**: Both agents appear in the list with their capabilities

### Test 9: Task Delegation Flow

**Objective**: Test full task delegation workflow.

**Prerequisites**: At least one agent running (from Test 8)

**Steps**:
1. Discover agents
2. Query for specific capability
3. Send task to matching agent
4. Check task status

```bash
cargo run -p arkavo -- task "
1. Use list_agents to find available agents
2. Use agent_query to find an agent with security capabilities
3. Send a task to that agent asking it to review a code snippet
4. Get the task status to see if it completed
"
```

**Expected**: Full workflow completes with task_id returned

### Test 10: AG-UI Integration

**Objective**: Verify mesh tools work with AG-UI interface.

**Steps**:
1. Start UI
```bash
cargo run -p arkavo -- ui
```

2. Open browser to http://localhost:7700/chat
3. Use the agent grid to view discovered agents
4. Send a message through the chat interface

**Expected**: UI shows agent grid, chat works, discovered agents appear

## Load Testing

### Test 11: Concurrent Discovery

**Objective**: Verify concurrent mDNS discovery doesn't cause issues.

**Steps**:
```bash
# Run multiple discoveries in parallel
for i in {1..5}; do
  cargo run -p arkavo -- task "Use list_agents with refresh=true" &
done
wait
```

**Expected**: All complete without errors or deadlocks

### Test 12: Rapid Task Sends

**Objective**: Verify rapid task delegation doesn't overwhelm agents.

**Prerequisites**: One agent running

**Steps**:
```bash
for i in {1..10}; do
  cargo run -p arkavo -- task "Send a simple ping task to the first available agent" &
done
wait
```

**Expected**: All tasks accepted (some may queue)

## Error Handling Tests

### Test 13: Network Timeout

**Objective**: Verify graceful handling of unreachable agents.

**Steps**:
1. Register a fake agent address manually
2. Attempt to send task

**Expected**: Returns error after timeout (60s default)

### Test 14: Invalid JSON Response

**Objective**: Verify handling of malformed agent responses.

**Steps**: Mock an agent that returns invalid JSON

**Expected**: Returns parse error with details

## Performance Benchmarks

### Test 15: Discovery Latency

**Objective**: Measure mDNS discovery time.

**Steps**:
```bash
time cargo run -p arkavo -- task "Use list_agents with refresh=true" 2>&1 | grep -E "(real|agents)"
```

**Expected**: Discovery completes within 5 seconds

### Test 16: Tool Lookup Performance

**Objective**: Verify tool lookup is fast.

**Steps**: Instrument tool registry lookup time

**Expected**: Tool lookup < 1ms

## Regression Tests

Add test cases for any bugs found during testing:

| Bug ID | Description | Test Case |
|--------|-------------|-----------|
| - | - | - |

## Test Matrix

| Test | macOS arm64 | Linux x64 | Windows x64 |
|------|-------------|-----------|-------------|
| Unit Tests | Required | Required | Required |
| Tool Registration | Required | Required | N/A* |
| mDNS Discovery | Required | Required | N/A* |
| E2E Multi-Agent | Required | Optional | N/A* |

*Windows builds exclude mcp-tools feature by default

## Automation

Future: Add these tests to CI pipeline via:

```yaml
# .github/workflows/mesh-tools-tests.yml
jobs:
  test:
    steps:
      - run: cargo test -p arkavo-mesh-tools
      - run: cargo run -p arkavo -- task "Use list_agents to verify tool registration"
```
