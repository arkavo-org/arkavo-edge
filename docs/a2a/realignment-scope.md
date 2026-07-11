# A2A Spec Realignment — Scope of Work & Suggested Architecture

**Status:** scoping (2026-06-17) · **Owner:** A2A realignment · **Decisions locked:** CWT/DID-only auth · big-bang cutover · edge is both server & client · "more standard the better."

## Problem

`arkavo-protocol`'s A2A surface has **drifted from the A2A specification**. It exposes ~30 bespoke JSON-RPC methods (plus OAuth2/JWT/mTLS auth, challenge-response onboarding, chunked file transfer, DLP, rate limiting) where the standard A2A surface is **11 operations**. We realign onto the spec-conformant stack — `a2a-lf` (standard core) + `arkavo-a2a-rs` (Arkavo extension layer) — moving genuinely-A2A concerns **into the extension layer** and evicting non-A2A concerns **off the wire**. Outcome: the A2A wire carries only A2A; everything else lives on a purpose-built surface.

This document scopes the changes required and proposes the target architecture. It is not a task-by-task execution plan.

---

## Suggested architecture

### Target topology

```
                         ┌─────────────────────────── arkavo-edge process ───────────────────────────┐
   A2A peers / Swift     │                                                                            │
   clients (CWT/DID) ───▶│  Transports          Spec middleware            Integration seam           │
                         │  ┌───────────┐        ┌──────────────────┐      ┌──────────────────────┐   │
   WS  (arkavo/a2a) ────▶│  │A2AWsServer│──┐     │ CwtAuthLayer     │      │ ArkavoRequestHandler │   │
   iroh(arkavo/a2a)────▶ │  │A2AIrohSrv │──┼────▶│ (verify CWT/DID) │─────▶│  (impl a2a-lf        │   │
   HTTP(jsonrpc) ──────▶ │  │jsonrpc_rtr│──┘     │ GatedDispatcher  │      │   RequestHandler)    │   │
                         │  └───────────┘        │ (TØR-G policy)   │      └──────────┬───────────┘   │
                         │  AgentCard producer   │ rate-limit layer │                 │ delegates to  │
                         │  (/.well-known/...)   │ DLP interceptor  │                 ▼               │
                         │                       └──────────────────┘   router · conductor ·          │
                         │                                              task store · learning bus ·   │
                         │   ── A2A wire ends here ──                   MCP-tools-as-skills            │
                         │                                                                            │
                         │   OFF-WIRE surfaces (not A2A):                                              │
                         │   • AG-UI HTTP/SSE  → metrics, budget, ARP                                  │
                         │   • agent-local HTTPS → config (AGENTS.md), specialize                      │
                         │   • KAS service     → kas.publicKey / kas.rewrap                            │
                         └────────────────────────────────────────────────────────────────────────────┘
```

### Key architectural decisions

1. **One new integration crate: `arkavo-a2a-edge`.** It owns the dependency tree from the spec stack (iroh, rustls-no-provider), installs the ring crypto provider once, implements `a2a_server::RequestHandler` (`ArkavoRequestHandler`), produces the AgentCard, and assembles the transports + middleware. This isolates the heavy/foreign deps from the rest of the workspace and gives one place to reason about the seam. `arkavo-a2a-rs` is vendored as a workspace-excluded submodule under `vendor/` and consumed by path.

2. **Handler delegates to existing internals — nothing in the conductor/router/task-store changes.** The drift is entirely at the protocol/transport layer. `ArkavoRequestHandler` maps the 11 standard ops onto the *same* delegation chains the bespoke `#[rpc]` impl uses today (`crates/arkavo-server/src/server/handlers/*`). Recommendation: implement `RequestHandler` **directly** (not via `DefaultRequestHandler`) because arkavo-edge has a dual delegation path — orchestrator (`agent_event_tx.send(AgentEvent::IncomingMessage)`) vs specialist (`execute_with_conductor_and_learning`) — that `DefaultRequestHandler`'s single-executor model doesn't capture. Reuse `arkavo-tasks::TaskStore` behind the handler for `get_task`/`list_tasks`/`cancel_task`.

   | Standard op | Delegates to (existing) |
   |---|---|
   | `send_message` / `send_streaming_message` | `handlers::messaging::handle_message_send` → conductor + `router.route_with_tools_hinted` + `task_executor` + `learning_bus` |
   | `get_task` | `task_store.get_task` + `task_store.get_task_result` |
   | `list_tasks` | `conductor.store().list_all` / `task_store.list_tasks` |
   | `cancel_task` | `task_executor.update_task_status(.., Canceled)` |
   | `subscribe_to_task` | adapt the tokio `broadcast` delta stream → `BoxStream<StreamResponse>` |
   | `get_extended_agent_card` | AgentCard producer + `mcp_registry.list_all_tools()` (MCP tools surfaced as skills) |

