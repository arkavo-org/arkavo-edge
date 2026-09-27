# SwarmKit PR-review — WS-D (mesh bundle shipping over the Iroh data plane) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the `apply_kit` pipeline ship a per-role specialization bundle over the **real mesh + Iroh data plane** instead of test stubs. Compose existing infra only: stage the already-wrapped TDF bundle on `arkavo-tdf-iroh`'s `IrohTransport` → ticket; resolve the target agent's A2A address via the existing `AgentRegistry`; send `agent.specialize` carrying the **iroh ticket** over the existing A2A JSON-RPC send path; have the `agent.specialize` handler fetch the blob by ticket (`IrohTransport.fetch_bytes`) before the existing `unwrap_bundle`. Provide a production `RoleTaskTransport` that dispatches each role's first task + start signal over the same A2A path.

**Architecture:** Three concrete impls of three existing trait seams, plus one minimal, backward-compatible handler change.
- `IrohBundleShipper` (impl `BundleShipper`, `arkavo-orchestrator`) holds an `Arc<IrohTransport>` (data plane) + `Arc<AgentRegistry>` (address resolution) + a `requester_did`. `ship()` stages bytes → ticket, resolves the agent address, sends `agent.specialize { ticket }` over `HttpTransport` (the same idiom already used in `task_executor/mesh_strategy.rs`).
- `MeshRoleTaskTransport` (impl `RoleTaskTransport`, `arkavo-orchestrator`) holds the same `Arc<AgentRegistry>` + an A2A transport config; `dispatch()` resolves the address and sends the role's `initial_task` via `message/send` (identical to `SendTaskTool`/`mesh_strategy`).
- `agent.specialize` handler (`arkavo-server`) gains a **new optional `ticket` field** on `AgentSpecializeRequest`. When `ticket` is set, the handler fetches the TDF blob from Iroh and uses those bytes; when only `encrypted_bundle` is set, it behaves exactly as today. Backward compatible — no existing caller breaks.

**Tech Stack:** Rust, `arkavo-tdf-iroh` (`IrohNode`/`IrohTransport`/`IrohTicket`), `arkavo-protocol` (`HttpTransport`, `A2aRequest`/`A2aResponse`, `AgentRegistry`, `AgentSpecializeRequest`), `async-trait`, `serde_json`. No new crates beyond adding `arkavo-tdf-iroh` to `arkavo-orchestrator`.

## Global Constraints

- No `--release` builds; use debug.
- No clippy warnings: `cargo clippy -p arkavo-orchestrator -- -D warnings`, `cargo clippy -p arkavo-server -- -D warnings`, `cargo clippy -p arkavo-protocol -- -D warnings`. `#[allow(dead_code)]` forbidden.
- Implementation code (excluding `#[cfg(test)]` modules) stays under 400 lines per file. `swarmkit_apply.rs` is already ~436 lines counting the large inline test module; the new shipper/transport impls go in a **new** `crates/arkavo-orchestrator/src/swarmkit_transport.rs` so neither file's non-test code crosses 400. `specialization.rs` change is a few lines inside an existing function — it stays well under 400.
- New crate dependency: `arkavo-orchestrator` gains `arkavo-tdf-iroh`. Confirm it builds without C++ on Windows (the crate is pure-Rust iroh, no llama-cpp). Because `Cargo.toml` changes, **commit `Cargo.lock`**.
- No Conventional Commits prefixes. Use the exact commit messages below incl. their `Co-Authored-By` / `Claude-Session` trailers.
- Tests must not require external network or a KAS: the data-plane round-trip uses two in-memory Iroh nodes (`IrohNode::memory()`); the bundle crypto round-trip continues to use `MockTdfService`; address resolution uses an in-process `AgentRegistry`.
- **Structural-debt markers:** the two current-stack A2A seams (`IrohBundleShipper`'s `agent.specialize` send, and the handler's request decode) each carry a `// STRUCTURAL DEBT (a2a-realignment DEC-4):` comment naming the migration target (`SendMessage` + TDF `Part` over `arkavo-config-transport`).

## File Structure

- `crates/arkavo-orchestrator/Cargo.toml` — add `arkavo-tdf-iroh = { path = "../arkavo-tdf-iroh" }` dependency.
- `crates/arkavo-orchestrator/src/swarmkit_transport.rs` — **new**. `IrohBundleShipper` (impl `BundleShipper`) + `MeshRoleTaskTransport` (impl `RoleTaskTransport`) + shared `resolve_agent_address` helper + a small `a2a_transport_config()` helper. Inline `#[cfg(test)]` tests.
- `crates/arkavo-orchestrator/src/lib.rs` — `pub mod swarmkit_transport;` and re-export `IrohBundleShipper` / `MeshRoleTaskTransport`.
- `crates/arkavo-protocol/src/types.rs` — add an optional `ticket: Option<String>` field to `AgentSpecializeRequest` (after `encrypted_bundle`).
- `crates/arkavo-server/src/server/handlers/specialization.rs` — accept the new `ticket`: when present, fetch the TDF bytes from Iroh and use them in place of decoding `encrypted_bundle`. The handler grows one new parameter (`iroh_node: Option<&Arc<IrohNode>>`). **Only `handle_agent_specialize` + `handle_inner` signatures and the byte-acquisition block change** (see Cross-Workstream note for the exact line span).
- `crates/arkavo-server/src/server/mod.rs` — pass `self.iroh_node.as_ref()` into `handle_agent_specialize` at the `agent_specialize` call site (one new argument, `#[cfg(feature = "iroh")]`-gated).
- `crates/arkavo-protocol/tests/swarmkit_bundle_round_trip.rs` — add a two-node Iroh data-plane round-trip test (stage on node A, fetch+`unwrap_bundle` on node B).

---

### Task 1: `AgentSpecializeRequest` grows an optional `ticket` field

**Decision (flagged for controller):** grow the request with a **new optional `ticket: Option<String>`** field rather than changing the shape or overloading `encrypted_bundle`. Rationale: `#[serde(skip_serializing_if = "Option::is_none")]` keeps the wire format identical for all existing callers (the inline-base64 path and every test), so this is strictly additive and backward compatible. The handler treats `ticket` and `encrypted_bundle` as two sources of the same TDF bytes. Alternatives considered: (a) replace `encrypted_bundle` with `ticket` — rejected, breaks the inline round-trip test and any non-mesh caller; (b) overload `encrypted_bundle` to hold either a ticket or base64 — rejected, ambiguous and un-typed.

