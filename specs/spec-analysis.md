# Spec Analysis: Conflicts, Gaps, and Optimizations

version: 0.67.1
date: 2026-03-19

## Conflicts

### Git commit convention contradiction

`git.spec.yaml` invariant states "Commit messages follow conventional format" but `CLAUDE.md` explicitly forbids conventional commits (`feat:`, `fix:`, etc.). The spec must be updated to match the project convention.

**Resolution**: Update `git.spec.yaml` invariant to reflect project-standard commit messages.

### Chat session state machine incomplete

`chat-session.spec.yaml` invariant defines states `Active -> Closing -> Closed`, but scenario CHAT-010 introduces a `Zombie` state for abnormal exits not listed in the invariant.

**Resolution**: Update the invariant to include the Zombie state.

### Token estimation claimed by two modules

`router.spec.yaml` ROUTER-016 and `context.spec.yaml` CTX-013 both describe token estimation, referencing different file paths. Only one implementation likely exists.

**Resolution**: Determine canonical location and update the other spec to reference it as a dependency rather than reimplementing.

### Module paths that don't match crate structure

| Spec | Module | Issue |
|---|---|---|
| `network-security.spec.yaml` | `arkavo_edge::security::network` | No `arkavo-edge` crate in `crates/` |
| `session-security.spec.yaml` | `arkavo_session::security` | No `security` submodule; files are top-level |
| `tdf-security.spec.yaml` | `arkavo_tdf::security` | No `security` submodule in `arkavo-tdf` |

**Resolution**: Update module paths to match actual crate structure. These are security cross-cutting specs that span multiple files within their respective crates.

### TDF-Iroh invariant wording

Invariant says "Transport handles raw bytes only, no encryption" but scenario IROH-008 describes encrypted TDF bytes flowing through Iroh. The invariant is technically correct (Iroh itself doesn't encrypt) but reads misleadingly.

**Resolution**: Clarify invariant: "Transport handles raw bytes; encryption occurs before handoff to Iroh."

## Overlaps

### Tool execution across 4 specs

Tool execution is described from different perspectives:
- `mcp-tools.spec.yaml` MCP-003: Tool registry execution
- `llm-core.spec.yaml` LLM-004: Tool calls from model output
- `chat-session.spec.yaml` CHAT-009: Session-level tool routing
- `mcp-runtime.spec.yaml` MCPR-003: Runtime with timeout/cancel

These are distinct layers (trait definition -> model parsing -> session routing -> runtime execution) but the boundaries could be documented more clearly in the index integration points.

### mDNS discovery in 3 specs

- `protocol.spec.yaml` PROTO-004
- `agui.spec.yaml` AGUI-002
- `mcp-mesh.spec.yaml` MESH-003/004

Each uses mDNS for different purposes (protocol discovery, GUI peer discovery, mesh agent listing) but the shared mDNS infrastructure is not consolidated.

### Health reporting in 3 specs

- `observability.spec.yaml` OBS-004
- `debugger.spec.yaml` DBG-004/005
- `cef.spec.yaml` CEF-003

Health checking has different scopes (system health, debug diagnostics, CEF command health) but could benefit from a shared health trait/interface.

### Rate limiting in 2 specs

- `protocol.spec.yaml` PROTO-007: Protocol-level rate limiting
- `network-security.spec.yaml` NET-010: Network-level per-IP rate limiting

The `arkavo-security` crate now has its own spec (`security.spec.yaml` SEC-003) that covers the governor-based implementation. The protocol and network specs should reference this as the authoritative rate limiting spec.

### Task orchestration / HRM boundary

`task-orchestration.spec.yaml` TASK-005 references `crates/arkavo-hrm/src/conductor.rs` directly. This crosses crate boundaries. The HRM spec should own conductor behavior; the task spec should describe the interaction as a dependency.

## Gaps (Addressed)

Ten new specs were created for crates previously missing coverage:

| New Spec | Crate | Scenarios |
|---|---|---|
| `agent-sdk.spec.yaml` | anthropic-agent-sdk | 5 |
| `agent.spec.yaml` | arkavo-agent | 5 |
| `evofabric.spec.yaml` | arkavo-evofabric | 8 |
| `kv-cache.spec.yaml` | arkavo-kv-cache | 5 |
| `mcp-traits.spec.yaml` | arkavo-mcp | 6 |
| `openclaw.spec.yaml` | arkavo-openclaw | 6 |
| `qr-registration.spec.yaml` | arkavo-registration | 5 |
| `security.spec.yaml` | arkavo-security | 6 |
| `server.spec.yaml` | arkavo-server | 10 |
| `validation.spec.yaml` | arkavo-validation | 7 |

Additionally, `task-orchestration.spec.yaml` was expanded with 4 new scenarios (TASK-009 through TASK-012) and its module path corrected from `arkavo_protocol::task_executor` to `arkavo_tasks`.

### Crates intentionally without specs

| Crate | Reason |
|---|---|
| `arkavo` | Thin binary wrapper; covered by `cli.spec.yaml` |
| `arkavo-bench` | Internal benchmarking infrastructure |
| `arkavo-llama-cpp` | FFI binding layer; behavior defined by upstream |
| `arkavo-llama-cpp-sys` | Auto-generated bindgen bindings |
| `arkavo-test-macros` | Test infrastructure proc macro |
| `arkavo-mcp-core` | Planned but unimplemented (README only) |

## Optimizations

### Consolidation candidates

**Config specs** (`config-bundle`, `config-encryption`, `config-transport`): These three specs have 4+5+4=13 scenarios and form a tight pipeline. They could be consolidated into a single `config.spec.yaml` if the crates are ever merged. For now, keep separate since they map to distinct crates.

**Security cross-cutting specs** (`network-security`, `session-security`, `tdf-security`, `security`): Four specs cover security from different angles. The new `security.spec.yaml` covers the `arkavo-security` crate. The other three cover security aspects of their parent domains. This is correct architecture (security is cross-cutting) but the relationship should be documented in index integration points.

### Missing integration points for index.yaml

These cross-component relationships exist but are not in the index:

- `Server → Agent`: Server manages agent lifecycle
- `Server → Learning Bus`: Server runs learning pipeline loops
- `Validation → Network Security`: Validation provides egress filter used by network security
- `Security → Protocol`: OAuth2/JWT used for protocol auth
- `MCP Traits → MCP Tools`: Tools implement the trait from arkavo-mcp
- `MCP Traits → MCP Runtime`: Runtime executes tools defined by trait
- `Agent SDK → MCP Traits`: SDK discovers and registers MCP tools
- `QR Registration → Crypto`: QR registration uses Ed25519 for challenge signing
- `KV Cache → Router`: Cache slots inform context for routing
- `EvoFabric → Crypto`: OpBundles signed with arkavo-crypto
- `OpenClaw → Protocol`: OpenClaw translates to A2A protocol
- `Validation → Session Security`: Log sanitization used in session security

### Stale file references

Several `refs` point to estimated line ranges that may have drifted. A CI check that validates `refs` paths (not line numbers) exist would prevent specs from becoming stale. Consider adding a simple check:

```bash
for ref in $(grep -oh 'crates/[^:]*' specs/arkavo-edge/*.spec.yaml | sort -u); do
  [ ! -e "$ref" ] && echo "STALE: $ref"
done
```

### Scenario count summary

After additions: **71 specs, 598 total scenarios** (was 61 specs, 531 scenarios).
