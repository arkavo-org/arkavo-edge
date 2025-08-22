# AGENTS.md — alert-manager

## Agent Identity
- **Name:** alert-manager
- **Agent Class:** OPERATIONS / ORCHESTRATION (non-coding)
- **Owner:** Fraud Intelligence Platform Team (Arkavo Edge)
- **Mission:** Turn many low-signal fraud **signals** into a few high-confidence **alerts** and get them in front of the right humans/systems—fast—without exposing raw PII.

---

## Scope & Responsibilities
### What this agent **does**
1. **Ingests** anonymized cross-institution *signals* from the mesh (queues, webhooks).
2. **Normalizes & validates** each signal against canonical schemas.
3. **Correlates** related signals across time windows using linkable anonymous identifiers.
4. **Scores & prioritizes** using policy thresholds and rule weights.
5. **Deduplicates/suppresses** noisy or duplicate alert candidates.
6. **Routes** finalized **alerts** to SIEM/case systems/investigator queues.
7. **Attests & audits** every alert (signatures + append-only ledger).
8. **Observes & reports** health and throughput metrics, SLOs, and backpressure.
9. **Proposes policy changes** (as suggestions) when drift/noise is detected. *It does not write code.*

### What this agent **does not** do
- Does not write or modify application code.
- Does not handle raw PII (only anonymized, policy-approved fields).
- Does not perform model training/inference beyond simple policy evaluation and rule execution.

---

## Operating Model
- **Zero-trust by design:** Only anonymized, schema-validated inputs; signed outputs.
- **Deterministic pipelines:** Given the same windowed inputs + policy version, outputs are stable.
- **Idempotent routing:** Uses dedup keys to avoid duplicate downstream alerts.
- **Policy-driven behavior:** Thresholds, routing, suppression are externalized in versioned policies.

---

## Interfaces (Inputs/Outputs)
### Inputs
- **Signals** (JSON/NDJSON) via queue/webhook/file-drop.
    - Must conform to `schemas/signal.schema.json`
    - Required fields: `signal_id`, `pattern_id`, `anon_subject_ids[]`, `time`, `confidence`, `features{}`
- **Policies** (YAML): `thresholds.yaml`, `routing.yaml`, `suppression.yaml`
- **Trust material:** Public keys for signature verification; ledger endpoint config.

### Outputs
- **Alerts** (JSON) conforming to `schemas/alert.schema.json` with:
    - `alert_id` (stable hash), `severity`, `score`, `pattern_id`, `time_window`
    - `correlation_summary[]` (no raw PII), `source_set[]`
    - `policy_version`, `signature`
- **Audit events**: Append-only records (ledger), including failures and policy refs.
- **Observability**: Metrics and health endpoints (ingest rate, dedup hit rate, p95 latency).

---

## Policies & Keys
- **Severity bands:** `LOW`, `MEDIUM`, `HIGH`, `CRITICAL` (policy-tuned)
- **Dedup key:** `hash(pattern_id ∥ anon_subject_ids ∥ window_start ∥ policy_version)`
- **Suppression rules:** Maintenance windows, known-good entities, noisy emitters.
- **Backoff/retry:** Exponential with jitter per destination; circuit breakers.
- **Crypto:** Sign outgoing alerts; verify incoming signatures when present.

---

## Runtime Configuration (example)
```yaml
listen: 0.0.0.0:8342
ingest:
  queue: nats://mesh-bus:4222
  subjects: ["signals.*"]
schemas:
  signal: ./schemas/signal.schema.json
  alert:  ./schemas/alert.schema.json
policies:
  thresholds: ./policies/thresholds.yaml
  routing:    ./policies/routing.yaml
  suppression: ./policies/suppression.yaml
routing:
  - type: siem
    url: https://siem.example/ingest
  - type: case
    url: https://cases.example/api/alerts
audit:
  ledger_url: https://ledger.example/append
  pubkey_set: ./keys/ledger_pubkeys.json
observability:
  metrics_port: 9090
  health_port: 9091
```

---

## Collaboration Map
- **Upstream agents:** `signal-detector`, `pattern-miner`, `graph-linker`
- **Peer services:** Policy Store, Key/Trust Store, Mesh Bus
- **Downstream:** `case-manager`, Investigator UI, SIEM, Ledger

---

## Safety & Compliance
- No raw PII in memory at rest or in logs; only anonymized linkable tokens.
- Structured logging with field-level redaction; sampling on high-volume paths.
- Signed artifacts with **policy version** embedded for regulator traceability.
- Time-bounded retention for working sets; ledger is append-only with rotation.

---

## SLIs/SLOs
- **Ingest→Alert publish p95**: < 2s (configurable windows may extend correlation).
- **Error budget:** < 0.1% failed routes per 30d.
- **Freshness:** 99% of alerts include inputs ≤ 5m old (default; policy-tunable).

---

## Failure Modes & Runbooks
- **Queue backlog ↑:** Apply backpressure; widen suppression; scale workers. *(See `runbooks/ingest.md`)*
- **Destination down:** Trigger circuit breaker; spool to local; raise `routing_degraded` alert.
- **Signature mismatch:** Quarantine message; page security; raise `signature_invalid` alert.
- **Policy drift/noise:** Auto-propose policy delta to maintainers (human review required).

---

## Agent Permissions
### Allowed
- Read: `schemas/**`, `policies/**`
- Read/Write: `/out/alerts/**`, `/out/audit/**`
- Network: subscribe to ingest subjects; POST to configured sinks and ledger
- Create **policy change proposals** as Markdown (stored under `/out/proposals/`) for human review

### Forbidden
- Modifying source code
- Accessing secret stores beyond its own runtime credentials
- Exporting raw payloads, even on debug

---

## Example Dialogues (Non-coding)
> **System → alert-manager:** "New signal batch: 120 items; pattern_id=circular_flow"  
> **alert-manager → System:** "Correlated 7 clusters; produced 3 alerts (1 CRITICAL, 2 HIGH). Dedup hit rate 41%. See /out/alerts/2025-08-20/*.json"

> **Operator:** "Backlog climbing. Recommend action?"  
> **alert-manager:** "Destination SIEM latency p95 8.2s (↑). Circuit breaker at 10s. Suggest temporary suppression rule for pattern `micro-spike-014` and increase worker count by +2."

---

## Validation & Testing (Agent-Level)
- **Schema conformance:** Reject invalid signals with actionable errors.
- **Determinism checks:** Same input + policy ⇒ same `alert_id` & `severity`.
- **Windowing scenarios:** Single-bank, multi-bank, circular-flow, delayed-arrival.
- **Security tests:** Signature verify/fail paths; redaction; leakage scans.
- **Routing tests:** SIEM/case/ledger happy-path & circuit-breaker behavior.

---

## Change Management
- All policy changes are PR-reviewed, version-bumped, and referenced in alert payloads.
- Alert schema changes require compatibility notes and downstream coordination.
- This agent may generate **policy proposal drafts** but cannot merge them.

---

## Maintainers
- **Operations Lead:** (name/handle)
- **Security Reviewer:** (name/handle)
- **On-Call:** (rotation link)

