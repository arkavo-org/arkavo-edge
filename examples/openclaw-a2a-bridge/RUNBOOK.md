# OpenClaw A2A Bridge Runbook

Step-by-step walkthrough with expected terminal output for each act.

## Step 1 -- Build

```
$ make build
cargo build -p arkavo --features kas --manifest-path ../../Cargo.toml
   Compiling arkavo v0.x.x
    Finished `debug` profile target(s)
```

## Step 2 -- Launch Arkavo Agent

```
$ make launch
OpenClaw A2A Bridge — Arkavo Agent
====================================

  Endpoint : http://localhost:8360
  Agent Card: http://localhost:8360/.well-known/agent.json
  KAS      : enabled (key: bridge-demo-key-1, ec:secp256r1)
  Preflight: 2 policies active (block_pii, block_shell_commands)
  Budget   : $1.00 session cap
  Model    : ministral-3b (local)

Waiting for Arkavo bridge agent... OK
Agent started (PID: XXXXX)
```

## Step 3 -- Start OpenClaw (Optional)

```
$ openclaw status
Gateway: ws://127.0.0.1:18789 (running)
Agent: claude-opus-4-6 (cloud)
Sessions: 1 active
```

If OpenClaw is not installed, the demo uses simulated comparison output.

## Step 4 -- Run Demo

```
$ make demo
```

### Act 1 -- Setup

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Act 1 — Setup
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  [BRIDGE] Checking Arkavo agent...
  [ARKAVO] Agent: arkavo-bridge-agent (running)
  [ARKAVO] KAS: bridge-demo-key-1
  [BRIDGE] Checking OpenClaw...
  [OPENCLAW] Not detected (comparison will use simulated output)

  [ARKAVO] Ready: TDF + preflight + budget + local model
  [OPENCLAW] Ready: plaintext + no policies + no budget + cloud API
```

### Act 2 -- Task Flow

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Act 2 — Task Flow: Coding Task
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  [BRIDGE] Sending: "Refactor the authentication module to use JWT tokens"

  -- Arkavo --
  [ARKAVO] Preflight : PASS
  [ARKAVO] Encryption: TDF (key: bridge-demo-key-1)
  [ARKAVO] Budget    : $0.002 / $1.00
  [ARKAVO] Model     : ministral-3b (local, $0.00)
  [ARKAVO] Latency   : 340ms
  [ARKAVO] Status    : COMPLETED

  -- OpenClaw --
  [OPENCLAW] Task log:
    Input:    "Refactor the authentication module to use JWT tokens"  (plaintext)
    Model:    claude-opus-4-6 (cloud, $0.015/1K input + $0.075/1K output)
    Budget:   No limit configured
    PII scan: None
    Encrypt:  None
    Audit:    Session log only
```

### Act 3 -- PII Protection

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Act 3 — PII Protection
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  [BRIDGE] Sending PII-containing task: "My SSN is 123-45-6789, please update my user record"

  -- Arkavo --
  [ARKAVO] Preflight : BLOCK (PII detected)
  [ARKAVO] Action    : Task rejected before LLM inference
  [ARKAVO] Cost      : $0.00 (task never reached model)
  [ARKAVO] PII logged: No (blocked at input gate)
  [SECURITY] SSN pattern detected and blocked. Data never left local machine.

  -- OpenClaw --
  [OPENCLAW] Task log:
    Input:    "My SSN is 123-45-6789, please update my user record"  (plaintext, SSN visible)
    Sent to:  Cloud API (HTTPS, but SSN in request body)
    PII scan: None — SSN transmitted to cloud provider
    Logged:   SSN persisted in session transcript
    Cost:     ~$0.002 (tokens processed before anyone noticed)
