# Context Topology Matrix

The Context Topology Matrix is a unified dashboard in the Arkavo Edge AG-UI that visualizes all 10 context handling mechanisms for long-running autonomous learning agents. It provides glass-box observability into how agents manage context windows, learn from experience, and share knowledge across the mesh.

## Accessing the Tab

Click the hexagon icon in the AG-UI sidebar, or navigate programmatically:

```javascript
switchView('context');
```

The tab polls the backend every 10 seconds while active via `RequestContextTopology` / `ContextTopologyUpdate` WebSocket events.

## Spatial Layout

The dashboard uses a CSS Grid with 5 spatial zones that map to the context data flow:

```
+------------------+------------------+
|    STRATEGY SWEEP | RLM DECOMP      |  <- Top: Ingestion & Strategy
+--------+---------+---------+--------+
| TOOL   | AGENT MESH        | GOSSIP |
| MEMORY | (center)          | NETWORK|  <- Middle: Working Memory + Core + Swarm
| MEMORY | Agent nodes with  | LIFE-  |
| LIFE-  | ring gauges &     | CYCLE  |
| CYCLE  | radar charts      | TRANS  |
+--------+---------+---------+--------+
| DECISION TRACES   | ANTI-PATTERN    |  <- Bottom: Safety & Tracing
|                   | SHIELD          |
+-------------------+-----------------+
```

## Zone Details

### Top Zone — Ingestion and Strategy

**Strategy Sweep** (Mechanism 3): Five horizontal bars showing Thompson Sampling composite scores for each context strategy variant:
- Full, SummaryOnly, LastNMessages, ArtifactReference, Ledger
- The winning strategy glows with an active highlight
- Score = completion_rate x loop_avoidance_rate x quality x overhead_penalty

**RLM Decomposition** (Mechanism 1): Stat cards showing:
- Manifest count (active decomposed contexts)
- Total chunks across all manifests
- Total tokens under management
- Activation threshold (default 70% of model context window)

### Left Zone — Short-Term Memory

**Tool Memory** (Mechanism 4): Sliding window of recent tool actions:
- Entry count / max entries (default 10)
- Error and duplicate counts
- Recent action types as a scrolling list
- Consecutive same-type counter (warns at 3+)
- "Observe data cached" badge when observation data is held

**Memory Lifecycle** (Mechanism 10): Three-tier funnel visualization:
- Transient (gray) — new knowledge, expires after TTL days
- Candidate (blue) — promoted after N successes (default 3)
- Canonical (green) — confirmed after M confirmations (default 10)
- Config thresholds displayed above the funnel

### Center Zone — Agent Mesh

**Multi-LLM Mesh** (Mechanisms 6, 7): Each connected agent renders as a node with:
- Ring gauge showing context window utilization (0-100%)
  - Blue < 60%, Yellow 60-85%, Red > 85%
- Agent name and model identifier
- Expected value (EV) and observation count from Thompson Sampling
- Radar chart showing per-category Beta prior expected values (when 3+ categories exist)

### Right Zone — Long-Term Learning and Swarm

**Gossip Network** (Mechanism 8): Animated pulse wave showing:
- Peer count (agents sharing via gossip protocol)
- Total events received
- Episodes synthesized and lessons stored
- Time since last gossip event

**Lifecycle Transitions**: Aggregate counts of tier transitions:
- Promoted (transient -> candidate)
- Expired (transient TTL exceeded)
- Distilled (candidate -> canonical)
- Demoted (canonical failure rate > 50%)

### Bottom Zone — Safety and Tracing

**Decision Traces** (Mechanism 5): Circuit-board SVG showing recent routing decisions:
- Task category node (color-coded by selection reason)
- Selection reason (ThompsonSampling, BudgetConstrained, SingleFeasible, Fallback)
- Selected model node
- Connected by animated flow pulse lines

**Anti-Pattern Shield** (Mechanism 9): Firewall-style list of failure signatures:
- Signature name (e.g., `hallucinated_tool:read_file`)
- Failure count
- Decayed weight bar (24-hour half-life exponential decay)
- Model and category metadata
- Opacity fades with decay — stale patterns visually recede

## Data Flow

```
RequestContextTopology (frontend → gateway)
  ↓
gateway_context.rs aggregates:
  - LearningModule → Thompson priors, category stats
  - Agent JSON-RPC "context/topology" → ToolMemory, AntiPatterns, RLM, Gossip
  - Agent JSON-RPC "learning/status" → fallback for lesson counts, peer counts
  ↓
ContextTopologyUpdate (gateway → frontend)
  ↓
context.js renders 5 zones with context-charts.js SVG helpers
```

