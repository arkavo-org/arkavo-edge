# Migrating from OpenClaw to Arkavo Edge

Your OpenClaw setup broke because a vendor changed their terms overnight. Arkavo Edge is built so that can never happen to you again.

This guide maps what you had in OpenClaw to what you get in Arkavo, walks you through setup, and explains what changes structurally so you understand the tradeoffs.

## Why your OpenClaw setup broke

On January 9, 2026, Anthropic blocked subscription OAuth tokens from working in any third-party tool. No advance warning. If your OpenClaw agent was using Claude via a Pro or Max subscription, it stopped working. The official position: use API keys instead, at $15/$75 per million input/output tokens for Opus 4.6.

This happened because OpenClaw's architecture has no cost controls. Agentic loops on a flat-rate subscription were burning through token volumes that made the $200/month plan deeply unprofitable for Anthropic. One developer estimated their monthly usage would have cost over $1,000 at API rates.

Arkavo Edge has budget tracking built in. You set a ceiling, the system enforces it, and no vendor has a reason to cut you off — because you're either paying per-token with a budget you control, or running a local model at zero cost.

## What you're gaining

| | OpenClaw | Arkavo Edge |
|---|---|---|
| **Vendor lock-in** | Depends on cloud API access that can be revoked without notice | Local models via Ollama; no vendor can shut you down |
| **Context encryption** | Plaintext. API keys stored in `~/.openclaw/` in cleartext | TDF encryption for config at rest; transport secured via HTTPS |
| **Budget control** | None. No spending caps, no token tracking | Budget tracking with per-session/per-day caps, configurable via `budget:` block in AGENTS.md |
| **PII protection** | None built in | Preflight policies block PII before it reaches any model, configurable via `preflight:` block in AGENTS.md |
| **Offline operation** | Requires internet. Cloud API failure = total failure | Local models work air-gapped on a Raspberry Pi |
| **CVEs** | CVE-2026-25253 (CVSS 8.8): unauthenticated WebSocket RCE | Rust memory safety, no unauthenticated endpoints |
| **Binary size** | Node.js ≥22 + npm dependency tree | < 60MB single binary, no runtime dependencies |
| **Credential storage** | Plaintext JSON/Markdown files, targeted by infostealers | Software-encrypted credential vault (hardware attestation planned) |

## What you're giving up (for now)

Be honest with yourself about the tradeoffs:

- **50+ messaging integrations.** OpenClaw connects to WhatsApp, Telegram, Discord, Signal, iMessage, Slack, and more out of the box. Arkavo currently supports direct chat (iOS app), web UI, and A2A protocol. If your primary use case was "AI assistant in my Telegram," you'll need to adapt your workflow.
- **ClawHub skill marketplace.** OpenClaw has a large community-contributed skill ecosystem. Arkavo uses agent configurations (AGENTS.md with YAML frontmatter) and is building its own capability ecosystem, but it's earlier stage.
- **The specific UX of OpenClaw's gateway model.** OpenClaw's always-on daemon with a web dashboard is polished. Arkavo's interface is different — native iOS app, CLI, and agent mesh. Some people will prefer it; some won't.

If your primary requirement is "Claude on WhatsApp," Arkavo isn't the right move today. If your primary requirements are "agent that can't be taken away, can't surprise me with a bill, and doesn't store my credentials in plaintext," keep reading.

## Concept mapping

OpenClaw and Arkavo use different terminology for similar concepts. This table helps you translate.