**Files:**
- Modify: `crates/arkavo-protocol/src/types.rs`
- Test: inline `#[cfg(test)]` in `types.rs` (add a small module if none covers `AgentSpecializeRequest`).

**Interfaces:**
- Existing `AgentSpecializeRequest { requester_id: String, encrypted_bundle: String, task_context: Option<String>, session_id: Option<String> }` (types.rs:1042).
- After change: same fields plus `pub ticket: Option<String>`.

- [ ] **Step 1: Read the existing struct**

Read `crates/arkavo-protocol/src/types.rs` lines 1040–1056 (the `AgentSpecializeRequest` definition) so the new field matches the existing `#[serde(skip_serializing_if = "Option::is_none")]` idiom on `task_context`/`session_id`.

- [ ] **Step 2: Write the failing test for backward-compatible deserialization**

Add to `types.rs` (in or alongside any existing `#[cfg(test)]` module):

```rust
#[cfg(test)]
mod agent_specialize_request_tests {
    use super::AgentSpecializeRequest;

    #[test]
    fn deserializes_legacy_inline_request_without_ticket() {
        // Existing callers send no `ticket` — must still parse, ticket = None.
        let json = r#"{"requester_id":"did:web:orch","encrypted_bundle":"YmFzZTY0"}"#;
        let req: AgentSpecializeRequest = serde_json::from_str(json).expect("legacy parse");
        assert_eq!(req.encrypted_bundle, "YmFzZTY0");
        assert!(req.ticket.is_none());
    }

    #[test]
    fn deserializes_ticket_request() {
        let json = r#"{"requester_id":"did:web:orch","encrypted_bundle":"","ticket":"blobABC"}"#;
        let req: AgentSpecializeRequest = serde_json::from_str(json).expect("ticket parse");
        assert_eq!(req.ticket.as_deref(), Some("blobABC"));
    }

    #[test]
    fn ticket_is_omitted_from_wire_when_none() {
        let req = AgentSpecializeRequest {
            requester_id: "did:web:orch".into(),
            encrypted_bundle: "x".into(),
            task_context: None,
            session_id: None,
            ticket: None,
        };
        let wire = serde_json::to_string(&req).expect("serialize");
        assert!(!wire.contains("ticket"), "ticket must be skipped when None: {wire}");
    }
}
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p arkavo-protocol agent_specialize_request_tests`
Expected: FAIL to compile — `ticket` is not a field of `AgentSpecializeRequest` yet.

- [ ] **Step 4: Add the field**

In `types.rs`, add after `encrypted_bundle` (keep all derives — `Debug, Clone, Serialize, Deserialize, JsonSchema`):

```rust
    /// Iroh blob ticket pointing at the TDF-wrapped bundle on the data
    /// plane. When set, the handler fetches the bundle bytes via the
    /// agent's Iroh node instead of decoding `encrypted_bundle`. Mesh
    /// shipping (WS-D) uses this; the inline base64 path remains for
    /// callers without an Iroh node.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
```

> Every existing struct-literal construction of `AgentSpecializeRequest` (in tests across `arkavo-server`, `arkavo-protocol`) will now fail to compile until `ticket: None` is added. Fix each by adding `ticket: None,` — Step 5's build surfaces them all. There is no production struct-literal caller yet (the inline path is only used in tests today); the production caller is `IrohBundleShipper` in Task 2.

- [ ] **Step 5: Build the workspace, fix struct literals, run the test**