## Event Schema

Request (frontend → backend):

```json
{ "type": "requestContextTopology" }
```

Response (backend → frontend):

```json
{
  "type": "contextTopologyUpdate",
  "rlm": {
    "manifestCount": 3,
    "totalChunks": 24,
    "totalTokens": 12800,
    "activationThreshold": 0.7
  },
  "contextStrategies": [
    {
      "strategy": "Full",
      "completionRate": 0.95,
      "loopAvoidanceRate": 0.8,
      "avgContextBytes": 8200,
      "compositeScore": 0.72,
      "burstCount": 15
    }
  ],
  "toolMemory": {
    "entryCount": 7,
    "maxEntries": 10,
    "errorCount": 2,
    "duplicateCount": 1,
    "recentActionTypes": ["code_search", "file_read"],
    "consecutiveSameType": 0,
    "hasObserveData": true
  },
  "decisionTraces": [
    {
      "traceId": "a1b2c3",
      "taskCategory": "code_generation",
      "selectedModel": "qwen3.5-9b",
      "selectionReason": "ThompsonSampling",
      "budgetUsagePct": 45.2,
      "feasibleCount": 3,
      "timestamp": "2026-03-23T21:30:00Z"
    }
  ],
  "antiPatterns": [
    {
      "model": "qwen3.5-9b",
      "category": "code_generation",
      "failureSignature": "hallucinated_tool:read_file",
      "failureCount": 5,
      "decayedWeight": 0.75,
      "lastSeen": "2026-03-23T21:00:00Z"
    }
  ],
  "memoryLifecycle": {
    "promoted": 8,
    "distilled": 3,
    "expired": 2,
    "demoted": 1,
    "transientTtlDays": 7,
    "promotionThreshold": 3,
    "canonicalThreshold": 10
  },
  "gossip": {
    "eventsReceived": 142,
    "episodesSynthesized": 18,
    "lessonsStored": 7,
    "gossipPeers": 4,
    "lastEventSecsAgo": 12
  },
  "agents": [
    {
      "agentId": "code-specialist",
      "model": "qwen3.5-9b",
      "contextUtilizationPct": 67,
      "alpha": 8.5,
      "betaParam": 2.1,
      "expectedValue": 0.80,
      "totalObservations": 42,
      "categoryStats": [
        { "category": "code", "alpha": 6, "betaParam": 1.5, "expectedValue": 0.8, "observations": 20 }
      ]
    }
  ],
  "timestamp": "2026-03-23T21:30:00Z"
}
```

## Mock Data for Development

Inject this into the browser console to populate all zones without running agents:

```javascript
handleContextTopologyUpdate({
    type: 'contextTopologyUpdate',
    rlm: {
        manifestCount: 3,
        totalChunks: 24,
        totalTokens: 12800,
        activationThreshold: 0.7
    },
    contextStrategies: [
        { strategy: 'Full', completionRate: 0.95, loopAvoidanceRate: 0.8, avgContextBytes: 8200, compositeScore: 0.72, burstCount: 15 },
        { strategy: 'SummaryOnly', completionRate: 0.7, loopAvoidanceRate: 0.9, avgContextBytes: 1200, compositeScore: 0.58, burstCount: 8 },
        { strategy: 'LastNMessages', completionRate: 0.88, loopAvoidanceRate: 0.85, avgContextBytes: 3400, compositeScore: 0.68, burstCount: 22 },
        { strategy: 'ArtifactRef', completionRate: 0.6, loopAvoidanceRate: 0.95, avgContextBytes: 800, compositeScore: 0.45, burstCount: 3 },
        { strategy: 'Ledger', completionRate: 0.82, loopAvoidanceRate: 0.88, avgContextBytes: 2100, compositeScore: 0.63, burstCount: 11 }
    ],
    toolMemory: {
        entryCount: 7,
        maxEntries: 10,
        errorCount: 2,
        duplicateCount: 1,
        recentActionTypes: ['code_search', 'file_read', 'code_search', 'build_check'],
        consecutiveSameType: 0,
        hasObserveData: true
    },
    decisionTraces: [
        { traceId: 'a1b2c3', taskCategory: 'code_generation', selectedModel: 'qwen3.5-9b', selectionReason: 'ThompsonSampling', budgetUsagePct: 45.2, feasibleCount: 3, timestamp: '2026-03-23T21:30:00Z' },
        { traceId: 'd4e5f6', taskCategory: 'analysis', selectedModel: 'ministral-3b', selectionReason: 'BudgetConstrained', budgetUsagePct: 82.1, feasibleCount: 2, timestamp: '2026-03-23T21:29:00Z' },
        { traceId: 'g7h8i9', taskCategory: 'summarization', selectedModel: 'qwen3.5-0.8b', selectionReason: 'ThompsonSampling', budgetUsagePct: 12.5, feasibleCount: 4, timestamp: '2026-03-23T21:28:00Z' }
    ],
    antiPatterns: [
        { model: 'qwen3.5-9b', category: 'code_generation', failureSignature: 'hallucinated_tool:read_file', failureCount: 5, decayedWeight: 0.75, lastSeen: '2026-03-23T21:00:00Z' },
        { model: 'ministral-3b', category: 'analysis', failureSignature: 'timeout:inference', failureCount: 2, decayedWeight: 0.35, lastSeen: '2026-03-23T20:30:00Z' }
    ],
    memoryLifecycle: {
        promoted: 8,
        distilled: 3,
        expired: 2,
        demoted: 1,
        transientTtlDays: 7,
        promotionThreshold: 3,
        canonicalThreshold: 10
    },
    gossip: {
        eventsReceived: 142,
        episodesSynthesized: 18,
        lessonsStored: 7,
        gossipPeers: 4,
        lastEventSecsAgo: 12
    },
    agents: [
        {
            agentId: 'code-specialist',
            model: 'qwen3.5-9b',
            contextUtilizationPct: 67,
            alpha: 8.5,
            betaParam: 2.1,
            expectedValue: 0.80,
            totalObservations: 42,
            categoryStats: [
                { category: 'code', alpha: 6, betaParam: 1.5, expectedValue: 0.8, observations: 20 },
                { category: 'analysis', alpha: 3, betaParam: 2, expectedValue: 0.6, observations: 10 },
                { category: 'summary', alpha: 4, betaParam: 1, expectedValue: 0.8, observations: 8 }
            ]
        },
        {
            agentId: 'fast-synth',
            model: 'qwen3.5-0.8b',
            contextUtilizationPct: 23,
            alpha: 5.2,
            betaParam: 3.8,
            expectedValue: 0.58,
            totalObservations: 28,
            categoryStats: [
                { category: 'summary', alpha: 4, betaParam: 1, expectedValue: 0.8, observations: 15 },
                { category: 'format', alpha: 2, betaParam: 3, expectedValue: 0.4, observations: 8 },
                { category: 'code', alpha: 1, betaParam: 2, expectedValue: 0.33, observations: 5 }
            ]
        },
        {
            agentId: 'planner-agent',
            model: 'ministral-3b',
            contextUtilizationPct: 85,
            alpha: 6.0,
            betaParam: 1.5,
            expectedValue: 0.80,
            totalObservations: 35,
            categoryStats: []
        }
    ],
    timestamp: '2026-03-23T21:30:00Z'
});
```

## File Map

| File | Purpose |
|------|---------|
| `crates/arkavo-agui/src/types.rs` | `RequestContextTopology`, `ContextTopologyUpdate`, 8 snapshot structs |
| `crates/arkavo-agui/src/gateway_context.rs` | Backend handler, JSON-RPC aggregation, 6 unit tests |
| `crates/arkavo-agui/src/gateway_ws.rs` | Dispatch arm for `RequestContextTopology` |
| `crates/arkavo-agui/src/gateway_static.rs` | Static file serving for context JS |
| `crates/arkavo-agui/static/js/panels/context.js` | Panel module: polling, rendering, 5 zone builders |
| `crates/arkavo-agui/static/js/panels/context-charts.js` | SVG helpers: gauges, bars, circuit, funnel, radar, pulse |
| `crates/arkavo-agui/static/styles/dashboard.css` | Grid layout, zone styles, flow pulse animation |
| `crates/arkavo-agui/static/index.html` | Nav button, view panel, script tags |
| `crates/arkavo-agui/static/js/app.js` | Event routing, view switching, polling lifecycle |
| `crates/arkavo-agui/static/js/state.js` | `AppState.contextTopology` field |

## Agent-Side Integration

For agents to populate all zones, expose a `context/topology` JSON-RPC method that returns the snapshot fields above. The gateway falls back to `learning/status` for basic gossip data when `context/topology` is not available.