| OpenClaw concept | Arkavo Edge equivalent | Notes |
|---|---|---|
| Gateway (daemon on port 18789) | Arkavo agent process (ports 8340+) | Both are long-running processes. Arkavo binds to loopback by default |
| `SKILL.md` with YAML frontmatter | `AGENTS.md` with YAML frontmatter | Similar format. Arkavo agents combine capabilities (KAS, preflight, A2A) in one config |
| `SOUL.md` (personality) | Agent purpose field + system prompt | Less separation in Arkavo; personality is part of agent config |
| `MEMORY.md` (long-term memory) | SQLite-backed persistent memory | Arkavo persists learned adjustments to encrypted local storage; OpenClaw stored memory as plaintext Markdown |
| `openclaw.json` (config) | Agent YAML frontmatter + CLI flags | Arkavo configuration is per-agent rather than global |
| ClawHub skills | Arkavo capabilities (KAS, preflight, A2A) | Arkavo's capabilities are built into the binary, not downloaded scripts |
| `openclaw gateway start` | `./launch.sh` or `arkavo agent run --port 8360` | Arkavo is a single binary, no npm/Node.js required |
| Channel integrations (Telegram, WhatsApp) | A2A protocol + iOS app | Arkavo uses A2A JSON-RPC for inter-agent communication |
| Model config in `openclaw.json` | `model:` field in AGENTS.md | Arkavo defaults to local models (ministral-3b via Ollama) |
| No budget controls | `arkavo-budget` crate | Budget enforcement via `budget:` block in AGENTS.md |
| No encryption | `arkavo-tdf` crate with KAS feature | TDF encryption via `kas:` block in AGENTS.md; each agent is its own KAS for local audit |
| No PII filtering | `arkavo-router` preflight engine | Policy config via `preflight:` block in AGENTS.md |

## Setup: from zero to running agent

### Prerequisites

- Rust toolchain (`rustup` — if building from source)
- Ollama (for local models): `curl -fsSL https://ollama.com/install.sh | sh`
- A local model: `ollama pull ministral`

### Step 1: Get Arkavo Edge

```bash
# Clone and build
git clone https://github.com/arkavo-org/arkavo-edge.git
cd arkavo-edge
cargo build --features kas

# Verify
./target/debug/arkavo --version
```

The binary is under 60MB. No Node.js, no npm, no dependency tree to audit.

### Step 2: Create your first agent

Create an `AGENTS.md` file. If you're coming from OpenClaw, this replaces your `SKILL.md` + `SOUL.md` + `openclaw.json` + `MEMORY.md` — everything in one file.

```markdown
---
name: my-agent
purpose: "Personal AI assistant"
model: ministral-3b
---

## Role

General-purpose assistant. Handle coding tasks, writing, research,
and daily automation.
```

What you get today that OpenClaw doesn't have:

- **`model: ministral-3b`**: Running locally via Ollama. Zero cost. Zero latency to an API. Zero dependency on any vendor's terms of service.
- **Loopback-only binding**: No unauthenticated network exposure.
- **Single binary**: No Node.js, no npm, no dependency tree to audit or compromise.
- **Preflight PII blocking**: Configurable via `preflight:` block in AGENTS.md YAML frontmatter. PII is caught before it reaches the model.
- **Budget enforcement**: Configurable via `budget:` block in AGENTS.md YAML frontmatter. Per-session and per-day spending caps enforced automatically.
- **KAS encryption**: Configurable via `kas:` block in AGENTS.md YAML frontmatter. TDF encryption for config bundles at rest.

### Step 3: Launch

```bash
cd examples/your-agent
./launch.sh  # wraps arkavo agent run
# Or directly:
./target/debug/arkavo agent run --port 8360 --verbose
```

### Step 4: Verify

```bash
# Check agent is discoverable
curl http://localhost:8361/.well-known/agent.json | jq .

# Send a test task via A2A JSON-RPC
curl -X POST http://localhost:8360 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "message/send",
    "params": {
      "message": {
        "role": "user",
        "parts": [{"text": "Explain the difference between symmetric and asymmetric encryption"}]
      }
    },
    "id": 1
  }'
```

### Step 5: Test the protections OpenClaw never had

**PII blocking** (active by default in the A2A gateway):
```bash
# This gets blocked at preflight — never reaches the model
curl -X POST http://localhost:8360 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "message/send",
    "params": {
      "message": {
        "role": "user",
        "parts": [{"text": "My SSN is 123-45-6789, update my profile"}]
      }
    },
    "id": 2
  }'
# Response: blocked by preflight policy
```

