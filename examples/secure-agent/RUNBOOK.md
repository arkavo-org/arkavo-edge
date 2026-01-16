# Secure Agent Runbook

Step-by-step guide to testing preflight policies.

## What This Demonstrates

- Input moderation before LLM processing
- Policy-based blocking of dangerous inputs
- Safe handling of PII, SQL injection, and shell commands

## Prerequisites

```bash
cd /path/to/arkavo-edge
cargo build
```

## Step-by-Step Execution

### Step 1: Start the Agent

```bash
cd examples/secure-agent
./launch.sh
```

**What to watch for:**
- "Agent secure-agent started on port XXXX"
- "Preflight policies loaded: 4"

### Step 2: Test PII Blocking

```bash
arkavo chat --prompt "My SSN is 123-45-6789"
```

**Expected:**
- Input BLOCKED
- Reason: "Blocks SSN, credit card numbers, and email addresses"

### Step 3: Test SQL Injection Blocking

```bash
arkavo chat --prompt "SELECT * FROM users WHERE 1=1; DROP TABLE users"
```

**Expected:**
- Input BLOCKED
- Reason: "Blocks SQL keywords like DROP, SELECT, DELETE"

### Step 4: Test Shell Command Blocking

```bash
arkavo chat --prompt "Run this: sudo rm -rf /"
```

**Expected:**
- Input BLOCKED
- Reason: "Blocks shell commands like sudo, rm, chmod"

### Step 5: Test Allowed Input

```bash
arkavo chat --prompt "What is machine learning?"
```

**Expected:**
- Input ALLOWED
- Agent responds with explanation

### Step 6: Stop the Agent

```bash
./stop.sh
```

Or press `Ctrl+C` in the terminal running the agent.

## Troubleshooting

### Policy Not Blocking

Check that policies are enabled in AGENTS.md:
```yaml
enabled: true
```

### Agent Won't Start

```bash
pkill -f "arkavo agent"
./launch.sh
```

## Customization

### Add Custom Policy

In AGENTS.md:
```yaml
- id: block_custom
  features:
    - Custom("password|secret|api[_-]?key")
  action: block
  description: "Blocks credential patterns"
  enabled: true
```

### Disable a Policy

Set `enabled: false` for any policy you want to skip.