3. **Streaming adapter.** Today streaming is jsonrpsee subscription sinks fed by tokio `broadcast` channels (`handlers::streaming`, via a broadcast→mpsc forwarder). The spec stack returns `BoxStream<Result<StreamResponse>>`. A thin adapter wraps the receiver as a `BoxStream` — delta *production* is untouched; only the egress shape changes. **Lag is a correctness hazard:** `broadcast` drops on lag, and the current forwarder (`while let Ok(d) = broadcast_rx.recv()`, `chat_session.rs:424`+) silently exits on `RecvError::Lagged` — a slow subscriber gets a *truncated* stream that looks like normal completion, not an error. The adapter MUST map `Lagged` to an explicit stream error (or a resync); WS2 carries a backpressure acceptance test, not just round-trip.

4. **Auth: P-256 CWT/DID, minted from device identity.** See "Identity" below — this is the single biggest *new* behavior, because agent identity is currently **Ed25519** and CWT mandates **P-256**.

5. **Policy: a new `PolicyEvaluator` adapter over existing circuits.** See "Policy" below — arkavo-edge has the circuit-eval building blocks but **no async request-gate evaluator** today; one must be built.

6. **iroh: share one endpoint.** The agent already runs an iroh `Endpoint` (`arkavo-tdf-iroh::IrohNode`, N0 preset, `iroh_blobs::ALPN`) for TDF blobs. `arkavo-a2a-iroh::A2AIrohServer` uses its own ALPN (`arkavo/a2a/1`). Target: **one endpoint, both ALPN protocols registered on the iroh `Router`** — not two iroh nodes (binary-size + port budget). This likely needs a small upstream change to `arkavo-a2a-iroh` to accept an injected `Endpoint` (see upstream candidates).

7. **MCP tools become A2A skills.** The existing `A2aMcpBridge` (in `arkavo-protocol`) is re-homed into `arkavo-a2a-edge`: MCP tools from `arkavo-mcp-tools`/`arkavo-mcp-mesh` are advertised as `AgentCard.skills` and invoked via `SendMessage`, rather than via a bespoke `mcp/*` RPC. **No `rmcp` is added to `arkavo-a2a-rs`** (core-neutrality; A2A≠MCP) — the bridge stays an arkavo-edge concern.

8. **A2A wire becomes task-focused; everything else moves to a dedicated surface.** Metrics/budget/ARP → AG-UI (which already has handlers); config/specialize → agent-local HTTPS; KAS → standalone service; rate-limit/DLP → middleware layers.

---

## Capability disposition matrix

Codes: **STANDARD** = native `a2a-lf` op · **EXT** = `arkavo-a2a-rs` extension · **MSG** = standard A2A message + AgentCard skill · **OFF-WIRE** = arkavo-edge non-A2A surface · **DROP** = removed.