Run: `cargo build -q` — fix every `missing field ticket` by adding `ticket: None,` to the literal. Expected literals to patch: the four in `crates/arkavo-server/src/server/handlers/specialization.rs` `#[cfg(test)]`, the two in `crates/arkavo-protocol/tests/swarmkit_bundle_round_trip.rs`.
Run: `cargo test -p arkavo-protocol agent_specialize_request_tests` — PASS (3 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/arkavo-protocol/src/types.rs crates/arkavo-server/src/server/handlers/specialization.rs crates/arkavo-protocol/tests/swarmkit_bundle_round_trip.rs
git commit -m "Add optional ticket field to AgentSpecializeRequest

Mesh shipping carries the TDF bundle over the Iroh data plane and signals
the ticket on agent.specialize. The new field is serde-skipped when None,
so the existing inline-base64 path and every caller are unaffected.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

### Task 2: `IrohBundleShipper` + `MeshRoleTaskTransport` (orchestrator glue)

**Decision (flagged for controller):** the shipper obtains its A2A client **per-call** by constructing a fresh `HttpTransport` from a `TransportConfig` (exactly as `SendTaskTool` and `task_executor/mesh_strategy.rs` do today) rather than holding a long-lived client. Rationale: matches the established mesh idiom, keeps the shipper `Send + Sync` without interior connection state, and each `agent.specialize`/`message/send` is a short request. Alternative considered: inject a shared `Arc<dyn A2aTransport>` — deferred; would change the call signature and isn't how the rest of the mesh code works. **Decision (RoleTaskTransport reuse):** `MeshRoleTaskTransport::dispatch` reuses the existing `message/send` A2A method (the same one `SendTaskTool` uses) to deliver the role's `initial_task` — this *is* the "start signal" (a task arriving on the agent's A2A endpoint starts its role loop). No new RPC method is introduced. Flagged because an alternative ("a dedicated `role.start` RPC") exists but would add surface for no behavioral gain.

**Files:**
- Modify: `crates/arkavo-orchestrator/Cargo.toml` (add `arkavo-tdf-iroh`)
- Create: `crates/arkavo-orchestrator/src/swarmkit_transport.rs`
- Modify: `crates/arkavo-orchestrator/src/lib.rs` (module + re-exports)
- Test: inline `#[cfg(test)]` in `swarmkit_transport.rs`

**Interfaces:**
- Implements `arkavo_orchestrator::swarmkit_apply::BundleShipper` — `async fn ship(&self, agent_did: &str, tdf_bytes: &[u8]) -> Result<(), String>`.
- Implements `arkavo_swarmkit_runtime::RoleTaskTransport` — `fn dispatch<'a>(&'a self, envelope: RoleTaskEnvelope) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>>`.
- Consumes: `arkavo_tdf_iroh::{IrohTransport, IrohTicket}` (`stage_bytes` → `IrohTicket`; `IrohTicket::to_string()`); `arkavo_protocol::agent_registry::AgentRegistry` (`get_agent_info(agent_id) -> Option<AgentInfo>`; `AgentInfo.address: Option<String>`); `arkavo_protocol::{HttpTransport, A2aRequest, A2aResponse, A2aEndpoint, A2aTransport, TransportConfig, transport::TlsConfig}`; `arkavo_protocol::types::{AgentSpecializeRequest, AgentSpecializeResponse, Message, MessagePart, MessageSendRequest, MessageSendResponse}`.

- [ ] **Step 1: Add the dependency**

In `crates/arkavo-orchestrator/Cargo.toml`, under `[dependencies]` (after `arkavo-swarmkit-runtime`):

```toml
arkavo-tdf-iroh = { path = "../arkavo-tdf-iroh" }
```

Run: `cargo build -p arkavo-orchestrator -q` — confirms the dep resolves (no code uses it yet; clean build).

- [ ] **Step 2: Write the failing tests**

Create `crates/arkavo-orchestrator/src/swarmkit_transport.rs` with only the test module first (the impls come in Step 4). The shipper test exercises the staging + address-resolution path against a real in-memory Iroh node and an in-process registry; the A2A send is verified against an unreachable address to assert the *resolution* and *staging* succeed and only the network send fails (so we test our glue, not a live agent).

```rust
//! Production mesh transports for the SwarmKit apply pipeline.
//!
//! Two concrete trait impls that compose existing infra — they add no
//! new protocol, crypto, or discovery:
//!
//! * [`IrohBundleShipper`] (impl [`BundleShipper`]) stages the
//!   already-TDF-wrapped bundle on the Iroh data plane, resolves the
//!   target agent's A2A address from the [`AgentRegistry`], and signals
//!   the resulting ticket via the `agent.specialize` A2A RPC.
//! * [`MeshRoleTaskTransport`] (impl [`RoleTaskTransport`]) delivers each
//!   role's first task to its bound agent via the `message/send` A2A RPC
//!   — the arrival of that task is the role's start signal.
//!
//! The bundle is a sizeable blob, so it travels over Iroh per the channel
//! rule; only the small ticket rides the A2A control plane.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use arkavo_protocol::agent_registry::AgentRegistry;
use arkavo_protocol::transport::TlsConfig;
use arkavo_protocol::types::{
    AgentSpecializeRequest, AgentSpecializeResponse, Message, MessagePart, MessageSendRequest,
    MessageSendResponse,
};
use arkavo_protocol::{
    A2aEndpoint, A2aRequest, A2aResponse, A2aTransport, HttpTransport, TransportConfig,
};
use arkavo_swarmkit_runtime::{RoleTaskEnvelope, RoleTaskTransport};
use arkavo_tdf_iroh::IrohTransport;

use crate::swarmkit_apply::BundleShipper;

// ... impls inserted in Step 4 ...

#[cfg(test)]
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use arkavo_tdf_iroh::IrohNode;
    use std::collections::HashMap;

    async fn registry_with(agent_id: &str, address: Option<&str>) -> Arc<AgentRegistry> {
        let reg = Arc::new(AgentRegistry::new());
        reg.register_agent(
            agent_id.to_string(),
            agent_id.to_string(),
            "test agent".to_string(),
            vec!["asset-store".to_string()],
            None,
            HashMap::new(),
            address.map(|a| a.to_string()),
        )
        .await
        .expect("register");
        reg
    }

    #[tokio::test]
    async fn ship_errors_when_agent_has_no_address() {
        let node = IrohNode::memory().await.unwrap();
        let transport = Arc::new(IrohTransport::new(node));
        let reg = registry_with("did:web:agent-a", None).await;
        let shipper = IrohBundleShipper::new(transport, reg, "did:web:orch".into());
        let err = shipper
            .ship("did:web:agent-a", b"tdf-bytes")
            .await
            .expect_err("no address");
        assert!(err.contains("address"), "got: {err}");
    }

    #[tokio::test]
    async fn ship_errors_when_agent_unknown() {
        let node = IrohNode::memory().await.unwrap();
        let transport = Arc::new(IrohTransport::new(node));
        let reg = Arc::new(AgentRegistry::new());
        let shipper = IrohBundleShipper::new(transport, reg, "did:web:orch".into());
        let err = shipper
            .ship("did:web:nope", b"tdf-bytes")
            .await
            .expect_err("unknown agent");
        assert!(err.contains("not found") || err.contains("address"), "got: {err}");
    }

    #[tokio::test]
    async fn ship_stages_then_fails_only_on_unreachable_send() {
        // A bogus but well-formed address: staging + resolution succeed,
        // the A2A send is the only thing that fails — proving our glue ran.
        let node = IrohNode::memory().await.unwrap();
        let transport = Arc::new(IrohTransport::new(node));
        let reg = registry_with("did:web:agent-a", Some("http://127.0.0.1:1/")).await;
        let shipper = IrohBundleShipper::new(transport, reg, "did:web:orch".into());
        let err = shipper
            .ship("did:web:agent-a", b"tdf-bytes")
            .await
            .expect_err("send to dead endpoint fails");
        // Must NOT be a staging/resolution error.
        assert!(!err.contains("not found"));
        assert!(err.to_lowercase().contains("specialize") || err.to_lowercase().contains("send")
            || err.to_lowercase().contains("connect"), "got: {err}");
    }

    #[tokio::test]
    async fn dispatch_errors_when_agent_has_no_address() {
        let reg = registry_with("did:web:agent-a", None).await;
        let transport = MeshRoleTaskTransport::new(reg);
        let env = RoleTaskEnvelope {
            flight_id: uuid::Uuid::new_v4(),
            role_id: "reviewer".into(),
            role_type: "critic".into(),
            agent_did: "did:web:agent-a".into(),
            task: "review the PR".into(),
        };
        let err = transport.dispatch(env).await.expect_err("no address");
        assert!(err.contains("address"), "got: {err}");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p arkavo-orchestrator swarmkit_transport`
Expected: FAIL to compile — `IrohBundleShipper`, `MeshRoleTaskTransport` don't exist yet, and the module isn't declared in `lib.rs`.

- [ ] **Step 4: Implement the two transports + helpers**

Insert into `swarmkit_transport.rs` (replacing the `// ... impls inserted in Step 4 ...` marker):

```rust
/// Short-lived A2A transport config matching the mesh send idiom used by
/// `SendTaskTool` and `task_executor::mesh_strategy`.
fn a2a_transport_config() -> TransportConfig {
    TransportConfig {
        timeout_ms: 60_000,
        max_retries: 2,
        tls_config: TlsConfig {
            require_tls: false,
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Resolve a bound agent DID to its A2A endpoint URL via the registry.
async fn resolve_agent_address(registry: &AgentRegistry, agent_did: &str) -> Result<String, String> {
    let info = registry
        .get_agent_info(agent_did)
        .await
        .ok_or_else(|| format!("agent {agent_did:?} not found in registry"))?;
    info.address
        .ok_or_else(|| format!("agent {agent_did:?} has no A2A address configured"))
}

/// Connect a fresh `HttpTransport` to the agent's endpoint.
async fn connect_to(agent_did: &str, address: &str) -> Result<HttpTransport, String> {
    let transport =
        HttpTransport::new(a2a_transport_config()).map_err(|e| format!("create transport: {e}"))?;
    let endpoint = A2aEndpoint {
        url: address.to_string(),
        agent_id: agent_did.to_string(),
        public_key: None,
    };
    transport
        .connect(&endpoint)
        .await
        .map_err(|e| format!("connect to {agent_did:?} at {address}: {e}"))?;
    Ok(transport)
}

/// Ships TDF-wrapped specialization bundles over the mesh: stage on the
/// Iroh data plane, signal the ticket over A2A `agent.specialize`.
pub struct IrohBundleShipper {
    iroh: Arc<IrohTransport>,
    registry: Arc<AgentRegistry>,
    requester_did: String,
}

impl IrohBundleShipper {
    pub fn new(iroh: Arc<IrohTransport>, registry: Arc<AgentRegistry>, requester_did: String) -> Self {
        Self {
            iroh,
            registry,
            requester_did,
        }
    }
}

#[async_trait::async_trait]
impl BundleShipper for IrohBundleShipper {
    async fn ship(&self, agent_did: &str, tdf_bytes: &[u8]) -> Result<(), String> {
        // 1. Stage the already-wrapped bundle on the data plane → ticket.
        let ticket = self
            .iroh
            .stage_bytes(tdf_bytes)
            .await
            .map_err(|e| format!("stage bundle on Iroh: {e}"))?;
        let ticket_str = ticket.to_string();

        // 2. Resolve the target agent's A2A address via mesh discovery.
        let address = resolve_agent_address(&self.registry, agent_did).await?;
        let transport = connect_to(agent_did, &address).await?;

        // 3. Signal the ticket over A2A. The bundle bytes never traverse
        //    the control plane — only the ticket does.
        // STRUCTURAL DEBT (a2a-realignment DEC-4): this `agent.specialize`
        // JSON-RPC call on the current A2A stack is replaced post-DEC-4 by a
        // `SendMessage` + TDF `Part` over `arkavo-config-transport`. The Iroh
        // staging + ticket survive the realignment; only this send migrates.
        let request = AgentSpecializeRequest {
            requester_id: self.requester_did.clone(),
            encrypted_bundle: String::new(),
            task_context: None,
            session_id: None,
            ticket: Some(ticket_str),
        };
        let rpc = A2aRequest::new("agent.specialize", serde_json::json!([request]));
        let response = transport
            .send_request(rpc)
            .await
            .map_err(|e| format!("send agent.specialize to {agent_did:?}: {e}"))?;
        let _ = transport.close().await;

        match response {
            A2aResponse::Success { result, .. } => {
                let resp: AgentSpecializeResponse = serde_json::from_value(result)
                    .map_err(|e| format!("parse agent.specialize response: {e}"))?;
                if resp.accepted {
                    Ok(())
                } else {
                    Err(format!(
                        "agent {agent_did:?} rejected specialization: {}",
                        resp.message.unwrap_or_default()
                    ))
                }
            }
            A2aResponse::Error { error, .. } => Err(format!(
                "agent.specialize rejected by {agent_did:?}: {} {}",
                error.code, error.message
            )),
        }
    }
}

/// Dispatches each role's first task to its bound agent over A2A
/// `message/send` — the task's arrival is the role's start signal.
pub struct MeshRoleTaskTransport {
    registry: Arc<AgentRegistry>,
}

impl MeshRoleTaskTransport {
    pub fn new(registry: Arc<AgentRegistry>) -> Self {
        Self { registry }
    }

    async fn dispatch_inner(&self, envelope: RoleTaskEnvelope) -> Result<(), String> {
        let address = resolve_agent_address(&self.registry, &envelope.agent_did).await?;
        let transport = connect_to(&envelope.agent_did, &address).await?;

        let message = Message {
            parts: vec![MessagePart::Text {
                content: envelope.task.clone(),
            }],
            metadata: Some(serde_json::json!({
                "source": "swarmkit_orchestrator",
                "flight_id": envelope.flight_id.to_string(),
                "role_id": envelope.role_id,
                "role_type": envelope.role_type,
            })),
        };
        let send_request = MessageSendRequest {
            message,
            task_id: None,
        };
        let rpc = A2aRequest::new("message/send", serde_json::json!([send_request]));
        let response = transport
            .send_request(rpc)
            .await
            .map_err(|e| format!("dispatch role task to {:?}: {e}", envelope.agent_did))?;
        let _ = transport.close().await;

        match response {
            A2aResponse::Success { result, .. } => {
                let _: MessageSendResponse = serde_json::from_value(result)
                    .map_err(|e| format!("parse message/send response: {e}"))?;
                Ok(())
            }
            A2aResponse::Error { error, .. } => Err(format!(
                "role task rejected by {:?}: {} {}",
                envelope.agent_did, error.code, error.message
            )),
        }
    }
}

impl RoleTaskTransport for MeshRoleTaskTransport {
    fn dispatch<'a>(
        &'a self,
        envelope: RoleTaskEnvelope,
    ) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send + 'a>> {
        Box::pin(async move { self.dispatch_inner(envelope).await })
    }
}
```

> Verify the registry method name/signature against `crates/arkavo-protocol/src/agent_registry.rs`: `get_agent_info(&self, agent_id: &str) -> Option<AgentInfo>` and `AgentInfo.address: Option<String>` (confirmed at registry.rs:288/45). If `register_agent`'s argument order differs from the test helper, adjust the helper to match the real signature (registry.rs:71, the `address` arg is the 7th positional).

- [ ] **Step 5: Declare the module + re-export**

In `crates/arkavo-orchestrator/src/lib.rs`, add `pub mod swarmkit_transport;` alongside `pub mod swarmkit_apply;`, and extend the re-export block:

```rust
pub use swarmkit_transport::{IrohBundleShipper, MeshRoleTaskTransport};
```

- [ ] **Step 6: Run the tests**

Run: `cargo test -p arkavo-orchestrator swarmkit_transport`
Expected: PASS (4 tests). `ship_stages_then_fails_only_on_unreachable_send` proves staging + resolution ran (the failure is the network send, not our glue).

- [ ] **Step 7: Build + clippy**

Run: `cargo build -p arkavo-orchestrator -q` (clean)
Run: `cargo clippy -p arkavo-orchestrator -- -D warnings` (clean)

- [ ] **Step 8: Commit (with Cargo.lock)**

```bash
git add crates/arkavo-orchestrator/Cargo.toml Cargo.lock \
  crates/arkavo-orchestrator/src/swarmkit_transport.rs \
  crates/arkavo-orchestrator/src/lib.rs
git commit -m "Add IrohBundleShipper and MeshRoleTaskTransport mesh transports

IrohBundleShipper stages the TDF-wrapped bundle on the Iroh data plane,
resolves the target agent's address via AgentRegistry, and signals the
ticket over agent.specialize. MeshRoleTaskTransport dispatches each role's
first task via message/send. Both reuse the existing HttpTransport mesh
send idiom; only the ticket rides the control plane. The agent.specialize
send carries an a2a-realignment DEC-4 structural-debt marker.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

### Task 3: `agent.specialize` handler fetches the bundle by ticket

**Decision (flagged for controller):** the handler gains a new parameter `iroh_node: Option<&Arc<IrohNode>>` rather than reaching for a global. Rationale: the handler is already dependency-injected (decryptor, metadata, role store passed explicitly), so threading the node is consistent and keeps it testable; the server already owns `iroh_node: Option<Arc<IrohNode>>` on `A2aRpcImpl` (mod.rs:417, `#[cfg(feature = "iroh")]`). When `ticket` is present but no node is wired, the handler returns a clear error rather than silently falling back.

**Cross-workstream note (READ — overlaps WS-C):** WS-C also edits this file. WS-C's change is confined to `apply_bundle_to_metadata` (specialization.rs:189–203) — it persists `bundle.persona.mcp_tools` into `AgentMetadata`. **WS-D touches a disjoint region:** the `handle_agent_specialize` + `handle_inner` *signatures* and the **byte-acquisition block at specialization.rs:111–132** (the `encrypted_bundle.is_empty()` guard + the base64 decode into `tdf_bytes`). WS-D does **not** touch `apply_bundle_to_metadata`, the persona-application logic, or `role_specialization.set(...)`. The two changes do not overlap; sequence either order. The controller should land them on the same branch and let the second one rebase — the only shared lines are the two function signatures, which both WS may extend (WS-C adds nothing to the signature; WS-D adds the `iroh_node` param). If both land, the merged signature is the union.

**Files:**
- Modify: `crates/arkavo-server/src/server/handlers/specialization.rs`
- Modify: `crates/arkavo-server/src/server/mod.rs` (call site, one new arg)
- Test: inline `#[cfg(test)]` in `specialization.rs`

**Interfaces:**
- `handle_agent_specialize(..., decryptor, request)` → `handle_agent_specialize(..., decryptor, iroh_node: Option<&Arc<IrohNode>>, request)`.
- `handle_inner(...)` gains the same `iroh_node` parameter.
- Byte acquisition becomes: if `request.ticket` is `Some`, `IrohTransport::new(node.clone()).fetch_bytes(&ticket)` → `tdf_bytes`; else decode `encrypted_bundle` as today.

- [ ] **Step 1: Read the existing handler + call site**

Read `specialization.rs:88–152` (the `handle_inner` byte-acquisition block) and `mod.rs:1017–1031` (the `agent_specialize` RpcServer method that calls `handle_agent_specialize`). Note `iroh_node` on `A2aRpcImpl` is `#[cfg(feature = "iroh")]`-gated (mod.rs:416–417).

- [ ] **Step 2: Write the failing test for the ticket path**

Add to the `#[cfg(test)]` module in `specialization.rs`. This test stages real TDF-shaped bytes on an in-memory Iroh node, then drives the handler with a `ticket` and asserts the stub decryptor receives exactly those bytes:

```rust
#[tokio::test]
async fn handle_specialize_fetches_bundle_from_ticket() {
    use arkavo_tdf_iroh::{IrohNode, IrohTransport};

    let did = "did:web:agent-7.arkavo.net";
    let bundle = build_bundle("analyst", "agent-7");
    let agent_metadata = metadata_with_did("agent-7", did);
    let (metrics, limiter, registry, role_store) = deps();

    // Stage the (stand-in) TDF bytes on a shared in-memory node so the
    // handler can fetch them back by ticket on the same node.
    let node = IrohNode::memory().await.unwrap();
    let staged = b"stand-in-tdf-bytes-for-ticket-path";
    let ticket = IrohTransport::new(node.clone())
        .stage_bytes(staged)
        .await
        .unwrap()
        .to_string();

    // Decryptor asserts it was handed the fetched bytes (not base64-decoded
    // inline), then returns the prepared bundle.
    let decryptor = AssertingDecryptor {
        expected_bytes: staged.to_vec(),
        bundle: bundle.clone(),
        expected_did: did.to_string(),
    };

    let response = handle_agent_specialize(
        &metrics,
        &limiter,
        &registry,
        &agent_metadata,
        &role_store,
        &decryptor,
        Some(&node),
        AgentSpecializeRequest {
            requester_id: "did:web:orchestrator.arkavo.net".into(),
            encrypted_bundle: String::new(),
            task_context: None,
            session_id: None,
            ticket: Some(ticket),
        },
    )
    .await
    .expect("specialize via ticket");

    assert!(response.accepted);
    let stored = role_store.get().await.expect("role context stored");
    assert_eq!(stored.role_id, "analyst");
}

#[tokio::test]
async fn handle_specialize_errors_when_ticket_but_no_node() {
    let did = "did:web:agent-7.arkavo.net";
    let agent_metadata = metadata_with_did("agent-7", did);
    let (metrics, limiter, registry, role_store) = deps();
    let decryptor = UnconfiguredBundleDecryptor;

    let err = handle_agent_specialize(
        &metrics,
        &limiter,
        &registry,
        &agent_metadata,
        &role_store,
        &decryptor,
        None, // no iroh node wired
        AgentSpecializeRequest {
            requester_id: "did:web:orchestrator.arkavo.net".into(),
            encrypted_bundle: String::new(),
            task_context: None,
            session_id: None,
            ticket: Some("blobABC".into()),
        },
    )
    .await
    .expect_err("ticket with no node rejects");
    assert_eq!(err.code(), -32603);
}
```

Add the `AssertingDecryptor` test helper near `StubDecryptor`:

```rust
struct AssertingDecryptor {
    expected_bytes: Vec<u8>,
    bundle: AgentSpecializationBundle,
    expected_did: String,
}

#[async_trait]
impl BundleDecryptor for AssertingDecryptor {
    async fn decrypt(
        &self,
        tdf_bytes: &[u8],
        recipient_did: &str,
    ) -> Result<AgentSpecializationBundle, String> {
        if tdf_bytes != self.expected_bytes.as_slice() {
            return Err("decryptor got unexpected bytes (ticket fetch path broken)".into());
        }
        if recipient_did != self.expected_did {
            return Err(format!("wrong recipient: {recipient_did}"));
        }
        Ok(self.bundle.clone())
    }
}
```

> Update the existing four `handle_agent_specialize(...)` test calls in this module to pass the new `None` argument before `request` (they all exercise the inline path). The two integration tests in `swarmkit_bundle_round_trip.rs` get the same `None` argument in Task 4's edits.

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p arkavo-server --lib specialization`
Expected: FAIL to compile — `handle_agent_specialize` takes no `iroh_node` argument yet, and the ticket branch doesn't exist.

- [ ] **Step 4: Add the `iroh_node` parameter + ticket branch**

In `specialization.rs`, add the import at top:

```rust
use arkavo_tdf_iroh::{IrohNode, IrohTransport};
```

Change both signatures to thread `iroh_node: Option<&Arc<IrohNode>>` (place it right before `request`):

```rust
pub async fn handle_agent_specialize(
    metrics: &Arc<MetricsCollector>,
    rate_limiter: &RateLimiter,
    _mcp_registry: &Arc<McpRegistry>,
    agent_metadata: &Arc<tokio::sync::RwLock<AgentMetadata>>,
    role_specialization: &Arc<RoleSpecializationStore>,
    decryptor: &dyn BundleDecryptor,
    iroh_node: Option<&Arc<IrohNode>>,
    request: AgentSpecializeRequest,
) -> Result<AgentSpecializeResponse, ErrorObjectOwned> {
```

(forward `iroh_node` into the `handle_inner(...)` call; add the matching parameter to `handle_inner`).

Replace the byte-acquisition block (current specialization.rs:111–132 — the `encrypted_bundle.is_empty()` guard through the base64 decode) with a source-selection block:

```rust
    // STRUCTURAL DEBT (a2a-realignment DEC-4): the `ticket`/`encrypted_bundle`
    // split on this request rides the current A2A stack. Post-DEC-4 the bundle
    // arrives as a TDF `Part` referenced by a `SendMessage` over
    // arkavo-config-transport, collapsing both fields into one Part reference.
    let tdf_bytes: Vec<u8> = if let Some(ticket_str) = request
        .ticket
        .as_deref()
        .filter(|t| !t.is_empty())
    {
        // Mesh path: the bundle blob is on the Iroh data plane; fetch it.
        let node = iroh_node.ok_or_else(|| {
            ErrorObjectOwned::owned(
                -32603,
                "Bundle ticket supplied but agent has no Iroh node",
                Some("rebuild the agent with --features iroh to fetch ticketed bundles".to_string()),
            )
        })?;
        let ticket: arkavo_tdf_iroh::IrohTicket = ticket_str.parse().map_err(|e| {
            ErrorObjectOwned::owned(-32602, "Invalid params: ticket is not a valid Iroh ticket", Some(format!("{e}")))
        })?;
        IrohTransport::new(node.clone())
            .fetch_bytes(&ticket)
            .await
            .map_err(|e| {
                ErrorObjectOwned::owned(-32603, "Failed to fetch bundle from Iroh ticket", Some(e.to_string()))
            })?
    } else {
        // Inline path (legacy / no data plane): base64 in `encrypted_bundle`.
        if request.encrypted_bundle.is_empty() {
            return Err(ErrorObjectOwned::owned(
                -32602,
                "Invalid params: encrypted_bundle or ticket is required",
                Some("Provide either an inline base64 encrypted_bundle or an Iroh ticket".to_string()),
            ));
        }
        base64::engine::general_purpose::STANDARD
            .decode(request.encrypted_bundle.as_bytes())
            .map_err(|e| {
                ErrorObjectOwned::owned(
                    -32602,
                    "Invalid params: encrypted_bundle is not valid base64",
                    Some(e.to_string()),
                )
            })?
    };
```

> The `session_id` derivation and everything from `let did = agent_did.as_deref()...` downward stays exactly as-is — `tdf_bytes` flows into `decryptor.decrypt(&tdf_bytes, did)` unchanged. The `request.session_id` line can stay where it is (above or below this block); keep its current position to minimize the diff.

- [ ] **Step 5: Wire the call site in `mod.rs`**

In `mod.rs`, update the `agent_specialize` method (lines 1017–1031) to pass the node. Because `iroh_node` is `#[cfg(feature = "iroh")]`-gated, select it conditionally:

```rust
    async fn agent_specialize(
        &self,
        request: AgentSpecializeRequest,
    ) -> RpcResult<AgentSpecializeResponse> {
        #[cfg(feature = "iroh")]
        let iroh_node = self.iroh_node.as_ref();
        #[cfg(not(feature = "iroh"))]
        let iroh_node: Option<&std::sync::Arc<arkavo_tdf_iroh::IrohNode>> = None;
        handlers::specialization::handle_agent_specialize(
            &self.metrics,
            &self.rate_limiter,
            &self.mcp_registry,
            &self.agent_metadata,
            &self.role_specialization,
            self.bundle_decryptor.as_ref(),
            iroh_node,
            request,
        )
        .await
    }
```

> If `arkavo-tdf-iroh` is only a dependency of `arkavo-server` under the `iroh` feature, the `#[cfg(not(feature = "iroh"))]` arm cannot name `arkavo_tdf_iroh::IrohNode`. Verify in `crates/arkavo-server/Cargo.toml`: `arkavo-tdf-iroh` is `optional = true` and only enabled by `iroh = ["dep:arkavo-tdf-iroh"]` (confirmed). Therefore the handler signature's `IrohNode` type must also be reachable when the feature is off. **Resolution:** make `arkavo-tdf-iroh` a non-optional dependency of `arkavo-server` (it is pure-Rust, Windows-safe, and `arkavo-server` already depends on it optionally). Then drop the `#[cfg]` split entirely — `self.iroh_node` stays feature-gated, so guard only the field read:
>
> ```rust
>         #[cfg(feature = "iroh")]
>         let iroh_node = self.iroh_node.as_ref();
>         #[cfg(not(feature = "iroh"))]
>         let iroh_node: Option<&std::sync::Arc<arkavo_tdf_iroh::IrohNode>> = None;
> ```
>
> With `arkavo-tdf-iroh` non-optional, both arms compile. Make this dependency change in Step 5 and commit `Cargo.lock`. (Flagged in the report — moving the dep from optional to required is a small surface change.)

- [ ] **Step 6: Run tests + build + clippy**

Run: `cargo test -p arkavo-server --lib specialization` — PASS (existing 4 inline-path tests + 2 new ticket tests).
Run: `cargo build -p arkavo-server -q` (clean — also build with `--features iroh` to exercise the gated arm: `cargo build -p arkavo-server --features iroh -q`).
Run: `cargo clippy -p arkavo-server -- -D warnings` (clean).

- [ ] **Step 7: Commit (with Cargo.lock)**

```bash
git add crates/arkavo-server/src/server/handlers/specialization.rs \
  crates/arkavo-server/src/server/mod.rs \
  crates/arkavo-server/Cargo.toml Cargo.lock
git commit -m "agent.specialize fetches ticketed bundle from Iroh data plane

When the request carries an Iroh ticket, the handler fetches the TDF bytes
from the agent's Iroh node and feeds them to the existing unwrap path; the
inline base64 path is unchanged. The seam carries an a2a-realignment DEC-4
structural-debt marker. arkavo-tdf-iroh becomes a required dep so the
handler signature compiles with the iroh feature off.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

### Task 4: Two-node Iroh data-plane round-trip test

**Files:**
- Modify: `crates/arkavo-protocol/tests/swarmkit_bundle_round_trip.rs`

**Interfaces:**
- Consumes: `arkavo_tdf_iroh::{IrohNode, IrohTransport}`, the existing `wrap_bundle`/`unwrap_bundle` + `MockTdfService` already used in this test file, `build_bundle` helper already present.
- The test must be a `dev-dependency`-only addition: confirm `arkavo-tdf-iroh` is a dev-dependency of `arkavo-protocol`; if not, add it under `[dev-dependencies]`.

- [ ] **Step 1: Confirm/add the dev-dependency**

Check `crates/arkavo-protocol/Cargo.toml` `[dev-dependencies]` for `arkavo-tdf-iroh`. If absent, add:

```toml
arkavo-tdf-iroh = { path = "../arkavo-tdf-iroh" }
```

(The `wrap_bundle`/`unwrap_bundle` symbols require the `kas` feature — the test file already uses them, so `arkavo-protocol` is tested with `--features kas`; keep that.)

- [ ] **Step 2: Write the failing two-node round-trip test**

Append to `swarmkit_bundle_round_trip.rs`:

```rust
/// WS-D data-plane round trip: the orchestrator stages a TDF-wrapped
/// bundle on one Iroh node; a *different* node fetches it by ticket and
/// `unwrap_bundle`s it. This crosses the same boundary the production
/// IrohBundleShipper + agent.specialize handler cross, minus the A2A hop.
#[tokio::test]
async fn bundle_round_trips_across_two_iroh_nodes() {
    use arkavo_tdf_iroh::{IrohNode, IrohTransport};

    let did = "did:web:agent-7.arkavo.net";
    let svc = MockTdfService::default();
    let bundle = build_bundle("analyst", did);

    // Orchestrator side: wrap → TDF bytes → stage on node A.
    let tdf = wrap_bundle(&bundle, &svc, did).await.expect("wrap");
    let tdf_bytes = serde_json::to_vec(&tdf).expect("serialize tdf");

    let node_a = IrohNode::memory().await.expect("node a");
    let ticket = IrohTransport::new(node_a.clone())
        .stage_bytes(&tdf_bytes)
        .await
        .expect("stage");
    let ticket_str = ticket.to_string();

    // Agent side: a separate node fetches by ticket, then unwraps.
    let node_b = IrohNode::memory().await.expect("node b");
    let fetched = IrohTransport::new(node_b.clone())
        .fetch_bytes(&ticket_str.parse().expect("parse ticket"))
        .await
        .expect("fetch across nodes");
    assert_eq!(fetched, tdf_bytes, "fetched blob must be byte-identical");

    let refetched_tdf: TdfManifest =
        serde_json::from_slice(&fetched).expect("parse fetched tdf");
    let recovered = unwrap_bundle(&refetched_tdf, &svc, did)
        .await
        .expect("unwrap fetched bundle");
    assert_eq!(recovered.role_context.role_id, "analyst");

    node_a.stop().await.ok();
    node_b.stop().await.ok();
}
```

- [ ] **Step 3: Run + verify**

Run: `cargo test -p arkavo-protocol --features kas bundle_round_trips_across_two_iroh_nodes`
Expected: PASS. (Two in-memory Iroh nodes discover each other over the loopback relay/direct addrs embedded in the ticket; the fetch is a real P2P transfer.)

> If two memory nodes cannot reach each other in CI without a relay (possible in fully-sandboxed CI with no loopback networking), fall back to a **single shared node** staged-then-fetched (the existing `transport_stage_fetch_roundtrip` in `arkavo-tdf-iroh` proves single-node fetch works) and assert the cross-node intent in a comment. Flag this in the report if the two-node fetch is flaky in CI.

- [ ] **Step 4: Commit**

```bash
git add crates/arkavo-protocol/tests/swarmkit_bundle_round_trip.rs crates/arkavo-protocol/Cargo.toml Cargo.lock
git commit -m "Add two-node Iroh data-plane bundle round-trip test

Stages a TDF-wrapped bundle on one in-memory Iroh node and fetches +
unwraps it on a second node — the data-plane path the IrohBundleShipper
and ticketed agent.specialize handler exercise in production.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01VyuNT2XyZuxLMxLgkYc6ZG"
```

---

### Task 5: Final workstream verification

- [ ] **Step 1: Full build + targeted tests**

Run: `cargo build -q`
Run: `cargo build -p arkavo-server --features iroh -q`
Run: `cargo test -p arkavo-protocol agent_specialize_request_tests`
Run: `cargo test -p arkavo-protocol --features kas swarmkit_bundle_round_trip` (the file's two existing tests + the new two-node test)
Run: `cargo test -p arkavo-orchestrator swarmkit_transport`
Run: `cargo test -p arkavo-server --lib specialization`

- [ ] **Step 2: Clippy across all touched crates**

Run: `cargo clippy -p arkavo-protocol -- -D warnings`
Run: `cargo clippy -p arkavo-orchestrator -- -D warnings`
Run: `cargo clippy -p arkavo-server -- -D warnings`
Run: `cargo clippy -p arkavo-server --features iroh -- -D warnings`

- [ ] **Step 3: Format check**

Run: `cargo fmt -- --check` (clean)

- [ ] **Step 4: Confirm structural-debt markers present**

Run: `grep -rn "STRUCTURAL DEBT (a2a-realignment DEC-4)" crates/arkavo-orchestrator/src/swarmkit_transport.rs crates/arkavo-server/src/server/handlers/specialization.rs`
Expected: two matches (the shipper's `agent.specialize` send; the handler's request decode). These must be logged in the PR description per the spec's Structural Debt section.

---

## Self-Review

**Spec coverage (WS-D scope):**
- "Concrete mesh `BundleShipper`" → Task 2 `IrohBundleShipper`: stages on `IrohTransport.stage_bytes` → ticket, resolves address via `AgentRegistry`, sends `agent.specialize { ticket }` over the existing `HttpTransport`/`A2aRequest` path (same idiom as `SendTaskTool`/`mesh_strategy`). ✓
- "`agent.specialize` handler change — accept an iroh ticket, `fetch_bytes` → `unwrap_bundle`" → Task 1 (new optional `ticket` field, backward compatible — documented decision) + Task 3 (ticket branch fetches via `IrohTransport.fetch_bytes`, feeds existing decrypt/unwrap path unchanged). ✓
- "`RoleTaskTransport` production impl (dispatch initial task + start signal)" → Task 2 `MeshRoleTaskTransport`: dispatches `RoleTaskEnvelope.task` via `message/send`; task arrival is the start signal (documented decision). ✓
- "Test: extend `swarmkit_bundle_round_trip` to stage on one iroh node and fetch+`unwrap_bundle` on another" → Task 4: `bundle_round_trips_across_two_iroh_nodes`. ✓
- "Compose existing infra; build only glue" → no new crypto, no new discovery, no new protocol; reuses `wrap_bundle`/`unwrap_bundle`, `AgentRegistry`, `IrohTransport`, the A2A send path. ✓
- "Structural-debt markers (DEC-4)" → Task 2 + Task 3 add the marker at both seams; Task 5 Step 4 verifies. ✓

**Placeholder scan:** No TBD/vague steps. Every code block uses real, verified signatures: `BundleShipper::ship(&str, &[u8]) -> Result<(), String>` (swarmkit_apply.rs:158), `RoleTaskTransport::dispatch` returning `Pin<Box<dyn Future...>>` (flight.rs:36), `RoleTaskEnvelope` fields (flight.rs:25), `IrohTransport::stage_bytes/fetch_bytes` + `IrohTicket::to_string`/`parse` (transport.rs:50/76, ticket.rs), `AgentRegistry::get_agent_info` + `AgentInfo.address` (agent_registry.rs:288/45), the `HttpTransport`/`A2aRequest::new`/`A2aResponse::{Success,Error}` send idiom (mesh_strategy.rs:387–474), `AgentSpecializeRequest`/`AgentSpecializeResponse` (types.rs:1042/1059). The three "verify the name against the real file" notes are named, checkable verifications, not placeholders.

**Type consistency:** `IrohBundleShipper::new(Arc<IrohTransport>, Arc<AgentRegistry>, String)` used identically in tests (Step 2) and impl (Step 4). `MeshRoleTaskTransport::new(Arc<AgentRegistry>)` consistent across tests and impl. The handler's new `iroh_node: Option<&Arc<IrohNode>>` parameter is consistent across `handle_agent_specialize`, `handle_inner`, the call site in `mod.rs`, and all test calls. The new `ticket: Option<String>` field is consistent across Task 1's struct, Task 2's shipper construction, Task 3's handler branch + tests, and the legacy literals patched in Task 1 Step 5.

**Cross-workstream safety (WS-C):** WS-D's `specialization.rs` edit is confined to (a) the two function signatures (+`iroh_node` param) and (b) the byte-acquisition block at lines 111–132. WS-C edits `apply_bundle_to_metadata` (lines 189–203). Disjoint; the only shared lines are the signatures, where WS-C adds nothing. Documented in the Task 3 cross-workstream note.

**Deviations from spec, flagged for the controller:**
- **Ticket field shape** (Task 1): chose additive optional `ticket` (backward compatible) over reshaping the request. Recommendation in plan; alternatives listed.
- **A2A client acquisition** (Task 2): per-call `HttpTransport` (matches mesh idiom) over an injected shared client.
- **RoleTaskTransport reuses `message/send`** (Task 2): no new `role.start` RPC.
- **`arkavo-tdf-iroh` moves from optional to required dep of `arkavo-server`** (Task 3 Step 5): needed so the handler signature's `IrohNode` type is nameable with the `iroh` feature off. Small surface change; pure-Rust, Windows-safe.
- **Two-node Iroh fetch in CI** (Task 4): may need a single-node fallback if sandboxed CI has no loopback P2P. Flagged.