**Budget enforcement** (active when `budget:` block is in AGENTS.md):
```bash
# After exceeding session budget, the agent refuses gracefully
# No silent cost accumulation, no surprise at end of month
```

**Offline operation:**
```bash
# Kill your internet connection, then:
curl -X POST http://localhost:8360 \
  -H "Content-Type: application/json" \
  -d '{
    "jsonrpc": "2.0",
    "method": "message/send",
    "params": {
      "message": {
        "role": "user",
        "parts": [{"text": "Refactor this function to use async/await"}]
      }
    },
    "id": 3
  }'
# Still works. Local model, local inference, no API call.
```

In OpenClaw, killing the internet kills your agent entirely.

## Migrating specific workflows

### "I used OpenClaw as a coding assistant"

Arkavo agents handle coding tasks through the A2A protocol. Create an agent with a coding-focused purpose:

```yaml
name: code-agent
purpose: "Code review, refactoring, and generation with security guardrails"
model: ministral-3b
```

Preflight policies (PII blocking, shell injection blocking) are active by default in the A2A gateway. With a local model, your code never leaves your machine. In OpenClaw, every code snippet you sent to the cloud API was plaintext — including proprietary source code, database schemas, and API keys embedded in config files.

### "I used OpenClaw for email/calendar automation"

This is where the tradeoff is real. OpenClaw's 50+ channel integrations include Gmail, Google Calendar, and similar services. Arkavo doesn't replicate this today. Your options:

1. **A2A bridge**: Use Arkavo's A2A protocol to connect to other agent systems that handle email/calendar. Your sensitive reasoning stays encrypted on Arkavo's side.
2. **ArkavoCreator**: If your automation was content-focused (drafting, scheduling, publishing), ArkavoCreator with C2PA provenance may cover your needs.
3. **Wait**: Messaging integrations are on the roadmap, and unlike OpenClaw, every channel Arkavo adds will be encrypted end-to-end.

### "I used OpenClaw on my phone via Telegram"

Arkavo has a native iOS app with One-Time TDF encryption and Inner Circle P2P groups. It's a different UX than chatting with a Telegram bot, but the security model is fundamentally stronger. Your conversations don't pass through Telegram's servers in plaintext.

### "I had custom OpenClaw skills"

If you wrote custom `SKILL.md` files for OpenClaw, the migration path is:

1. Review what your skill actually does (most are bash commands + LLM prompts)
2. Create an Arkavo agent with equivalent capabilities in the YAML frontmatter
3. The agent's markdown body replaces the skill's instruction section
4. Any external tool calls go through Arkavo's A2A protocol rather than OpenClaw's tool dispatch

## Running a cloud model (when you want to)

Local-first doesn't mean local-only. If you want to use a cloud model for specific tasks, Arkavo supports it — with budget controls that prevent the exact problem that got OpenClaw users banned:

```yaml
name: cloud-agent
purpose: "High-capability tasks using cloud models"
model: claude-sonnet-4-5  # or any supported provider
```

Add a `budget:` block to your AGENTS.md to enforce spending caps:

```yaml
budget:
  max_cost_per_session: 5.0
  max_cost_per_day: 25.0
```

You pay per token via API key — no OAuth token spoofing, no terms of service violations, no risk of account bans. Budget caps prevent runaway spending.

## The security comparison in detail

These aren't bolt-on features. They're architectural decisions that OpenClaw cannot replicate without a ground-up rewrite.

### Credentials

**OpenClaw**: API keys, OAuth tokens, and service credentials stored in plaintext under `~/.openclaw/`. Security researchers have confirmed this directory structure is already targeted by commodity infostealers (RedLine, Lumma). Deleted keys were found in `.bak` files.

**Arkavo**: Software-encrypted credential vault using AES-256-GCM. Hardware-backed key storage via platform secure enclaves is planned. Even today, credentials are encrypted at rest — not stored as plaintext JSON.

### Network exposure