| Bespoke surface | Disp. | Target |
|---|---|---|
| `message/send`, `message/stream` | STANDARD | `send_message` / `send_streaming_message` |
| `tasks/get` · `tasks/list` · `tasks/cancel` · (subscribe) | STANDARD | `get_task` / `list_tasks` / `cancel_task` / `subscribe_to_task` |
| `chat/open` · `chat/send` · `chat/close` | MSG | `send_streaming_message`; session via `Message.contextId`; teaching events via `Message.metadata` |
| `rpc.discover` · `discover_features_*` · `agent_capabilities_get` | DROP→STANDARD | AgentCard + `get_extended_agent_card` |
| `agent_discover` | EXT/OFF-WIRE | AgentCard + `arkavo-a2a-iroh` discovery |
| `agent.specialize` · `agent.config.*` | MSG | unified bundle transport: `SendMessage` + TDF-encrypted `Part` (`arkavo-a2a-tdf`) + AgentCard skill, delivered via `arkavo-config-transport` |
| `system.metrics` · `system.metrics.subscribe` | OFF-WIRE | AG-UI `/metrics` + SSE `/metrics/subscribe` |
| `budget.compute_status` · `arp/get` | OFF-WIRE | AG-UI `/budget/status` + `/arp/{agent}/document` (handlers already exist) |
| `kas.publicKey` · `kas.rewrap` | OFF-WIRE | standalone KAS service (`arkavo-tdf::KasA2aHandler`) |
| `tdf/offer` · `tdf/share` | EXT | `arkavo-a2a-tdf` Part-level NanoTDF |
| `challenge_request` · `challenge_verify` | DROP | superseded by CWT minting |
| file upload/transfer (chunked) | STANDARD | A2A `FilePart`/artifacts in messages |
| `A2aMcpBridge` | OFF-WIRE | re-homed; MCP tools → A2A skills |
| OAuth2 / JWT / mTLS-identity | DROP | CWT/COSE + DID:key |
| `A2aPolicy` (allow/deny) | EXT | `arkavo-a2a-policy::GatedDispatcher` + new `PolicyEvaluator` |
| rate limiting | OFF-WIRE/EXT | tower layer (already exists) |
| DLP / data-classification | OFF-WIRE | new `CallInterceptor`/tower layer (**security-critical**) |
| SQLite session persistence | OFF-WIRE | arkavo-edge task/context store |

---

## Scope of work (by workstream)

Sizing: **S** ≈ days · **M** ≈ 1–2 wks · **L** ≈ 2–4 wks · **XL** ≈ 4 wks+. Dependencies in parentheses.

### WS0 — Upstream-gap spike · **S–M**
Front-loads integration risk and produces the *evidence* for every upstream decision. **The spike code is not throwaway — it becomes the WS1/WS2 foundation.**
- **Build** the minimal `arkavo-a2a-edge` seam (WS1 skeleton) + a handler/transport vertical slice (start of WS2: an `EchoHandler` round-tripped over `A2AWsServer`/`A2AWsClient`), specifically to make the compiler surface every place we'd otherwise patch the spec stack.
- **Enumerate & triage** each gap into one of three lanes:
  - **(a) Arkavo extension / missing capability → `arkavo-a2a-rs`** (ours, fast). **Default home for anything the spec stack can't yet do.** Known member: injected-endpoint `A2AIrohServer::serve_on(endpoint, …)` to share the TDF iroh node (WS6).
  - **(b) Upstream *bug* in `a2a-lf` → fix on the `arkavo-ai/a2a-rs` fork (BUG-FIX-ONLY) and PR to LF upstream.** The fork exists for bugs, **not features** — a missing *capability* is solved in lane (a), never by feature-patching the fork. This keeps the fork diff minimal and rebasable onto `a2aproject/a2a-rs`, so it retires as LF merges the fixes.
  - **(c) arkavo-edge-local glue** — no upstream action.
- **Land** lane-(a) changes in `arkavo-a2a-rs`, **tag a release**, and re-point the vendored submodule (WS1) at the tag. The `a2a-lf` dep stays pinned to the bug-fix-only fork.
- **Hard-block on LF governance (DEC-6, decided):** if a gap can be met *only* by an `a2a-lf` **feature** change (not a bug, not layerable in arkavo-a2a-rs), it does **not** go on the fork — it blocks on an LF-governance contribution. This is an **accepted critical-path risk** (out of our control), which is exactly why WS0 runs first: surface any such gap as early as possible. Probe the known-needed extension points in the spike (e.g. injected iroh endpoint — an arkavo-a2a-rs extension, *not* a2a-lf core) to confirm none require an a2a-lf feature.
- **Deliverable:** the triaged upstream-gap list + a tagged `arkavo-a2a-rs`. This *is* the answer to "should upstream adopt before we migrate" — evidence-driven, not guessed.
- **Fork policy (DEC-5, resolved):** the `a2a-lf` pin tracks the `arkavo-ai/a2a-rs` fork, which is **bug-fix-only**; capability gaps go to lane (a).
- **Swift lockstep:** the same triage + tag applies to `arkavo-a2a-swift` (its `a2a-swift` fork is bug-fix-only too). Pair the Swift tag to the Rust tag at the same spec rev, gated green by the conformance matrix (WS11).

