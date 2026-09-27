# Substrate trade study: torg circuits and WASM components

Decision record for issue #615. Status: **ratified** (pending review).

## Decision

Evolved capability in arkavo-edge is split across two substrates with a hard
boundary between them:

- **Decision plane — TØRG circuits.** Whether and when an action may happen
  is decided by torg boolean graphs, compiled to `CompiledCircuit` for
  nanosecond evaluation. This stays the only substrate for gating decisions.
- **Effect plane — WASM Component Model via wasmtime.** How an effectful
  capability does its work (parsing, transformation, API choreography) is the
  domain of evolved WASM components, executed under wasmtime with fuel and
  capability-scoped imports.

Every evolved capability is gated by a torg-verified circuit: the circuit
decides whether/when, the component decides only how. A component call that
the gating circuit denies never executes.

## Why the split

The verification power of torg circuits comes from their **inexpressiveness**.
A boolean graph over typed features has no loops, no allocation, no I/O — so
the verifier surface is real and total:

- `arkavo-sat` extracts CNF (Tseitin) and uses a CDCL solver to probe
  decision boundaries, find policy holes, and prove tautology/contradiction.
- `arkavo-sbe` layers circuits hierarchically (Invariant / Policy / Adaptive)
  with Ed25519-signed invariant contracts that lower layers cannot modify by
  construction, and bounded model checking on policy updates.
- `arkavo-torg-circuits` compiles graphs to pre-allocated buffers evaluated
  in nanoseconds — cheap enough to gate every request.

Generalizing circuits toward effectful capability would destroy exactly the
property that makes them verifiable. Conversely, effectful code needs a real
execution substrate; that substrate must be sandboxed, metered, and
deny-by-default. The WASM Component Model is the only candidate that gives
us all three plus a typed contract surface (WIT) and a production embedder
(wasmtime) with fuel/epoch interruption and per-instance import control.

## Alternatives considered

| Alternative | Rejected because |
|---|---|
| Extend torg with effectful nodes | Destroys SAT-verifiability — the whole value of the decision plane. |
| Native dynamic libraries (dlopen) | No sandbox, no metering, no capability scoping; a single evolved bug is process compromise. |
| Embedded scripting (Lua/Rhai/JS) | Weaker isolation than WASM, no typed contract surface, another language in the threat model. |
| Subprocess-per-capability | Sandbox possible but heavyweight (process per call), capability scoping via OS only, poor portability to the supported targets. |
| WASM core modules (no component model) | Loses typed interfaces (WIT) and structured imports; capability scoping degenerates to hand-rolled ABI conventions. |

## Version pinning

- **WIT contract format: WASI 0.2 worlds for v1.** WASI 0.3 (async, shipped
  February 2026) is adopted only after one wasmtime LTS cycle on 0.3 —
  contract churn is the main schedule risk for evolved-capability stability.
- wasmtime is pinned to an LTS release and upgraded deliberately, never
  floated.

## Security invariants

- **No hot evolved code.** Components execute only as members of a signed,
  TDF-wrapped kit. The kit signature (ed25519 over BLAKE3 of the canonical
  manifest) covers the component digest; an unsigned or re-signed-without-
  review component does not load.
- **Deny-by-default imports.** A component's imports are derived from the
  role's ARP: network egress rules → outbound host grants, fs scopes →
  preopened dirs, `McpToolGrant` → callable tool imports. No ambient
  authority.
- **Metered execution.** Fuel/epoch budgets derive from the role ARP budget
  config; exhaustion is a trap, recorded in the DecisionTrace, never a hang.
- **Canary first.** New capability runs under an ARP Quarantine-scoped canary
  before general availability (see CapabilityProposal gates, issue #617).

## Empirical gate (issue #616)

Labels are insufficient — the same lesson as MoE effective parameters. The
spike must demonstrate, with numbers on M-series and mini-DC hardware:

1. Fuel/epoch budgets driven from ARP budget config.
2. Import grants derived from a role ARP (network→egress, fs→allowed_paths,
   McpToolGrant mapping).
3. Cold instantiate, pooled instantiate, and call overhead vs an in-process
   tool.
4. One toy evolved tool end to end: WIT world → component → kit-signed →
   executed under a quarantine canary.

Binary-size note: wasmtime is a multi-megabyte dependency and the repo has a
≤60MB binary budget plus a Windows no-C++ default build. The embedding crate
stays out of `default-members` and behind a non-default feature until the
spike numbers justify promotion.

## Consequences

- Capability addition is structurally separate from policy tightening: a new
  component cannot ride the TighteningProposal channel (closed effect enum);
  it goes through the CapabilityProposal gates (#617) — verifier pass
  (sbe+sat), WIT conformance, SwarmKit conformance, quarantine canary, human
  approval, kit re-sign. Two channels, asymmetric friction: tightenings
  flow, capabilities crawl.
- The decision plane remains formally analyzable forever; growth pressure
  lands on the effect plane where the sandbox absorbs it.
