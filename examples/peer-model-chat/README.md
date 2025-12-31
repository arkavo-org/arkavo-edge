# Peer Model Chat: Two Agents Collaborate Using Different Models

This example demonstrates peer-to-peer agent collaboration where two agents with different model specializations help each other handle tasks outside their expertise.

## The Story

Alice and Bob are peer agents with complementary capabilities:
- **Alice** uses Qwen3-0.6B, optimized for fast reasoning and general chat
- **Bob** uses Ministral-3B, optimized for structured output and code

When a question arrives that is outside an agent's specialty, they automatically query their peer for help via the A2A protocol.

## Why This Matters

- **Model Specialization**: Different models excel at different tasks
- **Peer Collaboration**: Agents recognize their limitations and seek help
- **Zero-Config Discovery**: mDNS enables automatic peer discovery
- **Edge Computing**: Low-latency local inference without cloud dependency

## Quick Start

### Prerequisites

```bash
# Build Arkavo
cargo build -p arkavo

# Ensure models are available (auto-download on first use)
# - Qwen3-0.6B for Alice
# - Ministral-3B for Bob
```

### Run the Demo

```bash
# 1. Launch both agents
./launch_chat.sh

# 2. In another terminal, inject a knowledge gap question
./inject_question.sh

# 3. Watch the peer collaboration in logs
tail -f logs/*.log

# 4. Stop the agents when done
./stop_chat.sh
```

## Directory Structure

```
peer-model-chat/
├── README.md                    # This file
├── launch_chat.sh               # Start both agents
├── stop_chat.sh                 # Stop all agents
├── inject_question.sh           # Inject knowledge gap questions
├── alice/
│   └── AGENTS.md                # Alice config (Qwen3-0.6B)
├── bob/
│   └── AGENTS.md                # Bob config (Ministral-3B)
└── logs/                        # Runtime logs
```

## Model Capabilities

| Agent | Model | Strengths | Weaknesses |
|-------|-------|-----------|------------|
| Alice | Qwen3-0.6B | Fast reasoning, general chat, creative writing | Code generation, structured output |
| Bob | Ministral-3B | Code generation, JSON/YAML, technical specs | Creative writing, casual chat |

## How It Works

### Agent Discovery

Both agents use mDNS for zero-configuration discovery:
- Service type: `_a2a._tcp.local.`
- Alice advertises on port 8370
- Bob advertises on port 8371

### Knowledge Gap Detection

When an agent receives a request outside its expertise:
1. Agent analyzes the request type (code, creative, general)
2. Agent checks if request matches its skills
3. If mismatch, agent queries peer via `agent_query` RPC

### Peer Query Flow

```
User -> Alice: "Write Rust binary search"
Alice -> Recognizes code request
Alice -> A2A query to Bob
Bob -> Generates code with Ministral-3B
Bob -> Returns to Alice
Alice -> Relays to User
```

## Architecture

```
┌─────────────────┐                    ┌─────────────────┐
│     Alice       │<───── A2A ────────>│      Bob        │
│  (Qwen3-0.6B)   │                    │  (Ministral-3B) │
│  port: 8370     │                    │  port: 8371     │
│                 │                    │                 │
│  Strengths:     │                    │  Strengths:     │
│  - Fast chat    │                    │  - Code gen     │
│  - Creative     │                    │  - JSON/YAML    │
│  - Reasoning    │                    │  - Tech specs   │
└─────────────────┘                    └─────────────────┘
```

## Expected Output

When running `./inject_question.sh`:

```
━━━━━━ INJECTING KNOWLEDGE GAP QUESTION ━━━━━━

[INJECT] Asking Alice a code question...

Question: "Write a Rust function that implements binary search."

[ALICE ] Response received
{
  "jsonrpc": "2.0",
  "result": {
    "task_id": "...",
    "status": "submitted"
  },
  "id": 1
}
```

## Extending the Example

### Add More Agents

Create additional peer directories with specialized models:
- `charlie/` with vision model for image tasks
- `diana/` with reasoning model for complex analysis

### Custom Knowledge Gaps

Modify the `inject_question.sh` to test different scenarios:
- Math problems (route to reasoning specialist)
- Image description (route to vision specialist)
- Translation (route to multilingual specialist)
