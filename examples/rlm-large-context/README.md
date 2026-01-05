# RLM Large Context Demo

This example demonstrates RLM (Recursive Language Models) handling contexts 100x beyond a model's context window.

## The Story

A local 7B model with an 8K context window needs to analyze a 100K+ token codebase for security vulnerabilities. Without RLM, it would truncate most of the code. With RLM, it decomposes the codebase into chunks, searches for relevant sections, and probes specific code on demand.

## Why This Matters

1. **Edge AI Enablement**: Local models can now handle enterprise-scale codebases
2. **Cost Efficiency**: Only fetch what you need, not the entire context
3. **Privacy**: Large codebases stay on-device, only queries go to the model

## Quick Start

### Prerequisites

```bash
# Build Arkavo
cargo build -p arkavo

# Verify RLM tools are available
../../target/debug/arkavo chat --prompt "List available context tools"
```

### Run the Demo

```bash
# 1. Generate a large synthetic codebase
./generate_codebase.sh

# 2. Run the analysis task
./run_analysis.sh

# 3. Watch RLM decompose and query the context
```

## How It Works

### RLM Activation

When context exceeds 70% of model's context window:

1. **Decompose**: Large input split into ~4KB semantic chunks
2. **Manifest**: Chunk metadata stored with hints for searching
3. **System Prompt**: LLM instructed to use context tools
4. **Tool Loop**: LLM calls `context_search` and `context_probe` as needed

### Context Tools

| Tool | Description |
|------|-------------|
| `context_decompose` | Break large text into searchable chunks |
| `context_probe` | Fetch specific chunks by index |
| `context_search` | Find chunks matching keywords |

### The Analysis Flow

```
Large Codebase (100K+ tokens)
         │
         ▼
┌─────────────────────┐
│ context_decompose   │ → Manifest: 25 chunks, 102K tokens
└─────────────────────┘
         │
         ▼
┌─────────────────────┐
│ context_search      │ → "auth", "password" → chunks 3, 7, 15
│ (keywords)          │
└─────────────────────┘
         │
         ▼
┌─────────────────────┐
│ context_probe       │ → Fetch chunks 3, 7, 15 (~12K tokens)
│ (indices)           │
└─────────────────────┘
         │
         ▼
    LLM Analysis
    (fits in 8K window!)
```

## Expected Output

```
━━━━━━ RLM ACTIVATION ━━━━━━

[RLM] Context size: 102,400 tokens
[RLM] Model context: 8,192 tokens
[RLM] Activating RLM mode...

━━━━━━ DECOMPOSITION ━━━━━━

[RLM] Decomposing context into chunks...
[RLM] Created manifest: rlm-a1b2c3d4
[RLM] Chunks: 25
[RLM] Total tokens: 102,400

Chunk previews:
  [0] main.rs: fn main() { ... } (4,096 tokens)
  [1] auth/mod.rs: pub mod auth { ... } (4,096 tokens)
  [2] auth/login.rs: async fn login() { ... } (4,096 tokens)
  ...

━━━━━━ SEARCH ━━━━━━

[LLM] Searching for security-relevant code...
[TOOL] context_search("rlm-a1b2c3d4", ["password", "auth", "sql"])

Found 3 matching chunks:
  [3] auth/password.rs - password hashing
  [7] db/queries.rs - SQL query builder
  [15] api/auth.rs - authentication endpoints

━━━━━━ PROBE ━━━━━━

[TOOL] context_probe("rlm-a1b2c3d4", [3, 7, 15])

Fetched 12,288 tokens (3 chunks)

━━━━━━ ANALYSIS ━━━━━━

[LLM] Analyzing security vulnerabilities...

Security Report:
1. auth/password.rs:42 - Using MD5 for password hashing (CRITICAL)
2. db/queries.rs:128 - SQL injection vulnerability (HIGH)
3. api/auth.rs:67 - Missing rate limiting (MEDIUM)
```

## Directory Structure

```
rlm-large-context/
├── README.md              # This file
├── RUNBOOK.md             # Detailed test procedures
├── generate_codebase.sh   # Generate synthetic large codebase
├── run_analysis.sh        # Run the RLM analysis demo
├── synthetic_repo/        # Generated test codebase (gitignored)
└── logs/                  # Runtime logs (gitignored)
```

## Architecture

```
┌──────────────────────────────────────────────────┐
│                   arkavo task                     │
│         "Analyze codebase for vulnerabilities"    │
└──────────────────────┬───────────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────────┐
│              RLM Context Manager                  │
│  ┌─────────────┐  ┌─────────────┐  ┌──────────┐  │
│  │ Decomposer  │  │  Manifest   │  │  Chunks  │  │
│  │ (semantic)  │──│  Storage    │──│  Cache   │  │
│  └─────────────┘  └─────────────┘  └──────────┘  │
└──────────────────────┬───────────────────────────┘
                       │
                       ▼
┌──────────────────────────────────────────────────┐
│                 MCP Tool Loop                     │
│  context_search → context_probe → LLM response   │
└──────────────────────────────────────────────────┘
```

## Video Recording Tips

1. **Hook**: "What if your local AI could analyze codebases 100x larger than its memory?"
2. **Setup**: Show the 100K token codebase being generated
3. **Problem**: "A 7B model only has 8K tokens - that's 8% of the code"
4. **Solution**: RLM decomposes into chunks
5. **Search**: LLM finds relevant security code
6. **Probe**: Only fetches what it needs
7. **Result**: Complete security analysis from a tiny model
8. **Payoff**: "Edge AI just got enterprise-grade"