### WS1 — Dependency intake & integration crate · **M** (WS0)
- **Create** `vendor/arkavo-a2a-rs` submodule (Apache-2.0 head), workspace `exclude`, path deps for `arkavo-a2a` + the `a2a-lf` git pin (`rev 7676ec9…`) in `[workspace.dependencies]`.
- **Create** `crates/arkavo-a2a-edge` with `install_crypto_provider()` (ring), re-exports, and the assembly entry point.
- **Verify** no OpenSSL enters the tree; iroh is shared, not duplicated.
- **Risk:** Cargo workspace-inheritance — arkavo-a2a-rs crates must resolve `workspace = true` against *their own* root, so they stay excluded from arkavo-edge's workspace and are consumed by path only.

### WS2 — RequestHandler & server bring-up · **L** (WS1)
- **Create** `arkavo-a2a-edge/src/handler.rs` (`ArkavoRequestHandler` impl of the 11 ops, delegating per the table above), `src/server.rs` (bind `A2AWsServer` + shared-endpoint `A2AIrohServer` + `jsonrpc_router`), `src/card.rs` (`AgentCardProducer` advertising extensions + MCP-tool skills), `src/stream.rs` (broadcast→`BoxStream` adapter).
- **Modify** `crates/arkavo-cli/src/commands/agent.rs::start_agent_server` (~1160) and `crates/arkavo-server/src/server/a2a_server.rs` (`A2aServer::new`/`start_with_port`) to construct the same dependency set (conductor, router, task store, learning bus, mcp registry, iroh) and hand them to `arkavo-a2a-edge::serve()` instead of the jsonrpsee `#[rpc]` server.
- **Reuse** `arkavo-tasks::TaskStore` (trait already matches: `create/get/update_status/list/result`).
- **Swift-reachable ingress:** `A2AWsServer` (WS) is the primary transport Swift agents use (`URLSessionWebSocketTask`); bind it on a stable, externally reachable address. (See Cross-language parity.)
- **Risk:** orchestrator-vs-specialist dual path must be preserved inside `send_message`.
- **Risk (streaming backpressure):** the broadcast→`BoxStream` adapter must handle `RecvError::Lagged` explicitly — today's forwarder exits silently on lag (latent truncation bug, `chat_session.rs:424`+). Add a backpressure/lag acceptance test across **both** delegation paths, not just a happy-path round-trip.

### WS3 — Identity: P-256 CWT/DID · **L** (WS2)
- **The core change:** agent identity is **Ed25519** today (`arkavo-crypto::AgentKeypair`, used at `welcome.rs`/`agent.rs`); CWT requires **P-256**. `arkavo-crypto::P256SigningKeypair`/`P256VerifyingKey` **already exist with DID:key support** (`to_did_key`, multicodec 0x1200) but are currently only used for iOS registration.
- **Modify** agent startup to provision a **P-256 identity** (persist via `arkavo-device-identity::keypair`), set `AgentIdentity.did = P256VerifyingKey::to_did_key()`.
- **Create** `arkavo-a2a-edge/src/identity.rs`: build `ConfirmationKey{x,y}` from the SEC1 public bytes; construct `CwtCredential::new(p256_signing_key, iss, sub=agent_did, aud, cnf)`; attach `CwtCallInterceptor` on clients and `CwtAuthLayer` on the server.
- **Decision required (DEC-1):** existing agents hold Ed25519 identities. Options: (a) **migrate** agent identity to P-256 (cleanest, one identity); (b) **dual-key** — keep Ed25519 for legacy, add P-256 for A2A CWT. Recommend (a) for new agents + a one-time re-provision for existing; flag any external system that pinned the Ed25519 DID. Ties into [[project_hardware_attestation_p0]] (HATT hardware root should issue the P-256 key).
- **Replaces** the `mtls_integration` / `oauth2` / `onboarding_integration` behaviors with CWT/DID regression tests (valid passes; expired/replayed/forbidden-issuer rejected per `aia-identity-v1`).