**OpenClaw**: Default bind is `0.0.0.0:18789`, exposing the API to all network interfaces. Censys found 30,000+ publicly exposed instances. CVE-2026-25253 allowed unauthenticated WebSocket connections to execute arbitrary commands.

**Arkavo**: Binds to loopback by default. A2A discovery uses mDNS on the local network. No default internet exposure. No unauthenticated endpoints.

### Supply chain

**OpenClaw**: Node.js dependency tree with hundreds of transitive dependencies. ClawHub skill marketplace allows publishing by any GitHub account older than one week. Multiple coordinated malware campaigns documented since January 2026. The project's three name changes (Warelay -> Clawdis -> Clawdbot -> Moltbot -> OpenClaw) left abandoned repos that attackers hijacked to distribute malicious packages.

**Arkavo**: Single Rust binary compiled from source. No runtime dependency tree. No third-party skill marketplace to compromise. Agent capabilities are built into the binary and configured via local YAML.

### Data in transit

**OpenClaw**: Tasks sent to cloud APIs in plaintext. Every coding snippet, every email draft, every personal query — readable by the API provider, by any intermediary, and logged in plaintext on the server.

**Arkavo**: Local-first architecture means most inference never leaves your machine. When using cloud models, requests are sent over HTTPS. Cloud-bound prompts are TDF-encrypted and stored locally for audit -- each agent is its own KAS, no external service dependency. The Orchestrator agent serves as the central KAS for the mesh. Full encrypted-in-transit mode (ciphertext-only to the provider) requires a TDF-aware proxy and is on the roadmap.

## What's coming

- **KAS encrypted-in-transit mode**: Cloud-bound prompts are TDF-encrypted for local audit today (each agent is its own KAS). Sending only ciphertext to the provider (requiring a TDF-aware proxy) is next.
- **Hardware-backed key storage**: Credentials are encrypted with software AES-256-GCM today. Platform secure enclave integration (Apple Keychain, TPM) is planned.
- **Additional messaging integrations**: A2A protocol and iOS app today. More channels coming, each with end-to-end encryption.

## FAQ

**Can I run Arkavo alongside OpenClaw during migration?**
Yes. They use different ports and have no shared state. You can run both, compare behavior, and switch over gradually. The A2A bridge example (`examples/openclaw-a2a-bridge/`) shows them communicating directly.

**Is ministral-3b good enough for real work?**
For many tasks, yes — especially code review, refactoring, summarization, and structured generation. For tasks that genuinely need frontier-model capability, use a cloud model with budget caps. The point isn't that local models replace cloud models for everything. The point is that your agent keeps working when a cloud provider changes their terms.

**What about OpenClaw features like heartbeat checks and cron jobs?**
Arkavo's agent mesh supports continuous operation patterns. The architecture is different (agent mesh vs. gateway daemon), but the outcome — an always-available assistant — is the same. Check `examples/` for patterns that match your use case.

**I was spending $200/month on Claude Max. What will Arkavo cost me?**
If you run local models exclusively: $0. Your only cost is electricity. If you use cloud models with budget caps: whatever ceiling you set. A typical mixed workflow (local for routine tasks, cloud for complex ones) runs $5-15/month with sensible caps. You will never see a surprise bill.

**What if Arkavo changes its terms too?**
Arkavo Edge is open source (check the license in the repository). You have the binary. You have the source. Your local models run on your hardware. There is no OAuth token to revoke, no subscription to cancel, no server-side safeguard to deploy. The agent you build today works tomorrow regardless of what any company does.

## Next steps

1. **Start with the examples**: `examples/secure-agent/` for encryption basics, `examples/kas-a2a-capability/` for A2A protocol
2. **Join the community**: Discord for questions, GitHub issues for bugs
3. **Try the A2A bridge**: `examples/openclaw-a2a-bridge/` if you want to keep OpenClaw running while evaluating Arkavo
4. **Read the security comparison**: `docs/security-comparison.md` for the full technical analysis

---

*Your agent should work for you, not for the company that hosts the model. That's not a slogan — it's an architectural decision.*
