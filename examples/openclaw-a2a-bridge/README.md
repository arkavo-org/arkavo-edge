# OpenClaw A2A Protocol Bridge

Demonstrates A2A protocol interoperability between Arkavo and OpenClaw, highlighting Arkavo's security-first advantages: TDF-encrypted context, budget caps, preflight policies, and local model support. Peers are discovered via mDNS (`runtime.mdns: true` in the kit).

## What You'll Learn

- A2A JSON-RPC 2.0 protocol interoperability between two agent ecosystems
- TDF payload-level encryption via KAS (Key Access Service)
- Preflight policy enforcement (PII blocking, shell command detection)
- Budget enforcement with session caps
- Local model inference vs cloud-only dependency

## Quick Start

```bash
make demo
```

One command: builds Arkavo, launches the agent, runs the five-act competitive narrative, generates a comparison report, and stops the agent.

## Architecture

```
┌─────────────────────┐         ┌─────────────────────────────────────┐
│     OpenClaw        │         │          Arkavo Agent               │
│                     │         │                                     │
│  Gateway            │  A2A    │  ┌──────────┐  ┌────────────────┐  │
│  ws://127.0.0.1:    │ JSON-RPC│  │ Preflight│──│ KAS (TDF)      │  │
│      18789          │◄───────►│  │ Policies │  │ AES-256-GCM    │  │
│                     │  :8360  │  └──────────┘  └────────────────┘  │
│  Model: cloud API   │         │  ┌──────────┐  ┌────────────────┐  │
│  Budget: none       │         │  │ Budget   │  │ ministral-3b   │  │
│  Encrypt: none      │         │  │ Tracker  │  │ (local, free)  │  │
│                     │         │  └──────────┘  └────────────────┘  │
└─────────────────────┘         └─────────────────────────────────────┘
                                     :8360 /.well-known/agent.json
```

## Security Comparison

| Feature              | Arkavo                   | OpenClaw               |
|----------------------|--------------------------|------------------------|
| Context encryption   | TDF (AES-256-GCM)       | Plaintext              |
| Budget enforcement   | $1.00/session cap        | None                   |
| PII protection       | Preflight block          | None                   |
| Model                | ministral-3b (local)     | cloud API (paid)       |
| Response latency     | ~340ms (local)           | ~1200ms (cloud RTT)    |
| Offline capability   | Full (local model)       | None                   |
| Cost for demo        | $0.00                    | ~$0.12 (estimated)     |
| Protocol             | A2A JSON-RPC 2.0         | A2A JSON-RPC 2.0       |

## TLS and Transport Security

Transport is plaintext HTTP on loopback (127.0.0.1). This is acceptable because:

1. Loopback traffic never leaves the machine
2. TDF provides payload-level encryption regardless of transport
3. Production deployments should enable TLS via `--tls-cert` and `--tls-key` flags

## Files

| File | Purpose |
|------|---------|
| `bridge-agent.swarmkit.yaml` | Arkavo bridge agent config (KAS + preflight + budget) |
| `SKILL.md` | OpenClaw skill definition for invoking the bridge |
| `bridge.sh` | Protocol bridge: curl-based A2A caller with security metadata |
| `demo.sh` | Five-act competitive narrative with report generation |
| `test-bridge.sh` | Automated A2A endpoint tests (6 tests) |
| `launch.sh` | Start Arkavo agent on port 8360 |
| `stop.sh` | Stop Arkavo agent |
| `tasks.json` | Demo task payloads (clean, PII, safe, budget-buster) |
| `Makefile` | Single entry point: `make demo`, `make test`, `make clean` |
| `RUNBOOK.md` | Step-by-step walkthrough with expected terminal output |

## Prerequisites

- Arkavo binary built with KAS feature: `cargo build -p arkavo --features kas`
- `curl` and `jq` installed
- OpenClaw installed (optional; demo uses simulated output if not present)

## JSON-RPC Methods

### Discovery

```bash
curl http://localhost:8360/.well-known/agent.json | jq .
```

### kas.publicKey

```bash
curl -X POST http://localhost:8360 \
  -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","id":1,"method":"kas.publicKey","params":{"request":{}}}'
```

### message/send

```bash
curl -X POST http://localhost:8360 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc":"2.0","id":2,
    "method":"message/send",
    "params":{
      "request":{
        "message":{
          "parts":[{"type":"text","content":"Explain TDF encryption"}]
        }
      }
    }
  }'
```

## Demo Acts

| Act | Title | What It Shows |
|-----|-------|---------------|
| 1 | Setup | Both ecosystems initialized, capabilities displayed |
| 2 | Task Flow | Same coding task processed with/without security layers |
| 3 | PII Protection | Arkavo blocks PII; OpenClaw sends it to the cloud |
| 4 | Budget Enforcement | Arkavo caps spending; OpenClaw processes unlimited |
| 5 | Offline | Arkavo works offline with local model; OpenClaw fails |