### WS4 — Policy: TØR-G gate evaluator · **L** (WS2)
- **Finding:** there is **no async request-gate evaluator** today. `arkavo-torg` is LLM constrained-decoding; `arkavo-torg-circuits::CompiledCircuit::evaluate(input) -> Option<usize>` is a boolean-circuit evaluator; `arkavo-arp-runtime` records outcomes. The building blocks exist; the gate does not.
- **Create** `arkavo-a2a-edge/src/policy.rs`: an `A2aPolicyBridge` implementing `arkavo_a2a_policy::PolicyEvaluator`, mapping `GateContext{op,message,taint,tenant}` → circuit features (`CircuitFeature` impl) → `CompiledCircuit::evaluate` → `Decision::{Allow,Deny(Rejection)}`, with `Rejection.policy_id/rule_id` sourced from the ARP document. Wrap the handler in `GatedDispatcher::new(handler, evaluator)`.
- **Delete** `arkavo-protocol::a2a_policy` — policy moves **out of the protocol layer** (DEC-2). The evaluator lives in this repo (`arkavo-a2a-edge`), delegating to `arkavo-torg-circuits`; it is not in `arkavo-protocol` and not pushed upstream to `arkavo-a2a-rs`. *Optional:* extract a reusable `arkavo-torg-gate` crate if gating is ever needed beyond A2A.
- **Confirm at build time:** the TØR-G circuit source (compiled artifact + feature map). *Taint feed is settled (verified):* `GateContext.taint` arrives on the inbound `Message.metadata` (`GatedDispatcher` → `Taint::from_message`, `arkavo-a2a-policy/src/server.rs:61`); arkavo-edge has no internal DLP/SEQ provenance in the request path today (types/tripwire only), so the gate is server-boundary middleware with **no conductor plumbing**.
- **Proof:** message ops deny → `TASK_STATE_REJECTED`; task ops deny → JSON-RPC −32099 with `arkavo.torg.v1.Rejection`.

### WS5 — TDF parts + KAS split · **M** (WS2)
- **Replace** `tdf/offer`+`tdf/share` wire methods with `arkavo-a2a-tdf` Part-level NanoTDF (`encrypt_inline`/`decrypt_inline`/`make_b3_url_part`) carried inside A2A messages.
- **Extract** KAS (`kas.publicKey`/`kas.rewrap`, backed by `arkavo-tdf::KasA2aHandler`) into a **standalone HTTPS service** surface (`/kas/public-key`, `/kas/rewrap`) — off the A2A wire, minimizing key-op attack surface.

### WS6 — iroh / discovery reconciliation · **M** (WS2)
- **Share** the single `arkavo-tdf-iroh::IrohNode` endpoint; register the A2A ALPN alongside `iroh_blobs::ALPN` on the iroh `Router`.
- **Upstream candidate:** `arkavo-a2a-iroh::A2AIrohServer::serve` currently builds its own endpoint — needs a variant that accepts an injected `Endpoint`. Push this change into `arkavo-a2a-rs` (see below).
- **`RelayGateway` for Swift:** stand up `arkavo-a2a-iroh::RelayGateway` (HTTPS+SSE) so Swift agents — which have no native iroh — reach iroh-discovered Rust agents. Required for Swift↔Edge mesh reachability (WS11).
- Discovery via AgentCard + iroh card resolution; reconcile with gossip per [[project_channel_architecture]].

### WS7 — Off-wire relocations · **L** (WS2; parallelizable)
Good news: AG-UI already has the handlers for most of these.
- **Metrics** (`arkavo-server/.../mod.rs:1172-1220,1332-1488`) → AG-UI HTTP `/metrics` + SSE `/metrics/subscribe` (`arkavo-agui` gateway_events; `MetricsCollector` stays).
- **Budget** (`mod.rs:1155`) → AG-UI `/budget/status` (`BudgetAgUiEvent` exists in `arkavo-budget`, `budget_handler.rs`).
- **ARP** (`handlers/arp.rs`) → AG-UI `/arp/{agent}/document` (`arp_handler.rs` exists; per-agent keyed — see [[project_arp_per_agent]]).
- **Config + Specialize — unified bundle transport (DEC-4).** `handlers/config.rs` and `handlers/specialization.rs` (`BundleDecryptor`) collapse into one path: an encrypted bundle delivered orchestrator→agent as a **standard A2A `SendMessage`** carrying a TDF-encrypted `Part` (`arkavo-a2a-tdf`), advertised as an AgentCard skill, with `arkavo-config-transport` as the bundle-delivery layer. No bespoke `/config*` or `/specialize` side-channel. (This row is **MSG**, not off-wire — it is legitimately agent↔agent, just expressed in the standard message shape.)
- **MCP bridge** (`a2a_mcp_bridge.rs`) → re-home into `arkavo-a2a-edge`; tools advertised as AgentCard skills.