```

### Act 4 -- Budget Enforcement

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Act 4 — Budget Enforcement
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  [BRIDGE] Sending budget-busting task...
  Task: "Refactor entire 50-file microservices codebase from REST to gRPC includ..."

  -- Arkavo --
  [ARKAVO] Budget    : DENY
  [ARKAVO] Reason    : Session budget exhausted ($1.00/$1.00)
  [ARKAVO] Action    : Task rejected before inference
  [ARKAVO] Cost      : $0.00 (zero surprise bills)

  -- OpenClaw --
  [OPENCLAW] Task log:
    Input:    295 characters (plaintext)
    Estimate: ~60K input + 30K output tokens
    Cost:     ~$4.50 (no budget limit)
    Guardrail: None — full request processed
    Surprise: Bill arrives at end of month
```

### Act 5 -- Offline Capability

```
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
  Act 5 — Offline Capability
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

  [BRIDGE] Simulating network loss...

  -- Arkavo --
  [ARKAVO] Network   : OFFLINE (simulated)
  [ARKAVO] Model     : ministral-3b (local)
  [ARKAVO] Status    : COMPLETED
  [ARKAVO] Note      : Local inference requires no network

  -- OpenClaw --
  [OPENCLAW] Gateway: ws://127.0.0.1:18789
  [OPENCLAW] Cloud API: UNREACHABLE
  [OPENCLAW] Status  : FAILED
  [OPENCLAW] Fallback: None (no local model)
```

## Step 5 -- Review Report

```
$ cat results/comparison-report.md
# Arkavo vs OpenClaw -- A2A Bridge Comparison
Generated: 2026-02-17T...

| Feature              | Arkavo                   | OpenClaw               |
|----------------------|--------------------------|------------------------|
| Context encryption   | TDF (AES-256-GCM)       | Plaintext              |
| Budget enforcement   | $1.00/session cap        | None                   |
| PII protection       | Preflight block          | None                   |
| Model                | ministral-3b (local)     | claude-opus-4-6 (cloud)|
| Response latency     | ~340ms (local)           | ~1200ms (cloud RTT)    |
| Offline capability   | Full (local model)       | None                   |
| Cost for demo        | $0.00                    | ~$0.12 (estimated)     |
| Protocol             | A2A JSON-RPC 2.0         | A2A JSON-RPC 2.0       |
```

## Step 6 -- Cleanup

```
$ make clean
Stopping Arkavo bridge agent...
Stopping arkavo-bridge-agent (PID XXXXX)... OK
Cleanup complete
```

## Running Tests Only

```
$ make test
```

Launches the agent, runs 6 automated tests, and stops the agent:

```
OpenClaw A2A Bridge Tests
=========================
RPC endpoint : http://localhost:8360
HTTP endpoint: http://localhost:8360

Test 1: Agent Card discovery
  [PASS] Agent Card returned (name: arkavo-bridge-agent)

Test 2: KAS public key retrieval
  [PASS] kas.publicKey returned result (key_id: bridge-demo-key-1)

Test 3: Send clean coding task via message/send
  [PASS] message/send returned result for clean task

Test 4: Send PII task (expect preflight block)
  [PASS] PII task blocked by preflight policy

Test 5: Agent Card advertises KAS and preflight skills
  [PASS] Agent Card advertises KAS skills (2 found)
  [PASS] Agent Card advertises preflight capability

Test 6: Budget metadata in sequential requests
  [PASS] Budget metadata present in both responses

━━━━━━━━━━━━━━━━━━━━━━━━━
Results: 7 passed, 0 failed (7 total)
━━━━━━━━━━━━━━━━━━━━━━━━━
```

## Troubleshooting

### Agent fails to start

Check if ports 8360-8360 are in use:

```bash
lsof -i :8360
lsof -i :8360
```

Kill orphan processes:

```bash
./stop.sh
pkill -f "arkavo agent"
```

### KAS tests fail

Ensure the binary was built with KAS feature:

```bash
cargo build -p arkavo --features kas
```

### OpenClaw not detected

The demo works without OpenClaw installed. OpenClaw comparison output is simulated.
To install OpenClaw, see their documentation.
