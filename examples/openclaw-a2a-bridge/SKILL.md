---
name: arkavo
description: "Send coding tasks to an Arkavo agent via A2A JSON-RPC. Use when tasks need encrypted context, budget-controlled inference, or local model processing."
metadata:
  openclaw:
    emoji: "\U0001F510"
    requires:
      bins: ["curl", "jq"]
---

# Arkavo A2A Skill for OpenClaw

This skill allows OpenClaw to route coding tasks to an Arkavo agent via the A2A
JSON-RPC 2.0 protocol.

## Invocation

From an OpenClaw session:

```
bash command:"./bridge.sh 'your coding task'"
```

## What Happens

1. **Discovery** - The bridge queries `http://localhost:8360/.well-known/agent.json`
   to verify the Arkavo agent is running and discover its capabilities.

2. **Task Submission** - The bridge sends a `message/send` JSON-RPC request to
   `http://localhost:8360` with the coding task.

3. **Security Processing** - Arkavo applies preflight policies (PII blocking,
   shell command detection), encrypts context via TDF, and tracks budget before
   invoking the local model.

4. **Result Return** - The response includes the task result plus security
   metadata: encryption status, preflight evaluation, budget tracking, and model
   used.

## When to Use

- Tasks requiring encrypted context (TDF payload-level encryption)
- Budget-controlled inference (session cap prevents runaway costs)
- Local model processing (no cloud dependency, zero inference cost)
- PII-sensitive inputs (preflight blocks before data reaches the LLM)

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `ARKAVO_A2A_ENDPOINT` | `http://localhost:8360` | Arkavo JSON-RPC endpoint |
| `ARKAVO_DISCOVERY_URL` | `http://localhost:8360` | Arkavo Agent Card endpoint |