### WS8 — Middlewares (rate-limit + DLP) · **L** (WS2) · **security-critical, net-new**
- **Rate limit:** `rate_limit_middleware.rs` is already an axum tower layer — wire it globally on the new server; remove per-method manual checks.
- **DLP:** `data_classification.rs` is **types only — no middleware exists yet.** Build a `CallInterceptor`/tower layer that scrubs request/response bodies. **Must keep green:** `security_vulnerabilities`, `mock_provider`, `e2e_security_test.sh`, `dlp_pii_security_test.sh` (per CLAUDE.md pre-push protocol). **This is the one workstream that is net-new behavior gating merge, not a relocation of working code** — the interceptor is built from scratch (by someone who didn't write the original DLP logic) and must keep four security suites green. Sized **L**, not M, for that reason; do not let DLP regress.

### WS9 — Consumer big-bang sweep + delete bespoke · **XL** (WS2–8)
- **Rewrite** 254 `arkavo_protocol::` references across 13 crates from `{A2aRequest,A2aResponse,A2aEndpoint,A2aTransport,HttpTransport}` to `a2a_client::A2AClient<…Transport>` + native ops. Order: clients first (`arkavo-cli` mesh/task/chat/agent, `arkavo-mcp-mesh`, `arkavo-orchestrator/mesh_strategy`, `arkavo-config-transport`, `arkavo-server/agent_loop`), then drop the server `#[rpc]` modules, then the re-export-only crates (`arkavo-session`, `arkavo-tasks`, `arkavo-agui`, `arkavo-github`, `arkavo-openclaw`).
- **Delete** the bespoke A2A modules in `arkavo-protocol` (transport, http, websocket, oauth2, jwt/auth, openrpc, discovery, registration, a2a_mcp_bridge, chat_session, file_transfer, …). OAuth2/JWT/mTLS-identity deletion is unblocked (DEC-3: no external auth clients). Keep only what relocated (DLP types, rate-limit, metrics collector) until their new homes land.
- **Big-bang — chosen deliberately (2026-06-17).** Rationale: coexistence would mean two server stacks over the *same* shared mutable state (task store, iroh node, learning bus, conductor) behind a long-lived feature flag threaded through 254 sites — that dual-path wiring is its own integration risk and churn; reaching one consistent surface fast is preferred over a prolonged two-surface period. *(Confirm/extend with the deciding constraint if different.)*
- **Mitigations (address the review's no-green-tree concern):** WS0 de-risks the seam; land WS3–WS8 as **independently-tested commits** — each relocation's own unit/integration + security suites pass *before* the consumer sweep, so only the mechanical consumer-wiring is deferred to merge, not the relocations' correctness. Keep the branch **short-lived**; conformance matrix (WS10) + security suites are the merge gate.

### WS10 — Conformance CI (the Swift↔Rust sync gate) · **M** (WS9)
- **Create** an `a2a-conformance` adapter wrapping `arkavo-a2a-edge::serve()` + client, mirroring `adapters/arkavo-ext-rust`. Run the rs↔swift matrix (ws / transport-equivalence / identity / policy / tdf / discovery) in CI so interop with `arkavo-a2a-swift` is continuously proven. **A green matrix is the definition of "in sync"** — it gates both the `arkavo-a2a-rs` and `arkavo-a2a-swift` tags.

### WS11 — Swift agent interop & lockstep · **L** (WS0 tag + WS2/WS6; partly sibling-repo)
- **Parallelize early:** needs only the WS0 tag + a reachable WS2/WS6 server — **does NOT wait for the WS9 sweep.** Starting Swift here gives the "real round-trip" deliverable maximum runway and surfaces interop bugs while the Rust side is still malleable.
- **Co-version & tag** `arkavo-a2a-swift` to the same spec rev as the WS0 `arkavo-a2a-rs` tag; keep the `a2a-swift` fork bug-fix-only (parity with the Rust fork policy).
- **Reachability:** confirm Edge binds WS ingress + the `RelayGateway` so SwiftAgentKit agents connect over `URLSessionWebSocketTask` / HTTPS-SSE (no native iroh on Swift).
- **Build out the Swift agents** (sibling repos): SwiftAgentKit agent on `arkavo-a2a-swift` (CWT/DID mint, rejection honoring, TDF Parts, relay discovery), on-device inference via `mlx-swift-lm`. Prove a real Swift agent ↔ Edge round-trip beyond the conformance harness (message/task lifecycle, streaming).
- **Deliverable:** a SwiftAgentKit agent that completes a task against Edge, green across the matrix it exercises.

---

## Functionality to push *into* `arkavo-a2a-rs` (upstream candidates)

The **full list is produced by the WS0 spike** and triaged across three lanes — **(a)** arkavo-a2a-rs (ours, fast; the default home for capability gaps), **(b)** `a2a-lf` **bug fixes only** on the arkavo-ai fork + PR to LF, **(c)** arkavo-edge-local. **The `a2a-lf` fork is bug-fix-only — features never accrete there;** keep the core-neutrality invariant too (no `rmcp`, no Arkavo-app coupling in wire/ws):
- **Injected-endpoint `A2AIrohServer`** (WS6) — a `serve_on(endpoint, …)` variant so a host that already runs iroh can share it. Clean, vendor-neutral, lane (a); the one gap known before the spike.
- **DLP-as-policy (maybe):** if DLP scrubbing is expressed as taint + `PolicyEvaluator` rules, part of it could live in `arkavo-a2a-policy`. Default: keep in arkavo-edge unless the conformance specs adopt it.
- Everything else (metrics/budget/config/KAS/MCP-bridge) is **not** A2A and stays in arkavo-edge.

---

## Cross-language parity (Swift ↔ Rust)

Goal: **Swift agents (SwiftAgentKit + `arkavo-a2a-swift`, on-device inference via `mlx-swift-lm`) communicate seamlessly with the Rust agent Edge.** The Swift and Rust extension layers are siblings implementing the same `a2a-conformance/specs/arkavo/*` contracts, so "in sync" has a precise definition: **the rs↔swift conformance matrix is green for every feature Edge uses.**

- **Co-versioning lockstep.** `arkavo-a2a-rs` and `arkavo-a2a-swift` pin to the **same spec rev**, and every tag is paired. WS0's `arkavo-a2a-rs` tag gets a matching `arkavo-a2a-swift` tag; the conformance matrix must be green before either ships.
- **Symmetric bug-fix-only forks.** Just as Rust rides a forked `a2a-lf` (`arkavo-ai/a2a-rs`), Swift rides a forked `a2a-swift` (`arkavo-ai/a2a-swift`, pinned `a1473fa…`, v0.1.1). **Both LF-SDK forks are bug-fix-only**; Swift capability gaps go into `arkavo-a2a-swift`, never the `a2a-swift` fork — same triage as WS0.
- **Transport reachability (architecture, not a fork).** Swift's transports are asymmetric to Rust's: WS via `URLSessionWebSocketTask` (Apple — full), but **no native iroh** (Rust-only). So Edge must offer Swift-reachable ingress:
  - **WS (`A2AWsServer`)** — primary Swift↔Edge transport (Apple platforms; WS2).
  - **`RelayGateway` (HTTPS+SSE)** — lets Swift agents reach iroh-discovered Rust agents without native iroh (WS6).
  - Plain HTTPS JSON-RPC for vanilla A2A peers.
- **Feature parity = the wire features Edge requires.** Every extension Edge mandates must be implemented + conformance-green on Swift: CWT/DID identity (mint + present), TØR-G **rejection honoring** on the client, TDF Parts, discovery via relay. `arkavo-a2a-swift` has all four targets; WS0/WS10 produce the parity-gap + matrix-skip list (native-iroh cells = skip on Swift, relay cells = run).
- **Boundary.** The Swift *agent runtime* (SwiftAgentKit + mlx) lives in **sibling repos**, not arkavo-edge. Edge owns the reachable, conformance-tested A2A server surface; the Swift agent owns speaking it. SwiftAgentKit already supports A2A **and** MCP, dovetailing with MCP-tools-as-skills.

## Outstanding decisions

- **DEC-1 (identity): RESOLVED — migrate Ed25519→P-256, HATT-issued.** Agent identity becomes a single P-256 key from the hardware root. (WS3)
- **DEC-2 (policy): RESOLVED — stays in this repo, out of the protocol layer.** The `PolicyEvaluator` lives in `arkavo-a2a-edge` (A2A glue) delegating to `arkavo-torg-circuits`; `arkavo-protocol::a2a_policy` is **deleted**. Not pushed upstream to `arkavo-a2a-rs`. (WS4) — **taint plumbing verified wire-carried** (`GatedDispatcher` reads `Message.metadata`; no internal DLP/SEQ provenance exists in the request path today), so the gate is server-boundary middleware with no conductor change. Residual unknown is only the **circuit artifact + feature map** (a data input the evaluator loads), not internal threading.
- **DEC-3 (external auth): RESOLVED (2026-06-17) — no OAuth2/JWT clients today.** CWT/DID-only is clean; WS9 deletes the OAuth2/JWT/mTLS-identity modules outright. No edge gateway required.
- **DEC-4 (specialize): RESOLVED — unified bundle transport, expressed as a standard A2A message.** Specialize and `agent.config.*` are the same pattern (encrypted bundle, orchestrator→agent). Both ride **one** transport (`arkavo-config-transport`), expressed as `SendMessage` carrying a TDF-encrypted bundle `Part` (`arkavo-a2a-tdf`) + an AgentCard skill — no side-channel. (WS5/WS7)
- **DEC-5 (source-of-truth remote): RESOLVED — pin tracks the `arkavo-ai/a2a-rs` fork, which is BUG-FIX-ONLY.** The fork exists solely to carry upstream `a2a-lf` bug fixes not yet in LF; no feature accretion. Keep the diff minimal + rebasable onto `a2aproject/a2a-rs`; retire fork patches as LF merges them. Missing *capabilities* go to `arkavo-a2a-rs`, never the fork.
- **DEC-6 (fork-feature fallback): RESOLVED — hard-block on LF governance.** An a2a-lf-feature-only gap never touches the fork; it blocks on an LF contribution. Accepted critical-path risk — WS0 surfaces any such gap first.
- **Sequencing (WS9): big-bang, reaffirmed (2026-06-17)** despite the review's incremental-coexistence case; rationale + mitigations captured in WS9.

## Risks

- **P-256 identity migration** is real behavior change, not cosmetic (DEC-1).
- **DLP middleware must be built and proven** — types exist, the interceptor does not; security suites gate the merge (WS8).
- **Big-bang broken window** across 254 sites — mitigate with seam-first sequencing and merge-time CI gate.
- **iroh sharing** needs the upstream injected-endpoint change, else two iroh nodes blow the binary/port budget (WS6).
- **Reconcile, don't duplicate:** TØR-G (WS4), identity (WS3), iroh (WS6) each must unify with the existing crate, not run a parallel implementation.
- **`a2a-lf` fork is bug-fix-only; feature-only gaps hard-block on LF (DEC-6).** This is an accepted critical-path risk outside our control — the single most likely schedule-slipper. WS0 runs first precisely to surface it early; keep the fork thin and retirable.

## Rough sizing summary

| WS | Title | Size |
|---|---|---|
| 0 | Upstream-gap spike (→ tag arkavo-a2a-rs) | S–M |
| 1 | Dependency intake & integration crate | M |
| 2 | RequestHandler & server bring-up | L |
| 3 | Identity: P-256 CWT/DID | L |
| 4 | Policy: TØR-G gate evaluator | L |
| 5 | TDF parts + KAS split | M |
| 6 | iroh / discovery reconciliation | M |
| 7 | Off-wire relocations | L |
| 8 | Middlewares (rate-limit + **net-new** DLP interceptor) | L |
| 9 | Consumer big-bang sweep + delete bespoke | XL |
| 10 | Conformance CI (Swift↔Rust sync gate) | M |
| 11 | Swift agent interop & lockstep | L |

Critical path: **WS0 (spike → land arkavo-a2a-rs PRs → tag, paired with an arkavo-a2a-swift tag)** → WS1 (vendor the tag) → WS2 → {WS3, WS4, WS5, WS6, WS7, WS8 in parallel} → WS9 → WS10 (conformance gate). **WS11 (Swift agents) branches off after WS0's tag + WS2/WS6 ingress — it does NOT wait for WS9.** WS0's spike code carries forward into WS1/WS2 — it is foundation, not a throwaway prototype. Swift co-versioning (WS11) tracks every Rust tag from WS0 onward; the conformance matrix (WS10) is the green-gate for both.
