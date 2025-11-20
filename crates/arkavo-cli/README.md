# arkavo-cli

Command-line interface for Arkavo Edge agent.

## How It Works

### Tool Integration Flow

The CLI uses a sophisticated tool integration system that enables LLMs to discover and execute tools on-demand:

#### 1. Progressive Tool Disclosure

Rather than overwhelming the LLM with all available tools upfront, the system uses **progressive disclosure**:

- Tools are discovered based on keywords in the user's query
- Only relevant tools are exposed, reducing token usage by 95-98%
- The judge component (Gemma 270M/4B) detects when tools are needed but missing

#### 2. Iteration Model

The tool execution loop distinguishes between two types of operations:

**Metadata Operations (don't count as iterations):**
- Tool discovery via `MISSING_TOOL_USE` detection
- Tool metadata requests via `REQUEST_TOOL` protocol
- These are "free" operations that help the LLM find the right tools

**Work Operations (count as iterations):**
- Actual tool execution
- Final response generation
- Maximum: 10 iterations by default

#### 3. Quality Gate

Each request flows through the router's quality gate:

```
User Query
    ↓
Router (Model Selection)
    ↓
Judge (Quality Validation)
    ↓
Tool Discovery (if needed)
    ↓
Tool Execution (Iteration 1)
    ↓
Result Processing
    ↓
Final Response (Iteration 2)
```

#### 4. Example Flow

For the query "what time is it?":

```
Step 1: Tool Discovery (no iteration)
├─ Judge detects missing tool for "time"
├─ Search registry for ["time", "clock"]
├─ Find: get_agent_time, sync_agent_time
└─ Feed tool definitions to LLM

Step 2: Tool Execution (iteration 1)
├─ LLM: call get_agent_time()
├─ Print: → get_agent_time
├─ Execute and get result
└─ Feed result back to LLM

Step 3: Final Response (iteration 2)
├─ LLM generates natural language answer
└─ Return to user
```

**Total**: 2 iterations used out of 10 available

### Tool Output

The CLI provides concise, human-readable tool execution feedback:

```
→ get_agent_time
The current time is 2025-11-20T17:50:38 UTC.

✓ Executed 1 tool(s) across 2 iteration(s)
```

For verbose output, use `--show-tool-execution` to see full arguments and results.

### Architecture Components

**Router** (`arkavo-router`)
- Model selection and routing
- Cost optimization
- Quality gate enforcement
- Progressive tool disclosure

**Judge** (`arkavo-router::judge`)
- Response quality validation
- Missing tool detection via heuristics
- Keyword extraction for tool search

**Tool Registry** (`arkavo-mcp-tools`)
- Tool registration and discovery
- Keyword-based search
- MCP protocol integration

**Tool Executor** (`arkavo-llm`)
- Async tool execution
- Result formatting
- Error handling

### Configuration

Default configuration in `ToolIntegrationConfig`:

```rust
pub struct ToolIntegrationConfig {
    pub max_tool_iterations: usize,     // Default: 10
    pub show_tool_execution: bool,       // Default: false
}
```

Adjust limits based on workflow complexity:
- Simple queries: 2-3 iterations
- Multi-tool workflows: 4-8 iterations
- Complex automation: 10+ iterations (default handles most cases)
