# Trust Layer Merge and Dispatch Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Land the eight stalled trust-layer branches on `main` as four sequential PRs, collapse the four CWT verifiers into one, and ship a permit-bound MCP dispatch gate that an operator can run today with `arkavo mcp proxy`.

**Architecture:** Phase A merges six foundation branches into one PR (version-only conflicts, one real spec-index conflict) and adds `/healthz`. Phase B extracts `arkavo-cwt` from #665 as the single COSE_Sign1 parser and key type, then rebases `arkavo-permit` on it. Phase C adds `arkavo-dispatch-gate` (authn → policy → budget over a permit and a proof-of-possession signature), plugs it into `arkavo-mcp-proxy` as a `PolicyHook`, records stage latency on `dispatch_gate`, and wires the `arkavo mcp proxy` command.

**Tech Stack:** Rust workspace, `coset` 0.4 + `ciborium` 0.2 (COSE/CBOR), `ed25519-dalek` + `p256` (signatures), `arkavo-crypto` key types, tokio stdio relay, axum for the AG-UI gateway, `arkavo-observability::subsystem_timing` for latency.

**Spec:** GitHub issue arkavo-org/arkavo-edge#655 (Epics 1.1, 1.2, 3.1, 3.2, 3.3), `docs/permit-cwt-schema.md` (arrives with #662), `specs/arkavo-edge/token-binding.spec.yaml` (arrives with #656), `docs/gate-latency-baseline.md` (arrives with #654).

## Global Constraints

- Root `Cargo.toml` `[workspace.package] version` must be strictly greater than `main` at merge time. CI job `version-check` compares with `sort -V`. Target versions: Phase A `0.91.1`, Phase B `0.91.2` then `0.91.3`, Phase C `0.92.0`. Before marking any PR ready, run `git fetch origin main && git show origin/main:Cargo.toml | grep -m1 '^version'`; if `main` moved past the target, bump again and regenerate `Cargo.lock`.
- One trust-layer PR open at a time. The DLP track keeps merging; every second open PR on this track re-creates the version pile this plan exists to clear.
- Merge branches with `git merge --no-ff origin/<branch>`, never rebase. The proxy branch predates #672; a rebase would re-apply its tree over the taint work, a merge keeps both.
- Regenerate `Cargo.lock` with `cargo update --workspace` after every root version change, then prove it with `cargo check --locked -q`. Commit `Cargo.lock` with `Cargo.toml`.
- No Conventional Commits prefixes. Commit subjects are plain sentences.
- `cargo fmt -- --check`, `cargo build -q`, `cargo clippy -- -D warnings` before every push. No `--release` builds locally.
- No `#[allow(dead_code)]`, no TODO comments, implementation files under 400 lines excluding tests.
- Pure Rust only for new deps (Windows no-C++ rule). `coset`, `ciborium`, `p256`, `ed25519-dalek` all qualify.
- Every bug fix or security check gets a regression test in the same commit.
- Out of scope for this plan, each needs its own plan: sequence-integrity stage (Epic 5.1), step-up approval (3.4), closure receipts (3.5), OIDC delegator bridge (2.1), Helm chart, the full #665 identity work beyond the crate extraction.

---

## Phase A: Foundation PR (version 0.91.1)

Facts this phase relies on, verified 2026-09-01 with `git merge-tree` against `origin/main` at `b3f01dca`:

- All six branches share merge-base `6975d04d`. `feature/epic0-foundation` already contains `fix/trust-layer-security` (`b9f4c760`), so #653 does not need a separate merge.
- Every branch conflicts with `main` only on `Cargo.toml` (one line) and `Cargo.lock` (81 workspace-member version stanzas), except `feature/epic0-foundation` and `feature/cwt-token-binding-spec`, which also conflict in `specs/arkavo-edge/index.yaml`.
- Branch-vs-branch conflicts are version-only. No source, spec, workflow, or Dockerfile overlaps between any pair.
- The `stats:` block in `index.yaml` is not CI-validated. `main`'s values (`total_specs: 83`, `total_scenarios: 805`, `critical_scenarios: 245`) are the correct count for `main`'s tree and already include epic0's `204 → 211` fix.

### Task A1: Start the branch, merge the loopback bind fix, set the version

**Files:**
- Modify: `Cargo.toml:165` (workspace version)
- Modify: `Cargo.lock` (regenerated)
- Merges: `origin/fix/agui-loopback-default` (#660)
- Add: `docs/superpowers/plans/2026-09-01-trust-layer-merge-and-gate.md` (this file)

**Interfaces:**
- Produces: branch `feature/trust-layer-foundation` at `0.91.1`; `crates/arkavo-agui/src/gateway_bind.rs` with `resolve_bind_addr()` and `BIND_ENV_VAR` (`ARKAVO_AGUI_BIND`).

- [ ] **Step 1: Create the branch from main and record the plan**

```bash
cd /Users/paul/Projects/arkavo/arkavo-edge
git fetch origin main fix/agui-loopback-default fix/router-fallback-test \
  feature/epic0-foundation feature/cwt-token-binding-spec feature/slim-build-feature-gates
git checkout -b feature/trust-layer-foundation origin/main
git add docs/superpowers/plans/2026-09-01-trust-layer-merge-and-gate.md
git commit -m "Add trust-layer merge and dispatch gate plan"
```

- [ ] **Step 2: Merge #660 and resolve the version conflict**

```bash
git merge --no-ff origin/fix/agui-loopback-default
# Expected: CONFLICT in Cargo.toml and Cargo.lock only
git checkout --ours Cargo.toml
sed -i '' '165s/version = "0.91.0"/version = "0.91.1"/' Cargo.toml
grep -n -m1 '^version' Cargo.toml
# Expected: 165:version = "0.91.1"
git checkout --ours Cargo.lock
cargo update --workspace
cargo check --locked -q
git add Cargo.toml Cargo.lock
git commit --no-edit
```

- [ ] **Step 3: Run the regression tests that came with #660**

Run: `cargo test -q -p arkavo-agui gateway_bind`
Expected: all tests in `gateway_bind` pass, none skipped.

- [ ] **Step 4: Verify the bind site now uses the resolver**

Run: `grep -n 'resolve_bind_addr\|0, 0, 0, 0' crates/arkavo-agui/src/gateway.rs`
Expected: a `resolve_bind_addr` call and no `[0, 0, 0, 0]` literal.

### Task A2: Merge the router fallback test fix

**Files:**
- Merges: `origin/fix/router-fallback-test` (#661)
- Modify: `Cargo.toml`, `Cargo.lock` (conflict resolution only)

- [ ] **Step 1: Merge and resolve**

```bash
git merge --no-ff origin/fix/router-fallback-test
git checkout --ours Cargo.toml Cargo.lock
cargo update --workspace && cargo check --locked -q
git add Cargo.toml Cargo.lock
git commit --no-edit
```

- [ ] **Step 2: Run the fixed test**

Run: `cargo test -q -p arkavo-orchestrator --test gemini_3_orc_suite_c test_orc_01c_router_fallback`
Expected: PASS. The assertion is now `decision.recommended_model.is_local()`.

### Task A3: Merge Epic 0 foundation (includes the trust-layer security fixes)

**Files:**
- Merges: `origin/feature/epic0-foundation` (#654, contains #653)
- Modify: `Cargo.toml`, `Cargo.lock`, `specs/arkavo-edge/index.yaml` (conflicts)
- Modify: `Dockerfile:1-7` (stale comment about `arkavo-ui-generator`, becomes wrong once A5 lands)

**Interfaces:**
- Produces: `arkavo_observability::subsystem_timing::SubsystemTimingRegistry::dispatch_gate: LatencyTracker` (Phase C records into it); `crates/arkavo-server/tests/kas_delegation_test.rs`; `docs/gate-latency-baseline.md`; `docs/evidence-service-decision.md`; `Dockerfile`; `.github/workflows/feature.yaml` `integrity` matrix group.

- [ ] **Step 1: Merge and resolve the three conflicts**

```bash
git merge --no-ff origin/feature/epic0-foundation
# Expected conflicts: Cargo.toml, Cargo.lock, specs/arkavo-edge/index.yaml
git checkout --ours Cargo.toml Cargo.lock
# epic0's only index.yaml edits are the header last_updated and the stats block;
# main's side is correct for both, and epic0 adds nothing else to this file.
git checkout --ours specs/arkavo-edge/index.yaml
cargo update --workspace && cargo check --locked -q
grep -c '<<<<<<<' specs/arkavo-edge/index.yaml Cargo.toml
# Expected: 0 for both
git add Cargo.toml Cargo.lock specs/arkavo-edge/index.yaml
git commit --no-edit
```

- [ ] **Step 2: Fix the stale Dockerfile comment**

Run: `sed -n '1,8p' Dockerfile` and replace the sentence that says the build transitively compiles `arkavo-llama-cpp-sys` until `arkavo-ui-generator` is feature-gated with:

```dockerfile
# Slim build: llama-cpp is feature-gated end to end (ui-generator, agui,
# orchestrator, server), so this image needs neither cmake nor the vendored
# llama.cpp tree.
```

Then `git add Dockerfile && git commit -m "Drop the stale llama-cpp caveat from the slim Dockerfile"`.

- [ ] **Step 3: Run the security and observability tests that came with #653 and #654**

```bash
cargo test -q -p arkavo-server --test kas_delegation_test
cargo test -q -p arkavo-protocol registration
cargo test -q -p arkavo-observability subsystem_timing
cargo check -q -p arkavo-router --benches
```

Expected: all pass. The router bench check proves `benches/gate_latency.rs` still compiles against `main`'s router.

- [ ] **Step 4: Confirm the delegation verification is fail-closed on main's code**

Run: `grep -n 'ARKAVO_ALLOW_UNVERIFIED_DELEGATION' crates/arkavo-protocol/src/registration/mod.rs AGENTS.md`
Expected: the env var is read in `registration/mod.rs` and documented in `AGENTS.md`.

### Task A4: Merge the CWT token-binding spec amendment

**Files:**
- Merges: `origin/feature/cwt-token-binding-spec` (#656)
- Modify: `specs/arkavo-edge/index.yaml` (header conflict; keep cwt's description edit at line ~34)

- [ ] **Step 1: Merge and resolve by hand**

```bash
git merge --no-ff origin/feature/cwt-token-binding-spec
git checkout --ours Cargo.toml Cargo.lock
cargo update --workspace && cargo check --locked -q
grep -n '<<<<<<<\|>>>>>>>' specs/arkavo-edge/index.yaml
```

Expected: one conflict block at the file header. Edit the file so the header reads:

```yaml
version: 0.91.0
last_updated: 2026-08-30
```

and delete the three marker lines and the `0.87.0` / `2026-08-26` side. Do not use `--ours` for this file: the branch's non-conflicting change to the token-binding description must survive.

- [ ] **Step 2: Verify both edits are present**

```bash
grep -c '<<<<<<<' specs/arkavo-edge/index.yaml   # Expected: 0
grep -n 'CWT/COSE cnf' specs/arkavo-edge/index.yaml   # Expected: one line in the token-binding component
grep -n 'RFC8747' specs/arkavo-edge/token-binding.spec.yaml | head -3   # Expected: hits
git add Cargo.toml Cargo.lock specs/arkavo-edge/index.yaml
git commit --no-edit
```

- [ ] **Step 3: Run the spec tooling**

Run: `cargo run -q -p xtask -- spec-test coverage --markdown --fail-under 1 > /dev/null && echo OK`
Expected: `OK`. This is the same command the `spec-coverage` job runs.

### Task A5: Merge the slim-build feature gates

**Files:**
- Merges: `origin/feature/slim-build-feature-gates` (#657)

- [ ] **Step 1: Merge and resolve**

```bash
git merge --no-ff origin/feature/slim-build-feature-gates
git checkout --ours Cargo.toml Cargo.lock
cargo update --workspace && cargo check --locked -q
git add Cargo.toml Cargo.lock
git commit --no-edit
```

- [ ] **Step 2: Prove the slim build no longer pulls llama-cpp**

```bash
cargo tree -q -p arkavo --no-default-features --features memory,mdns,mcp-tools,llm-remote,web-ui \
  | grep -c 'arkavo-llama-cpp-sys'
```

Expected: `0`. This is the feature set the `Dockerfile` builds with.

- [ ] **Step 3: Check the default build still has it**

Run: `cargo tree -q -p arkavo | grep -c 'arkavo-llama-cpp-sys'`
Expected: a number greater than `0`.

### Task A6: Add `/healthz` and `/readyz` to the AG-UI gateway

The container docs from #654 describe liveness and readiness probes, and no HTTP surface on `main` serves them. `HealthRegistry::global().get_overall_status()` in `arkavo-observability` already aggregates component reporters.

**Files:**
- Create: `crates/arkavo-agui/src/gateway_health.rs`
- Modify: `crates/arkavo-agui/src/lib.rs` (add `mod gateway_health;` beside `mod gateway_bind;`)
- Modify: `crates/arkavo-agui/src/gateway.rs:477-482` (static_routes)
- Modify: `docs/deploy/container.md` (probe paths)

**Interfaces:**
- Produces: `GET /healthz` → `200 ok` always once the listener is up; `GET /readyz` → `200` when overall status is `Healthy` or `Degraded`, `503` otherwise, JSON body `{"status": "<variant>"}`.

- [ ] **Step 1: Write the failing tests**

Create `crates/arkavo-agui/src/gateway_health.rs` with only the test module first:

```rust
//! Liveness and readiness probes for container orchestrators. `/healthz`
//! answers as soon as the listener is up; `/readyz` reflects the health
//! registry so a pod with a failed component is pulled from the service.

#[cfg(test)]
mod tests {
    use super::*;
    use axum::response::IntoResponse;

    #[tokio::test]
    async fn healthz_is_always_ok() {
        let response = healthz_handler().await.into_response();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn readyz_reports_registry_status() {
        let response = readyz_handler().await.into_response();
        let status = response.status();
        assert!(
            status == axum::http::StatusCode::OK
                || status == axum::http::StatusCode::SERVICE_UNAVAILABLE,
            "unexpected status {status}"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -q -p arkavo-agui gateway_health`
Expected: compile error, `healthz_handler` not found.

- [ ] **Step 3: Implement the handlers**

Prepend to `gateway_health.rs` above the test module:

```rust
use arkavo_observability::health_reporter::{HealthRegistry, HealthStatus};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

pub async fn healthz_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

pub async fn readyz_handler() -> impl IntoResponse {
    let status = HealthRegistry::global().get_overall_status().await;
    let code = match status {
        HealthStatus::Healthy | HealthStatus::Degraded => StatusCode::OK,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    (code, Json(json!({ "status": format!("{status:?}") })))
}
```

Check the enum variant names with `grep -n -A6 'pub enum HealthStatus' crates/arkavo-observability/src/health_reporter.rs` and adjust the match arms if they differ from `Healthy` / `Degraded`.

Register the module in `crates/arkavo-agui/src/lib.rs` next to `mod gateway_bind;`:

```rust
mod gateway_health;
```

Add the routes in `gateway.rs` inside `static_routes` (unauthenticated, not rate-limited, same as `/`):

```rust
        let static_routes = Router::new()
            .route("/", get(crate::gateway_static::index_handler))
            .route("/healthz", get(crate::gateway_health::healthz_handler))
            .route("/readyz", get(crate::gateway_health::readyz_handler))
            .route(
                "/static/*path",
                get(crate::gateway_static::static_file_handler),
            );
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -q -p arkavo-agui gateway_health`
Expected: 2 passed.

- [ ] **Step 5: Document the probes and commit**

In `docs/deploy/container.md`, under the section that discusses health, add:

```markdown
Liveness: `GET /healthz` on the AG-UI port returns `200 ok` once the listener is bound.
Readiness: `GET /readyz` returns `200` while the health registry reports healthy or degraded, `503` otherwise.
```

```bash
cargo fmt
git add crates/arkavo-agui/src/gateway_health.rs crates/arkavo-agui/src/lib.rs \
  crates/arkavo-agui/src/gateway.rs docs/deploy/container.md
git commit -m "Serve healthz and readyz probes from the AG-UI gateway"
```

### Task A7: Pre-push checks, PR, retire the six old PRs

- [ ] **Step 1: Full checklist**

```bash
cargo fmt -- --check
cargo build -q
cargo clippy -- -D warnings
cargo test -q -p arkavo-protocol --test security_vulnerabilities
cargo test -q -p arkavo-cli mock_provider
```

Expected: all clean. If clippy flags anything in merged code, fix it in a separate commit named for the file.

- [ ] **Step 2: Re-check the version floor and push**

```bash
git fetch origin main
git show origin/main:Cargo.toml | grep -m1 '^version'   # must be < 0.91.1, else bump and regenerate
git push -u origin feature/trust-layer-foundation
gh pr create --repo arkavo-org/arkavo-edge --title "Trust-layer foundation: security fixes, container, spec, slim build, loopback bind, health probes" \
  --body-file - <<'EOF'
Consolidates #653, #654, #656, #657, #660, #661 onto current main at 0.91.1, plus `/healthz` and `/readyz` on the AG-UI gateway.

Merge-only except for: Dockerfile comment fix, `gateway_health.rs`, `container.md` probe docs. The only non-version conflict was `specs/arkavo-edge/index.yaml`, resolved to main's counts (they already include the 204→211 drift fix from #654).

Tracking: #655.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
```

- [ ] **Step 3: Close the six superseded PRs after CI is green**

```bash
NEW=$(gh pr view --repo arkavo-org/arkavo-edge --json number --jq .number)
for n in 653 654 656 657 660 661; do
  gh pr close $n --repo arkavo-org/arkavo-edge --comment "Superseded by #$NEW (consolidated foundation PR at 0.91.1). Branch kept."
done
```

Do not delete the branches until the foundation PR is merged.

---

## Phase B: One CWT verifier (versions 0.91.2 and 0.91.3)

Facts this phase relies on:

- `crates/arkavo-cwt` exists only on `origin/agent-identity-cwt` (#665). Its verifier is ES256-only, key-set based, 12 tests, spec `specs/arkavo-edge/agent-cwt.spec.yaml` (ACWT-001..003). It uses `coset` 0.4 via a workspace dep that #665 adds.
- `crates/arkavo-permit` on `origin/feature/permit-cwt-crate` (#662) has its own COSE parse, its own `PermitVerifier` (Ed25519 + P-256 over `arkavo-crypto` types), pins `coset = "0.3"`, and has 47 tests plus three JSON vectors under `tests/vectors/`.
- `main` has no `coset`, `ciborium`, or `p256` in `[workspace.dependencies]`. `ed25519-dalek = "2.1"` is there.
- `#659` carries a third copy in `arkavo-authorization`. It is not merged in this plan; Task B4 tells its author what to depend on.

Consolidation decision: `arkavo-cwt` owns COSE_Sign1 parsing, algorithm policy, and the `VerifyingKey` type. `arkavo-permit` keeps permit claims, minting, `cnf` extraction, and time checks, but delegates parse and signature verification. Bearer-token key-set verification stays in `arkavo-cwt::verify`.

### Task B1: Extract `arkavo-cwt` from #665 into its own PR

**Files:**
- Create: `crates/arkavo-cwt/` (copied from the branch)
- Create: `specs/arkavo-edge/agent-cwt.spec.yaml` (copied)
- Modify: `Cargo.toml` (members, default-members, workspace deps, version `0.91.2`)
- Modify: `specs/arkavo-edge/index.yaml` (component entry)
- Modify: `.github/workflows/feature.yaml` (`protocol` arm)

**Interfaces:**
- Produces: crate `arkavo-cwt` with `verify(token_b64url, &KeySet, &VerifyOptions) -> Result<Claims, CwtError>`, `KeySet`, `CachedKeySet`, `Claims`, `CwtError`.

- [ ] **Step 1: Branch and copy the crate**

```bash
git fetch origin main agent-identity-cwt
git checkout -b feature/cwt-verifier-crate origin/main
git checkout origin/agent-identity-cwt -- crates/arkavo-cwt specs/arkavo-edge/agent-cwt.spec.yaml
```

- [ ] **Step 2: Register the crate and the deps**

In the root `Cargo.toml`, add `"crates/arkavo-cwt",` to `members` (alphabetically near `crates/arkavo-crypto`) and to `default-members` if that list exists (check with `grep -n 'default-members' Cargo.toml`). In `[workspace.dependencies]` add:

```toml
coset = "0.4"
ciborium = "0.2"
p256 = { version = "0.13", features = ["ecdsa"] }
```

Set the workspace version line to `0.91.2`. Then:

```bash
cargo update --workspace && cargo check --locked -q -p arkavo-cwt
```

- [ ] **Step 3: Index the spec**

```bash
git show origin/agent-identity-cwt:specs/arkavo-edge/index.yaml | grep -n -B1 -A12 'name: agent-cwt'
```

Copy that component block into `specs/arkavo-edge/index.yaml` in the same list as the `identity-session` component, keeping `scenario_count: 3`. Update `stats:` by adding 1 to `total_specs`, 3 to `total_scenarios`, and the ACWT critical count (`grep -c 'criticality: critical' specs/arkavo-edge/agent-cwt.spec.yaml`) to `critical_scenarios`.

Run: `cargo run -q -p xtask -- spec-test coverage --markdown --fail-under 1 > /dev/null && echo OK`
Expected: `OK`.

- [ ] **Step 4: Run the crate's tests and add it to CI**

Run: `cargo test -q -p arkavo-cwt`
Expected: 12 passed (one uses wiremock; needs no network).

In `.github/workflows/feature.yaml`, in the `protocol` arm of both the test step and the clippy step, add after the `arkavo-critic` lines:

```yaml
            cargo test --locked -p arkavo-cwt
```
```yaml
            cargo clippy --locked -p arkavo-cwt --lib --bins -- -D warnings
```

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add -A crates/arkavo-cwt specs/arkavo-edge Cargo.toml Cargo.lock .github/workflows/feature.yaml
git commit -m "Add arkavo-cwt: COSE_Sign1 CWT verification against a published key set"
```

### Task B2: Give `arkavo-cwt` a shared parser and key type

**Files:**
- Create: `crates/arkavo-cwt/src/key.rs`
- Create: `crates/arkavo-cwt/src/sign1.rs`
- Modify: `crates/arkavo-cwt/src/lib.rs` (modules, re-exports, two new `CwtError` variants)
- Modify: `crates/arkavo-cwt/src/keys.rs` (store `VerifyingKey`)
- Modify: `crates/arkavo-cwt/src/verify.rs` (use `sign1::parse`)
- Modify: `crates/arkavo-cwt/Cargo.toml` (add `ed25519-dalek = { workspace = true }`)

**Interfaces:**
- Produces:
  - `arkavo_cwt::VerifyingKey` enum `{ Ed25519(ed25519_dalek::VerifyingKey), P256(p256::ecdsa::VerifyingKey) }` with `algorithm() -> coset::iana::Algorithm`, `from_cose_key(&CoseKey) -> Result<Self, CwtError>`, `to_cose_key(&self) -> CoseKey`, `verify(&self, algorithm: Algorithm, data: &[u8], signature: &[u8]) -> Result<(), CwtError>`, `public_key_bytes(&self) -> Vec<u8>`.
  - `arkavo_cwt::sign1::{parse, ParsedSign1, CWT_TAG_PREFIX, MAX_TOKEN_BYTES}` where `parse(bytes: &[u8]) -> Result<ParsedSign1, CwtError>` accepts an optional tag-61 prefix and either a tagged (18) or untagged COSE_Sign1; `ParsedSign1 { pub sign1: CoseSign1, pub algorithm: Algorithm }` with `kid() -> &[u8]`, `payload() -> Result<&[u8], CwtError>`, `verify(&self, key: &VerifyingKey) -> Result<(), CwtError>`.
  - `CwtError::Key(String)` and `CwtError::KeyAlgorithmMismatch`.

- [ ] **Step 1: Write the failing tests for `key.rs`**

Create `crates/arkavo-cwt/src/key.rs` with this test module only:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use coset::iana::Algorithm;
    use ed25519_dalek::Signer as _;
    use p256::ecdsa::signature::Signer as _;

    #[test]
    fn ed25519_round_trips_through_cose_key() {
        let signing = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let key = VerifyingKey::Ed25519(signing.verifying_key());
        let back = VerifyingKey::from_cose_key(&key.to_cose_key()).unwrap();
        assert_eq!(back.public_key_bytes(), key.public_key_bytes());
        let sig = signing.sign(b"data");
        back.verify(Algorithm::EdDSA, b"data", &sig.to_bytes()).unwrap();
    }

    #[test]
    fn p256_round_trips_and_rejects_wrong_algorithm() {
        let signing = p256::ecdsa::SigningKey::random(&mut rand::rngs::OsRng);
        let key = VerifyingKey::P256(*signing.verifying_key());
        let back = VerifyingKey::from_cose_key(&key.to_cose_key()).unwrap();
        let sig: p256::ecdsa::Signature = signing.sign(b"data");
        back.verify(Algorithm::ES256, b"data", &sig.to_bytes()).unwrap();
        assert!(matches!(
            back.verify(Algorithm::EdDSA, b"data", &sig.to_bytes()),
            Err(CwtError::KeyAlgorithmMismatch)
        ));
    }

    #[test]
    fn tampered_signature_is_bad_signature() {
        let signing = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let key = VerifyingKey::Ed25519(signing.verifying_key());
        let mut sig = signing.sign(b"data").to_bytes();
        sig[0] ^= 0x01;
        assert!(matches!(
            key.verify(Algorithm::EdDSA, b"data", &sig),
            Err(CwtError::BadSignature)
        ));
    }

    #[test]
    fn short_p256_coordinate_is_rejected() {
        let mut cose = VerifyingKey::P256(*p256::ecdsa::SigningKey::random(&mut rand::rngs::OsRng).verifying_key()).to_cose_key();
        for (label, value) in cose.params.iter_mut() {
            if *label == coset::Label::Int(coset::iana::Ec2KeyParameter::X as i64)
                && let ciborium::Value::Bytes(bytes) = value
            {
                bytes.truncate(31);
            }
        }
        assert!(matches!(VerifyingKey::from_cose_key(&cose), Err(CwtError::Key(_))));
    }
}
```

Add `pub mod key; pub mod sign1;` and `pub use key::VerifyingKey; pub use sign1::{parse, ParsedSign1};` to `lib.rs`, and the two error variants:

```rust
    #[error("unusable COSE key: {0}")]
    Key(String),
    #[error("signature algorithm does not match the key type")]
    KeyAlgorithmMismatch,
```

Change the `UnsupportedAlgorithm` message to `"unsupported signature algorithm: expected EdDSA or ES256, got {0}"`.

Add to `crates/arkavo-cwt/Cargo.toml` under `[dependencies]`: `ed25519-dalek = { workspace = true }`. Create an empty `sign1.rs` containing only `//! COSE_Sign1 parsing shared by permit and bearer verification.` so the crate compiles.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -q -p arkavo-cwt key::`
Expected: compile error, `VerifyingKey` not found.

- [ ] **Step 3: Implement `key.rs`**

Prepend above the test module:

```rust
//! One verifying-key type for every CWT the edge checks. Permits carry the
//! key inline in `cnf`; bearer tokens look it up by `kid`. Both end here.

use crate::CwtError;
use ciborium::Value;
use coset::iana::{Algorithm, Ec2KeyParameter, EllipticCurve, EnumI64, KeyType, OkpKeyParameter};
use coset::{CoseKey, CoseKeyBuilder, Label, RegisteredLabel};
use p256::ecdsa::signature::Verifier as _;
use p256::elliptic_curve::sec1::ToEncodedPoint as _;

#[derive(Clone, Debug)]
pub enum VerifyingKey {
    Ed25519(ed25519_dalek::VerifyingKey),
    P256(p256::ecdsa::VerifyingKey),
}

impl VerifyingKey {
    pub fn algorithm(&self) -> Algorithm {
        match self {
            Self::Ed25519(_) => Algorithm::EdDSA,
            Self::P256(_) => Algorithm::ES256,
        }
    }

    pub fn from_cose_key(key: &CoseKey) -> Result<Self, CwtError> {
        match key.kty {
            RegisteredLabel::Assigned(KeyType::OKP) => {
                expect_curve(key, OkpKeyParameter::Crv.to_i64(), EllipticCurve::Ed25519)?;
                let x = bytes_param(key, OkpKeyParameter::X.to_i64(), "x")?;
                let raw: [u8; 32] = x
                    .try_into()
                    .map_err(|_| CwtError::Key("Ed25519 x must be 32 bytes".into()))?;
                ed25519_dalek::VerifyingKey::from_bytes(&raw)
                    .map(Self::Ed25519)
                    .map_err(|e| CwtError::Key(e.to_string()))
            }
            RegisteredLabel::Assigned(KeyType::EC2) => {
                expect_curve(key, Ec2KeyParameter::Crv.to_i64(), EllipticCurve::P_256)?;
                let x = bytes_param(key, Ec2KeyParameter::X.to_i64(), "x")?;
                let y = bytes_param(key, Ec2KeyParameter::Y.to_i64(), "y")?;
                if x.len() != 32 || y.len() != 32 {
                    return Err(CwtError::Key("P-256 coordinates must be 32 bytes".into()));
                }
                let point = p256::EncodedPoint::from_affine_coordinates(
                    p256::FieldBytes::from_slice(x),
                    p256::FieldBytes::from_slice(y),
                    false,
                );
                p256::ecdsa::VerifyingKey::from_encoded_point(&point)
                    .map(Self::P256)
                    .map_err(|e| CwtError::Key(e.to_string()))
            }
            _ => Err(CwtError::Key("key type is neither OKP nor EC2".into())),
        }
    }

    pub fn to_cose_key(&self) -> CoseKey {
        match self {
            Self::Ed25519(key) => CoseKeyBuilder::new_okp_key()
                .param(
                    OkpKeyParameter::Crv.to_i64(),
                    Value::from(EllipticCurve::Ed25519.to_i64()),
                )
                .param(OkpKeyParameter::X.to_i64(), Value::Bytes(key.to_bytes().to_vec()))
                .algorithm(Algorithm::EdDSA)
                .build(),
            Self::P256(key) => {
                let point = key.to_encoded_point(false);
                let x = point.x().map(|c| c.to_vec()).unwrap_or_default();
                let y = point.y().map(|c| c.to_vec()).unwrap_or_default();
                CoseKeyBuilder::new_ec2_pub_key(EllipticCurve::P_256, x, y)
                    .algorithm(Algorithm::ES256)
                    .build()
            }
        }
    }

    pub fn verify(&self, algorithm: Algorithm, data: &[u8], signature: &[u8]) -> Result<(), CwtError> {
        if algorithm != self.algorithm() {
            return Err(CwtError::KeyAlgorithmMismatch);
        }
        match self {
            Self::Ed25519(key) => {
                let sig = ed25519_dalek::Signature::from_slice(signature)
                    .map_err(|_| CwtError::BadSignature)?;
                key.verify_strict(data, &sig).map_err(|_| CwtError::BadSignature)
            }
            Self::P256(key) => {
                let sig = p256::ecdsa::Signature::from_slice(signature)
                    .map_err(|_| CwtError::BadSignature)?;
                key.verify(data, &sig).map_err(|_| CwtError::BadSignature)
            }
        }
    }

    /// 32 raw bytes for Ed25519, 65-byte uncompressed SEC1 for P-256.
    pub fn public_key_bytes(&self) -> Vec<u8> {
        match self {
            Self::Ed25519(key) => key.to_bytes().to_vec(),
            Self::P256(key) => key.to_encoded_point(false).as_bytes().to_vec(),
        }
    }
}

fn param<'a>(key: &'a CoseKey, label: i64) -> Option<&'a Value> {
    key.params
        .iter()
        .find(|(candidate, _)| *candidate == Label::Int(label))
        .map(|(_, value)| value)
}

fn bytes_param<'a>(key: &'a CoseKey, label: i64, name: &str) -> Result<&'a [u8], CwtError> {
    param(key, label)
        .and_then(Value::as_bytes)
        .map(Vec::as_slice)
        .ok_or_else(|| CwtError::Key(format!("COSE key is missing its {name} coordinate")))
}

fn expect_curve(key: &CoseKey, label: i64, curve: EllipticCurve) -> Result<(), CwtError> {
    let actual = param(key, label).and_then(Value::as_integer);
    if actual == Some(curve.to_i64().into()) {
        Ok(())
    } else {
        Err(CwtError::Key(format!("unexpected curve {actual:?}")))
    }
}
```

The `coset` 0.4 builder signatures are `CoseKeyBuilder::new_okp_key()`, `new_ec2_pub_key(curve, x: Vec<u8>, y: Vec<u8>)`, `.param(i64, Value)`, `.algorithm(Algorithm)`. If `rand` is not already a dev-dep of `arkavo-cwt`, it is on the copied `Cargo.toml` (`rand = { workspace = true }`); keep it.

- [ ] **Step 4: Run the key tests**

Run: `cargo test -q -p arkavo-cwt key::`
Expected: 4 passed.

- [ ] **Step 5: Write the failing tests for `sign1.rs`**

Append to `crates/arkavo-cwt/src/sign1.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use coset::{CborSerializable, CoseSign1Builder, HeaderBuilder, TaggedCborSerializable};
    use ed25519_dalek::Signer as _;

    fn signed(tagged: bool, prefix: bool) -> (Vec<u8>, VerifyingKey) {
        let signing = ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng);
        let protected = HeaderBuilder::new()
            .algorithm(coset::iana::Algorithm::EdDSA)
            .key_id(b"k1".to_vec())
            .build();
        let sign1 = CoseSign1Builder::new()
            .protected(protected)
            .payload(b"payload".to_vec())
            .create_signature(b"", |data| signing.sign(data).to_bytes().to_vec())
            .build();
        let mut bytes = if prefix { CWT_TAG_PREFIX.to_vec() } else { Vec::new() };
        if tagged {
            bytes.extend(sign1.to_tagged_vec().unwrap());
        } else {
            bytes.extend(sign1.to_vec().unwrap());
        }
        (bytes, VerifyingKey::Ed25519(signing.verifying_key()))
    }

    #[test]
    fn parses_all_four_wire_shapes() {
        for (tagged, prefix) in [(true, true), (true, false), (false, true), (false, false)] {
            let (bytes, key) = signed(tagged, prefix);
            let parsed = parse(&bytes).unwrap();
            assert_eq!(parsed.algorithm, coset::iana::Algorithm::EdDSA);
            assert_eq!(parsed.kid(), b"k1");
            assert_eq!(parsed.payload().unwrap(), b"payload");
            parsed.verify(&key).unwrap();
        }
    }

    #[test]
    fn rejects_oversized_input_before_parsing() {
        let big = vec![0u8; MAX_TOKEN_BYTES + 1];
        assert!(matches!(parse(&big), Err(CwtError::Cose(_))));
    }

    #[test]
    fn rejects_missing_alg() {
        let sign1 = CoseSign1Builder::new().payload(b"p".to_vec()).build();
        let bytes = sign1.to_vec().unwrap();
        assert!(matches!(parse(&bytes), Err(CwtError::UnsupportedAlgorithm(_))));
    }

    #[test]
    fn wrong_key_fails_verification() {
        let (bytes, _) = signed(true, true);
        let other = VerifyingKey::Ed25519(
            ed25519_dalek::SigningKey::generate(&mut rand::rngs::OsRng).verifying_key(),
        );
        assert!(matches!(parse(&bytes).unwrap().verify(&other), Err(CwtError::BadSignature)));
    }
}
```

- [ ] **Step 6: Run to verify they fail**

Run: `cargo test -q -p arkavo-cwt sign1::`
Expected: compile error, `parse` not found.

- [ ] **Step 7: Implement `sign1.rs`**

Insert above the test module:

```rust
use crate::{CwtError, VerifyingKey};
use coset::iana::Algorithm;
use coset::{CborSerializable, CoseSign1, RegisteredLabelWithPrivate, TaggedCborSerializable};

/// CBOR tag 61 (CWT) as it appears on the wire.
pub const CWT_TAG_PREFIX: [u8; 2] = [0xd8, 0x3d];

/// Untrusted input larger than this is refused before any CBOR work.
pub const MAX_TOKEN_BYTES: usize = 16 * 1024;

pub struct ParsedSign1 {
    pub sign1: CoseSign1,
    pub algorithm: Algorithm,
}

/// Parse a CWT-shaped COSE_Sign1. The tag-61 prefix is optional and the
/// COSE_Sign1 may be tagged (18) or bare: authnz-rs emits bare, permits
/// emit tagged, and both must verify through the same code.
pub fn parse(bytes: &[u8]) -> Result<ParsedSign1, CwtError> {
    if bytes.len() > MAX_TOKEN_BYTES {
        return Err(CwtError::Cose("token exceeds maximum size".into()));
    }
    let body = bytes.strip_prefix(&CWT_TAG_PREFIX[..]).unwrap_or(bytes);
    let sign1 = CoseSign1::from_tagged_slice(body)
        .or_else(|_| CoseSign1::from_slice(body))
        .map_err(|e| CwtError::Cose(e.to_string()))?;
    let algorithm = match &sign1.protected.header.alg {
        Some(RegisteredLabelWithPrivate::Assigned(alg @ (Algorithm::EdDSA | Algorithm::ES256))) => *alg,
        Some(other) => return Err(CwtError::UnsupportedAlgorithm(format!("{other:?}"))),
        None => return Err(CwtError::UnsupportedAlgorithm("none".into())),
    };
    Ok(ParsedSign1 { sign1, algorithm })
}

impl ParsedSign1 {
    pub fn kid(&self) -> &[u8] {
        &self.sign1.protected.header.key_id
    }

    pub fn payload(&self) -> Result<&[u8], CwtError> {
        self.sign1
            .payload
            .as_deref()
            .ok_or_else(|| CwtError::Cose("payload is detached".into()))
    }

    pub fn verify(&self, key: &VerifyingKey) -> Result<(), CwtError> {
        self.sign1
            .verify_signature(b"", |signature, data| key.verify(self.algorithm, data, signature))
    }
}
```

- [ ] **Step 8: Run the sign1 tests**

Run: `cargo test -q -p arkavo-cwt sign1::`
Expected: 4 passed.

- [ ] **Step 9: Route `keys.rs` and `verify.rs` through the shared types**

In `keys.rs`: change `keys: Vec<(Vec<u8>, VerifyingKey)>` to use `crate::VerifyingKey`, drop the `p256::` imports, `is_p256`, `param`, and `verifying_key` helpers, and make `from_cbor` build entries with:

```rust
        for key in &set.0 {
            if key.key_id.is_empty() {
                continue;
            }
            match crate::VerifyingKey::from_cose_key(key) {
                Ok(crate::VerifyingKey::P256(vk)) => keys.push((key.key_id.clone(), crate::VerifyingKey::P256(vk))),
                _ => continue,
            }
        }
```

Keep the `no usable ES256 P-256 keys` error and the return type of `get` as `Option<&crate::VerifyingKey>`.

In `verify.rs`: replace everything from the base64 decode through `verify_signature` with:

```rust
    let bytes = URL_SAFE_NO_PAD
        .decode(token_b64url)
        .map_err(|e| CwtError::Base64(e.to_string()))?;
    let parsed = crate::sign1::parse(&bytes)?;
    if parsed.algorithm != coset::iana::Algorithm::ES256 {
        return Err(CwtError::UnsupportedAlgorithm(format!("{:?}", parsed.algorithm)));
    }
    let kid = parsed.kid();
    if kid.is_empty() {
        return Err(CwtError::MissingKid);
    }
    let key = keys.get(kid).ok_or_else(|| CwtError::UnknownKid(hex(kid)))?;
    parsed.verify(key)?;
    let claims = Claims::from_cbor(parsed.payload()?)?;
    check_claims(&claims, opts)?;
    Ok(claims)
```

Remove the now-unused `CWT_TAG`, `CoseSign1`, `RegisteredLabelWithPrivate`, `Signature`, and `Verifier` imports.

- [ ] **Step 10: Run the whole crate and clippy**

```bash
cargo test -q -p arkavo-cwt
cargo clippy -q -p arkavo-cwt --all-targets -- -D warnings
```

Expected: 20 passed (12 original + 8 new), clippy clean. The original ES256-only bearer tests still pass because Step 9 re-imposes ES256 for key-set verification.

- [ ] **Step 11: Commit, push, PR**

```bash
cargo fmt
git add -A crates/arkavo-cwt
git commit -m "Share COSE_Sign1 parsing and the verifying-key type across CWT consumers"
git fetch origin main && git show origin/main:Cargo.toml | grep -m1 '^version'   # must be < 0.91.2
git push -u origin feature/cwt-verifier-crate
gh pr create --repo arkavo-org/arkavo-edge --title "Add arkavo-cwt as the single COSE/CWT verifier" --body-file - <<'EOF'
Extracts `crates/arkavo-cwt` from #665 unchanged, then adds `sign1::parse` (tag-61 optional, tagged or bare COSE_Sign1, EdDSA|ES256) and a `VerifyingKey` enum (Ed25519 + P-256, COSE_Key round-trip). Bearer verification (`verify`, `KeySet`) now goes through both. `arkavo-permit` (#662 follow-up) and `arkavo-authorization` (#659) are expected to depend on this crate instead of carrying their own parsers.

Workspace gains `coset = "0.4"`, `ciborium = "0.2"`, `p256 = "0.13"`.

Tracking: #655 (finding 1 of the 2026-09-01 review).

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
```

### Task B3: Rebase `arkavo-permit` on `arkavo-cwt` (version 0.91.3)

Start only after B2's PR is merged.

**Files:**
- Merges: `origin/feature/permit-cwt-crate` (#662)
- Modify: `crates/arkavo-permit/Cargo.toml`
- Modify: `crates/arkavo-permit/src/keys.rs` (`PermitVerifier` becomes a newtype over `arkavo_cwt::VerifyingKey`)
- Modify: `crates/arkavo-permit/src/permit.rs` (parse and signature check delegate)
- Modify: `crates/arkavo-permit/src/error.rs` (mapping from `CwtError`)
- Modify: `.github/workflows/feature.yaml` (`protocol` arm)

**Interfaces:**
- Consumes: `arkavo_cwt::{VerifyingKey, sign1::parse, sign1::CWT_TAG_PREFIX, CwtError}`.
- Produces (unchanged public API of `arkavo-permit`): `mint(&PermitClaims, &PermitSigner) -> Result<Vec<u8>, PermitError>`, `verify(&[u8], now: i64) -> Result<Permit, PermitError>`, `decode`, `Permit { claims, confirmation_key: PermitVerifier }`, `PermitVerifier::{algorithm, to_cose_key, from_cose_key, verify, public_key_bytes}`, `PermitSigner::{algorithm, public_key, cose_key, sign}`, `PermitClaims::verify_invocation`, `argument_hash`, `HashAlgorithm`.
- The three JSON vectors under `crates/arkavo-permit/tests/vectors/` must verify byte-for-byte unchanged. That is the regression test for this task.

- [ ] **Step 1: Branch, merge, set the version**

```bash
git fetch origin main feature/permit-cwt-crate
git checkout -b feature/permit-on-arkavo-cwt origin/main
git merge --no-ff origin/feature/permit-cwt-crate
git checkout --ours Cargo.toml Cargo.lock
sed -i '' 's/^version = "0.91.2"$/version = "0.91.3"/' Cargo.toml
grep -n -m1 '^version' Cargo.toml    # Expected: version = "0.91.3"
```

Add `"crates/arkavo-permit",` to `members` (and `default-members` if present) if the merge did not already, then:

```bash
cargo update --workspace && cargo check --locked -q -p arkavo-permit
git add Cargo.toml Cargo.lock && git commit --no-edit
cargo test -q -p arkavo-permit
```

Expected: 47 passed on the merged-but-unrefactored crate. If `coset` 0.3 vs 0.4 API drift breaks the build here, fix only what the compiler names and commit that as "Build arkavo-permit against coset 0.4".

- [ ] **Step 2: Switch the deps**

Replace the `coset = "0.3"` and `ciborium = "0.2"` lines in `crates/arkavo-permit/Cargo.toml` with:

```toml
arkavo-cwt = { path = "../arkavo-cwt" }
coset = { workspace = true }
ciborium = { workspace = true }
ed25519-dalek = { workspace = true }
p256 = { workspace = true }
```

- [ ] **Step 3: Add the error mapping**

In `crates/arkavo-permit/src/error.rs` add:

```rust
impl From<arkavo_cwt::CwtError> for PermitError {
    fn from(error: arkavo_cwt::CwtError) -> Self {
        use arkavo_cwt::CwtError as E;
        match error {
            E::BadSignature => Self::InvalidSignature,
            E::KeyAlgorithmMismatch => Self::KeyAlgorithmMismatch,
            E::Key(message) => Self::InvalidConfirmationKey(message),
            E::UnsupportedAlgorithm(message) => Self::UnsupportedAlgorithm(message),
            other => Self::Cose(other.to_string()),
        }
    }
}
```

- [ ] **Step 4: Make `PermitVerifier` a newtype**

In `keys.rs` replace the `PermitVerifier` enum and its `impl` with:

```rust
/// The permit's confirmation key. A thin wrapper so permit code keeps its
/// error type while the COSE key handling lives in `arkavo-cwt`.
#[derive(Clone, Debug)]
pub struct PermitVerifier(pub arkavo_cwt::VerifyingKey);

impl PermitVerifier {
    pub fn algorithm(&self) -> Algorithm {
        self.0.algorithm()
    }

    pub fn to_cose_key(&self) -> CoseKey {
        self.0.to_cose_key()
    }

    pub fn from_cose_key(key: &CoseKey) -> Result<Self, PermitError> {
        Ok(Self(arkavo_cwt::VerifyingKey::from_cose_key(key)?))
    }

    pub fn verify(&self, algorithm: Algorithm, data: &[u8], signature: &[u8]) -> Result<(), PermitError> {
        Ok(self.0.verify(algorithm, data, signature)?)
    }

    pub fn public_key_bytes(&self) -> Vec<u8> {
        self.0.public_key_bytes()
    }
}
```

and change `PermitSigner::public_key` to build the shared key from `arkavo-crypto` bytes:

```rust
    pub fn public_key(&self) -> PermitVerifier {
        match self {
            Self::Ed25519(keypair) => {
                let bytes = keypair.public_key().to_bytes();
                // arkavo-crypto only ever hands out well-formed 32-byte Ed25519 keys.
                let raw: [u8; 32] = bytes[..32].try_into().expect("Ed25519 public key is 32 bytes");
                let key = ed25519_dalek::VerifyingKey::from_bytes(&raw)
                    .expect("arkavo-crypto Ed25519 keys are canonical");
                PermitVerifier(arkavo_cwt::VerifyingKey::Ed25519(key))
            }
            Self::P256(keypair) => {
                let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(&keypair.public_key().to_sec1_bytes())
                    .expect("arkavo-crypto P-256 keys are valid SEC1 points");
                PermitVerifier(arkavo_cwt::VerifyingKey::P256(key))
            }
        }
    }
```

Delete the private COSE_Key builder/parser helpers in `keys.rs` that `from_cose_key`/`to_cose_key` used (they now live in `arkavo-cwt::key`). Keep `der_to_p1363` and `PermitSigner::sign`. Any `keys.rs` unit test that exercised those helpers directly moves to asserting through `PermitVerifier::from_cose_key(&signer.cose_key())`.

- [ ] **Step 5: Delegate parsing in `permit.rs`**

Replace `parse_sign1` and `header_algorithm` with:

```rust
fn parse_sign1(cwt: &[u8]) -> Result<arkavo_cwt::ParsedSign1, PermitError> {
    if cwt.len() > MAX_PERMIT_BYTES {
        return Err(PermitError::Cose("permit exceeds maximum size".to_string()));
    }
    if !cwt.starts_with(&arkavo_cwt::sign1::CWT_TAG_PREFIX) {
        return Err(PermitError::Cose("missing CBOR tag 61 (CWT)".to_string()));
    }
    Ok(arkavo_cwt::sign1::parse(cwt)?)
}
```

and rewrite `decode` and `verify`:

```rust
pub fn decode(cwt: &[u8]) -> Result<Permit, PermitError> {
    let parsed = parse_sign1(cwt)?;
    extract(&parsed)
}

pub fn verify(cwt: &[u8], now: i64) -> Result<Permit, PermitError> {
    let parsed = parse_sign1(cwt)?;
    let permit = extract(&parsed)?;
    parsed.verify(&permit.confirmation_key.0)?;
    let claims = &permit.claims;
    if now < claims.not_before {
        return Err(PermitError::NotYetValid { nbf: claims.not_before, now });
    }
    if now >= claims.expires_at {
        return Err(PermitError::Expired { exp: claims.expires_at, now });
    }
    if claims.issued_at > now {
        return Err(PermitError::IssuedInFuture { iat: claims.issued_at, now });
    }
    Ok(permit)
}

fn extract(parsed: &arkavo_cwt::ParsedSign1) -> Result<Permit, PermitError> {
    let payload = parsed.payload()?;
    let value: Value = ciborium::from_reader(payload)
        .map_err(|e| PermitError::CborDeserialize(format!("claims set: {e}")))?;
    let (claims, cose_key) = PermitClaims::from_cbor_value(&value)?;
    let confirmation_key = PermitVerifier::from_cose_key(&cose_key)?;
    Ok(Permit { claims, confirmation_key })
}
```

Drop the now-unused `CoseSign1`, `RegisteredLabelWithPrivate`, `Algorithm` imports from `permit.rs`; `mint` keeps `CoseSign1Builder`, `HeaderBuilder`, `TaggedCborSerializable`, `CoapContentFormat`.

- [ ] **Step 6: Run the full crate including the vectors**

```bash
cargo test -q -p arkavo-permit
cargo test -q -p arkavo-permit --test vectors_test
cargo clippy -q -p arkavo-permit --all-targets -- -D warnings
```

Expected: 47 passed, vectors pass unchanged. If a vector fails, the wire format changed and Step 5 is wrong; do not regenerate the vectors.

- [ ] **Step 7: Add to CI, commit, PR, close #662**

Add to the `protocol` arm of `feature.yaml` (test and clippy steps): `cargo test --locked -p arkavo-permit` and `cargo clippy --locked -p arkavo-permit --lib --bins -- -D warnings`.

```bash
cargo fmt
git add -A crates/arkavo-permit .github/workflows/feature.yaml
git commit -m "Verify permits through arkavo-cwt instead of a private COSE parser"
git fetch origin main && git show origin/main:Cargo.toml | grep -m1 '^version'   # must be < 0.91.3
git push -u origin feature/permit-on-arkavo-cwt
gh pr create --repo arkavo-org/arkavo-edge --title "Add arkavo-permit: CWT permits with cnf proof-of-possession" --body-file - <<'EOF'
#662 rebased on main and on `arkavo-cwt`. Permit claims, minting, `cnf` extraction, and the nbf/exp/iat window are unchanged; COSE_Sign1 parsing and the verifying-key type now come from `arkavo-cwt`. The three published test vectors verify byte-for-byte unchanged.

Spec: `docs/permit-cwt-schema.md`. Tracking: #655 Epic 3.1.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
NEW=$(gh pr view --repo arkavo-org/arkavo-edge --json number --jq .number)
gh pr close 662 --repo arkavo-org/arkavo-edge --comment "Superseded by #$NEW, rebased on arkavo-cwt."
```

### Task B4: Tell #665 and #659 what to depend on

- [ ] **Step 1: Comment on #665 and #659**

```bash
CWT_PR=<number of the B2 PR>
gh pr comment 665 --repo arkavo-org/arkavo-edge --body "arkavo-cwt landed on main via #$CWT_PR with the crate extracted from this branch plus a shared \`sign1::parse\` and \`VerifyingKey\`. Please merge main, drop this branch's copy of \`crates/arkavo-cwt\` and the \`agent-cwt\` spec entry (both already on main), and re-run. Everything else here (agent keypair slot, refresh loop, registration change) stays as this PR's scope."
gh pr comment 659 --repo arkavo-org/arkavo-edge --body "Per #655 (2026-09-01 review, finding 1): the CWT parser in \`arkavo-authorization/src/cwt_verify.rs\` is now the fourth copy. Main has \`arkavo-cwt\` (#$CWT_PR) with \`sign1::parse\`, \`VerifyingKey\`, and key-set verification. Please rebuild \`CwtVerifier\` on it (keep \`cti\` and duplicate-key strictness as a layer on top) or close this PR and reopen once the proxy's \`CallContext\` carries a principal. The dispatcher question is also settled: \`arkavo-mcp-proxy\` is the enforcement point, not \`arkavo-mcp-runtime/server.rs\`."
```

---

## Phase C: Permit-bound dispatch gate in the MCP proxy (version 0.92.0)

Facts this phase relies on:

- `crates/arkavo-mcp-proxy` on `origin/feature/mcp-proxy-skeleton` (#658): `PolicyHook::evaluate(&CallContext) -> Decision`, `CallContext { tool_name, arguments }`, `Decision::{Allow, Deny { reason }}`, `POLICY_DENIED = -32000`, `McpProxy::spawn(ProxyConfig, Arc<dyn PolicyHook>)` and `run(reader, writer)`. `handle_tool_call` builds the context from `params["name"]` / `params["arguments"]`. No `arkavo-*` deps. Integration test uses the Python fixture `tests/fixtures/echo_mcp_server.py`.
- `SubsystemTimingRegistry::dispatch_gate` exists on `main` after Phase A. Recording API: `arkavo_observability::subsystem_timing::global_timing().dispatch_gate.record(ms: u64)`.
- `arkavo_permit::PermitClaims::verify_invocation(tool_name, &Value, HashAlgorithm)` checks the tool name and argument hash. `Budget.max_invocations: u64` is required and ≥ 1.
- MCP `params._meta` is the request-scoped metadata slot; the permit and proof travel there as base64url strings.
- The CLI dispatches on `args[0]` with a `match` in `crates/arkavo-cli/src/lib.rs`; there is no `mcp` arm today. `arkavo-cli` already depends on `arkavo-observability`.

### Task C1: Rebase the proxy skeleton

**Files:**
- Merges: `origin/feature/mcp-proxy-skeleton` (#658)

- [ ] **Step 1: Branch, merge, version 0.92.0**

```bash
git fetch origin main feature/mcp-proxy-skeleton
git checkout -b feature/mcp-proxy-permit-gate origin/main
git merge --no-ff origin/feature/mcp-proxy-skeleton
git checkout --ours Cargo.toml Cargo.lock
sed -i '' 's/^version = "0.91.3"$/version = "0.92.0"/' Cargo.toml
grep -n -m1 '^version' Cargo.toml    # Expected: version = "0.92.0"
grep -n 'arkavo-mcp-proxy' Cargo.toml   # Expected: present in members; add if not
cargo update --workspace && cargo check --locked -q -p arkavo-mcp-proxy
git add Cargo.toml Cargo.lock && git commit --no-edit
```

- [ ] **Step 2: Run the skeleton's tests**

Run: `cargo test -q -p arkavo-mcp-proxy`
Expected: 5 unit + 2 integration tests pass. The integration tests spawn `python3`; confirm with `python3 --version`.

### Task C2: Proof-of-possession over an invocation in `arkavo-permit`

> **Superseded as implemented:** the digest names the permit by `Permit::id`
> — the digest of its signed content — rather than by its wire bytes, so the
> proof and the gate's budget counter use one notion of "the same permit".
> The signatures below therefore take `permit_id: &[u8; 32]` where they say
> `permit_cwt: &[u8]`, and `verify_invocation_proof` needs no token bytes at
> all. `docs/permit-cwt-schema.md` carries the shipped definition.

**Files:**
- Create: `crates/arkavo-permit/src/pop.rs`
- Modify: `crates/arkavo-permit/src/lib.rs` (`mod pop; pub use pop::{invocation_digest, prove_invocation, verify_invocation_proof};`)
- Modify: `crates/arkavo-permit/src/error.rs` (`InvalidProof` variant)
- Modify: `docs/permit-cwt-schema.md` (new section)

**Interfaces:**
- Produces:
  - `invocation_digest(permit_cwt: &[u8], tool_name: &str, arguments: &Value, algorithm: HashAlgorithm) -> Vec<u8>`
  - `prove_invocation(signer: &PermitSigner, permit_cwt: &[u8], tool_name: &str, arguments: &Value, algorithm: HashAlgorithm) -> Vec<u8>` (raw signature: 64 bytes for both Ed25519 and P-256 r||s)
  - `verify_invocation_proof(permit: &Permit, permit_cwt: &[u8], tool_name: &str, arguments: &Value, proof: &[u8], algorithm: HashAlgorithm) -> Result<(), PermitError>`
  - `PermitError::InvalidProof`

- [ ] **Step 1: Write the failing tests**

Create `crates/arkavo-permit/src/pop.rs` with the test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::{Budget, PermitClaims};
    use crate::keys::PermitSigner;
    use crate::{argument_hash, mint, verify};
    use arkavo_crypto::AgentKeypair;
    use serde_json::json;

    const NOW: i64 = 1_700_000_060;

    fn permit_for(signer: &PermitSigner, tool: &str, args: &serde_json::Value) -> Vec<u8> {
        let claims = PermitClaims {
            issuer: "edge".into(),
            subject: "agent-1".into(),
            expires_at: NOW + 300,
            not_before: NOW - 60,
            issued_at: NOW - 60,
            agent_workload_id: "wl-1".into(),
            policy_bundle_hash: vec![7; 32],
            tool_name: tool.into(),
            argument_hash: argument_hash(args, HashAlgorithm::Sha256),
            data_classifications: vec![],
            budget: Budget { max_invocations: 3, token_ceiling: None, cost_micro_usd: None },
            sequence_state_hash: vec![9; 32],
            parent_permit: None,
        };
        mint(&claims, signer).unwrap()
    }

    #[test]
    fn proof_from_the_cnf_key_verifies() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 42});
        let cwt = permit_for(&signer, "github.merge_pr", &args);
        let proof = prove_invocation(&signer, &cwt, "github.merge_pr", &args, HashAlgorithm::Sha256);
        let permit = verify(&cwt, NOW).unwrap();
        verify_invocation_proof(&permit, &cwt, "github.merge_pr", &args, &proof, HashAlgorithm::Sha256).unwrap();
    }

    #[test]
    fn replay_with_different_arguments_is_rejected() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 42});
        let cwt = permit_for(&signer, "github.merge_pr", &args);
        let proof = prove_invocation(&signer, &cwt, "github.merge_pr", &args, HashAlgorithm::Sha256);
        let permit = verify(&cwt, NOW).unwrap();
        let other = json!({"pr": 43});
        assert!(matches!(
            verify_invocation_proof(&permit, &cwt, "github.merge_pr", &other, &proof, HashAlgorithm::Sha256),
            Err(PermitError::InvalidProof)
        ));
    }

    #[test]
    fn proof_from_another_agent_is_rejected() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let intruder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 42});
        let cwt = permit_for(&signer, "github.merge_pr", &args);
        let proof = prove_invocation(&intruder, &cwt, "github.merge_pr", &args, HashAlgorithm::Sha256);
        let permit = verify(&cwt, NOW).unwrap();
        assert!(matches!(
            verify_invocation_proof(&permit, &cwt, "github.merge_pr", &args, &proof, HashAlgorithm::Sha256),
            Err(PermitError::InvalidProof)
        ));
    }

    #[test]
    fn digest_is_domain_separated_and_deterministic() {
        let args = json!({"b": 1, "a": 2});
        let d1 = invocation_digest(b"permit", "t", &args, HashAlgorithm::Sha256);
        let d2 = invocation_digest(b"permit", "t", &json!({"a": 2, "b": 1}), HashAlgorithm::Sha256);
        assert_eq!(d1, d2);
        assert_ne!(d1, invocation_digest(b"permit", "u", &args, HashAlgorithm::Sha256));
        assert_ne!(d1, invocation_digest(b"other", "t", &args, HashAlgorithm::Sha256));
        assert_eq!(d1.len(), 32);
    }
}
```

Add `InvalidProof` to `PermitError` in `error.rs`:

```rust
    #[error("proof-of-possession does not verify under the permit's cnf key")]
    InvalidProof,
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -q -p arkavo-permit pop::`
Expected: compile error, `prove_invocation` not found.

- [ ] **Step 3: Implement**

Prepend to `pop.rs`:

```rust
//! Proof-of-possession over one invocation. The permit binds tool and
//! argument hash; the proof shows the caller holds the `cnf` private key
//! for exactly this permit and exactly these arguments, so a captured
//! permit is useless to anyone else and to the same agent with other args.

use crate::canonical::argument_hash;
use crate::error::PermitError;
use crate::hash::HashAlgorithm;
use crate::keys::PermitSigner;
use crate::permit::Permit;
use serde_json::Value;

const DOMAIN: &[u8] = b"arkavo-permit-pop/v1";

pub fn invocation_digest(permit_cwt: &[u8], tool_name: &str, arguments: &Value, algorithm: HashAlgorithm) -> Vec<u8> {
    let mut input = Vec::with_capacity(DOMAIN.len() + 32 + 8 + tool_name.len() + 32);
    input.extend_from_slice(DOMAIN);
    input.extend_from_slice(&algorithm.digest(permit_cwt));
    input.extend_from_slice(&(tool_name.len() as u64).to_be_bytes());
    input.extend_from_slice(tool_name.as_bytes());
    input.extend_from_slice(&argument_hash(arguments, algorithm));
    algorithm.digest(&input)
}

pub fn prove_invocation(signer: &PermitSigner, permit_cwt: &[u8], tool_name: &str, arguments: &Value, algorithm: HashAlgorithm) -> Vec<u8> {
    signer.sign(&invocation_digest(permit_cwt, tool_name, arguments, algorithm))
}

pub fn verify_invocation_proof(
    permit: &Permit,
    permit_cwt: &[u8],
    tool_name: &str,
    arguments: &Value,
    proof: &[u8],
    algorithm: HashAlgorithm,
) -> Result<(), PermitError> {
    let digest = invocation_digest(permit_cwt, tool_name, arguments, algorithm);
    permit
        .confirmation_key
        .verify(permit.confirmation_key.algorithm(), &digest, proof)
        .map_err(|_| PermitError::InvalidProof)
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -q -p arkavo-permit pop::`
Expected: 4 passed.

- [ ] **Step 5: Document and commit**

Append to `docs/permit-cwt-schema.md`:

```markdown
## Proof of Possession per Invocation

Each `tools/call` carries, beside the permit, a raw signature by the `cnf` key over

    H( "arkavo-permit-pop/v1" || H(permit_cwt) || len(tool_name) as u64 BE || tool_name || argument_hash )

where `H` and `argument_hash` use the same hash algorithm as the permit. The proof is 64 bytes for both Ed25519 and ES256 (r || s). A proof for different arguments, a different permit, or from a different key does not verify. Replay of an identical call is bounded by the permit's `max_invocations`, enforced by the dispatch gate.
```

```bash
cargo fmt
git add crates/arkavo-permit docs/permit-cwt-schema.md
git commit -m "Add proof-of-possession over an invocation to arkavo-permit"
```

### Task C3: The `arkavo-dispatch-gate` crate

**Files:**
- Create: `crates/arkavo-dispatch-gate/Cargo.toml`
- Create: `crates/arkavo-dispatch-gate/src/lib.rs`
- Modify: `Cargo.toml` (members)

**Interfaces:**
- Consumes: `arkavo_permit::{verify, verify_invocation_proof, HashAlgorithm, Permit}`.
- Produces:

```rust
pub struct GateConfig { pub policy_bundle_hash: Vec<u8>, pub hash: HashAlgorithm, pub clock: fn() -> i64 }
pub struct GateRequest<'a> { pub tool_name: &'a str, pub arguments: &'a serde_json::Value, pub permit: &'a [u8], pub proof: &'a [u8] }
pub enum Stage { Authn, Policy, Budget }           // Display: "authn" | "policy" | "budget"
pub enum GateDecision { Allow { permit_id: [u8; 32], subject: String }, Deny { stage: Stage, reason: String } }
pub struct DispatchGate { .. }
impl DispatchGate { pub fn new(config: GateConfig) -> Self; pub fn evaluate(&self, request: &GateRequest<'_>) -> GateDecision; }
pub fn unix_now() -> i64;
```

- [ ] **Step 1: Create the crate manifest**

```toml
[package]
name = "arkavo-dispatch-gate"
version.workspace = true
edition.workspace = true
license.workspace = true

[lints]
workspace = true

[dependencies]
arkavo-permit = { path = "../arkavo-permit" }
serde_json = { workspace = true }
sha2 = { workspace = true }
thiserror = { workspace = true }

[dev-dependencies]
arkavo-crypto = { path = "../arkavo-crypto" }
```

Check `edition.workspace`/`license.workspace` keys match another crate (`sed -n '1,12p' crates/arkavo-cwt/Cargo.toml`) and copy that header verbatim. Add `"crates/arkavo-dispatch-gate",` to `members`.

- [ ] **Step 2: Write the failing tests**

`crates/arkavo-dispatch-gate/src/lib.rs`, test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_crypto::AgentKeypair;
    use arkavo_permit::{argument_hash, mint, prove_invocation, Budget, PermitClaims, PermitSigner};
    use serde_json::json;

    const NOW: i64 = 1_700_000_060;
    fn clock() -> i64 { NOW }

    fn gate() -> DispatchGate {
        DispatchGate::new(GateConfig { policy_bundle_hash: vec![7; 32], hash: HashAlgorithm::Sha256, clock })
    }

    fn permit(signer: &PermitSigner, tool: &str, args: &serde_json::Value, max: u64, exp: i64, bundle: u8) -> Vec<u8> {
        let claims = PermitClaims {
            issuer: "edge".into(),
            subject: "agent-1".into(),
            expires_at: exp,
            not_before: NOW - 60,
            issued_at: NOW - 60,
            agent_workload_id: "wl-1".into(),
            policy_bundle_hash: vec![bundle; 32],
            tool_name: tool.into(),
            argument_hash: argument_hash(args, HashAlgorithm::Sha256),
            data_classifications: vec![],
            budget: Budget { max_invocations: max, token_ceiling: None, cost_micro_usd: None },
            sequence_state_hash: vec![9; 32],
            parent_permit: None,
        };
        mint(&claims, signer).unwrap()
    }

    fn call<'a>(tool: &'a str, args: &'a serde_json::Value, cwt: &'a [u8], proof: &'a [u8]) -> GateRequest<'a> {
        GateRequest { tool_name: tool, arguments: args, permit: cwt, proof }
    }

    #[test]
    fn valid_permit_and_proof_allow() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&signer, "merge", &args, 2, NOW + 300, 7);
        let proof = prove_invocation(&signer, &cwt, "merge", &args, HashAlgorithm::Sha256);
        match gate().evaluate(&call("merge", &args, &cwt, &proof)) {
            GateDecision::Allow { subject, .. } => assert_eq!(subject, "agent-1"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn expired_permit_is_denied_at_authn() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({});
        let cwt = permit(&signer, "merge", &args, 2, NOW - 1, 7);
        let proof = prove_invocation(&signer, &cwt, "merge", &args, HashAlgorithm::Sha256);
        assert!(matches!(gate().evaluate(&call("merge", &args, &cwt, &proof)), GateDecision::Deny { stage: Stage::Authn, .. }));
    }

    #[test]
    fn replay_with_different_args_is_denied_at_authn() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&signer, "merge", &args, 2, NOW + 300, 7);
        let proof = prove_invocation(&signer, &cwt, "merge", &args, HashAlgorithm::Sha256);
        let other = json!({"pr": 2});
        assert!(matches!(gate().evaluate(&call("merge", &other, &cwt, &proof)), GateDecision::Deny { stage: Stage::Authn, .. }));
    }

    #[test]
    fn cross_agent_reuse_is_denied_at_authn() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let intruder = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&signer, "merge", &args, 2, NOW + 300, 7);
        let proof = prove_invocation(&intruder, &cwt, "merge", &args, HashAlgorithm::Sha256);
        assert!(matches!(gate().evaluate(&call("merge", &args, &cwt, &proof)), GateDecision::Deny { stage: Stage::Authn, .. }));
    }

    #[test]
    fn foreign_policy_bundle_is_denied_at_policy() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({});
        let cwt = permit(&signer, "merge", &args, 2, NOW + 300, 8);
        let proof = prove_invocation(&signer, &cwt, "merge", &args, HashAlgorithm::Sha256);
        assert!(matches!(gate().evaluate(&call("merge", &args, &cwt, &proof)), GateDecision::Deny { stage: Stage::Policy, .. }));
    }

    #[test]
    fn budget_exhaustion_is_denied_at_budget() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({});
        let cwt = permit(&signer, "merge", &args, 1, NOW + 300, 7);
        let proof = prove_invocation(&signer, &cwt, "merge", &args, HashAlgorithm::Sha256);
        let gate = gate();
        assert!(matches!(gate.evaluate(&call("merge", &args, &cwt, &proof)), GateDecision::Allow { .. }));
        assert!(matches!(gate.evaluate(&call("merge", &args, &cwt, &proof)), GateDecision::Deny { stage: Stage::Budget, .. }));
    }

    #[test]
    fn evaluation_stays_under_the_gate_budget() {
        let signer = PermitSigner::Ed25519(AgentKeypair::generate());
        let args = json!({"pr": 1});
        let cwt = permit(&signer, "merge", &args, 10_000, NOW + 300, 7);
        let proof = prove_invocation(&signer, &cwt, "merge", &args, HashAlgorithm::Sha256);
        let gate = gate();
        let mut samples: Vec<u128> = (0..200)
            .map(|_| {
                let start = std::time::Instant::now();
                let _ = gate.evaluate(&call("merge", &args, &cwt, &proof));
                start.elapsed().as_micros()
            })
            .collect();
        samples.sort_unstable();
        let p95 = samples[189];
        assert!(p95 < 25_000, "p95 {p95}µs exceeds the 25ms gate budget");
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -q -p arkavo-dispatch-gate`
Expected: compile error, `DispatchGate` not found.

- [ ] **Step 4: Implement**

Prepend to `lib.rs`:

```rust
//! The dispatch gate: authn (permit signature, window, proof-of-possession),
//! policy (bundle hash, tool and argument binding), budget (invocations per
//! permit). Local crypto only, no I/O, so it fits inside the 25ms budget
//! documented in `docs/gate-latency-baseline.md`. Sequence integrity and
//! step-up are later stages and plug in before `Allow` is returned.

use arkavo_permit::{verify, verify_invocation_proof, HashAlgorithm};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fmt;
use std::sync::Mutex;

pub struct GateConfig {
    pub policy_bundle_hash: Vec<u8>,
    pub hash: HashAlgorithm,
    pub clock: fn() -> i64,
}

pub struct GateRequest<'a> {
    pub tool_name: &'a str,
    pub arguments: &'a Value,
    pub permit: &'a [u8],
    pub proof: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Authn,
    Policy,
    Budget,
}

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Authn => "authn",
            Self::Policy => "policy",
            Self::Budget => "budget",
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GateDecision {
    Allow { permit_id: [u8; 32], subject: String },
    Deny { stage: Stage, reason: String },
}

struct Usage {
    invocations: u64,
    expires_at: i64,
}

/// Counters are keyed by permit digest and pruned by expiry once the map
/// grows, because a caller can mint arbitrarily many permits and the
/// table must not become a memory sink.
const PRUNE_ABOVE: usize = 4096;

pub struct DispatchGate {
    config: GateConfig,
    usage: Mutex<HashMap<[u8; 32], Usage>>,
}

impl DispatchGate {
    pub fn new(config: GateConfig) -> Self {
        Self { config, usage: Mutex::new(HashMap::new()) }
    }

    pub fn evaluate(&self, request: &GateRequest<'_>) -> GateDecision {
        let now = (self.config.clock)();

        let permit = match verify(request.permit, now) {
            Ok(permit) => permit,
            Err(error) => return deny(Stage::Authn, error.to_string()),
        };
        if let Err(error) = verify_invocation_proof(
            &permit,
            request.permit,
            request.tool_name,
            request.arguments,
            request.proof,
            self.config.hash,
        ) {
            return deny(Stage::Authn, error.to_string());
        }

        if permit.claims.policy_bundle_hash != self.config.policy_bundle_hash {
            return deny(Stage::Policy, "permit was issued under a different policy bundle".into());
        }
        if let Err(error) = permit
            .claims
            .verify_invocation(request.tool_name, request.arguments, self.config.hash)
        {
            return deny(Stage::Policy, error.to_string());
        }

        let permit_id = permit_id(request.permit);
        let mut usage = self.usage.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if usage.len() > PRUNE_ABOVE {
            usage.retain(|_, entry| entry.expires_at > now);
        }
        let entry = usage.entry(permit_id).or_insert(Usage { invocations: 0, expires_at: permit.claims.expires_at });
        if entry.invocations >= permit.claims.budget.max_invocations {
            return deny(Stage::Budget, format!("invocation budget of {} exhausted", permit.claims.budget.max_invocations));
        }
        entry.invocations += 1;

        GateDecision::Allow { permit_id, subject: permit.claims.subject.clone() }
    }
}

pub fn permit_id(permit_cwt: &[u8]) -> [u8; 32] {
    Sha256::digest(permit_cwt).into()
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn deny(stage: Stage, reason: String) -> GateDecision {
    GateDecision::Deny { stage, reason }
}
```

- [ ] **Step 5: Run the tests and clippy**

```bash
cargo test -q -p arkavo-dispatch-gate
cargo clippy -q -p arkavo-dispatch-gate --all-targets -- -D warnings
```

Expected: 7 passed. If the workspace clippy config forbids `unwrap_or_else` on a poisoned mutex, replace with `.expect("gate usage table is never poisoned")` and keep the comment.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add Cargo.toml Cargo.lock crates/arkavo-dispatch-gate
git commit -m "Add arkavo-dispatch-gate: authn, policy, and budget over a permit"
```

### Task C4: Plug the gate into the proxy as a `PolicyHook`

**Files:**
- Modify: `crates/arkavo-mcp-proxy/Cargo.toml` (deps: `arkavo-dispatch-gate`, `arkavo-permit`, `arkavo-observability`, `base64`)
- Modify: `crates/arkavo-mcp-proxy/src/policy.rs` (`CallContext` gains `permit`, `proof`)
- Modify: `crates/arkavo-mcp-proxy/src/proxy.rs:174-185` (`handle_tool_call` fills them from `_meta`)
- Create: `crates/arkavo-mcp-proxy/src/permit_hook.rs`
- Modify: `crates/arkavo-mcp-proxy/src/lib.rs` (`mod permit_hook; pub use permit_hook::PermitPolicy;`)
- Modify: `crates/arkavo-mcp-proxy/tests/stdio_passthrough.rs` (new test)

**Interfaces:**
- Consumes: `arkavo_dispatch_gate::{DispatchGate, GateConfig, GateRequest, GateDecision}`, `global_timing().dispatch_gate.record(u64)`.
- Produces: `CallContext { tool_name, arguments, permit: Option<Vec<u8>>, proof: Option<Vec<u8>> }`; `PermitPolicy::new(DispatchGate) -> Self` implementing `PolicyHook`; wire format `params._meta.arkavo.permit` and `params._meta.arkavo.pop`, both base64url without padding.

- [ ] **Step 1: Extend `CallContext` and its construction**

In `policy.rs`:

```rust
#[derive(Debug, Clone)]
pub struct CallContext {
    /// Name of the tool being invoked.
    pub tool_name: String,
    /// Arguments supplied by the caller.
    pub arguments: Value,
    /// Raw CWT permit bytes from `params._meta.arkavo.permit`, if present.
    pub permit: Option<Vec<u8>>,
    /// Raw proof-of-possession signature from `params._meta.arkavo.pop`.
    pub proof: Option<Vec<u8>>,
}
```

In `proxy.rs` `handle_tool_call`, after `tool_name` and `arguments` are read:

```rust
        let meta = params.and_then(|p| p.get("_meta")).and_then(|m| m.get("arkavo"));
        let permit = meta.and_then(|m| m.get("permit")).and_then(Value::as_str).and_then(decode_b64url);
        let proof = meta.and_then(|m| m.get("pop")).and_then(Value::as_str).and_then(decode_b64url);
        let ctx = CallContext { tool_name, arguments, permit, proof };
```

with a helper at the bottom of `proxy.rs`:

```rust
fn decode_b64url(text: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(text).ok()
}
```

Update the two existing `CallContext { .. }` literals in `policy.rs` tests with `permit: None, proof: None`. Add `base64 = { workspace = true }` to the proxy's `[dependencies]`.

- [ ] **Step 2: Write the failing integration test**

Append to `tests/stdio_passthrough.rs`:

```rust
#[tokio::test]
async fn permit_bound_call_is_allowed_once_and_refused_on_replay_or_tamper() {
    use arkavo_crypto::AgentKeypair;
    use arkavo_dispatch_gate::{DispatchGate, GateConfig};
    use arkavo_mcp_proxy::PermitPolicy;
    use arkavo_permit::{argument_hash, mint, prove_invocation, Budget, HashAlgorithm, PermitClaims, PermitSigner};
    use base64::Engine as _;

    let now = arkavo_dispatch_gate::unix_now();
    let signer = PermitSigner::Ed25519(AgentKeypair::generate());
    let args = json!({"n": 1});
    let claims = PermitClaims {
        issuer: "edge".into(),
        subject: "agent-1".into(),
        expires_at: now + 300,
        not_before: now - 60,
        issued_at: now - 60,
        agent_workload_id: "wl-1".into(),
        policy_bundle_hash: vec![7; 32],
        tool_name: "echo".into(),
        argument_hash: argument_hash(&args, HashAlgorithm::Sha256),
        data_classifications: vec![],
        budget: Budget { max_invocations: 1, token_ceiling: None, cost_micro_usd: None },
        sequence_state_hash: vec![9; 32],
        parent_permit: None,
    };
    let cwt = mint(&claims, &signer).unwrap();
    let proof = prove_invocation(&signer, &cwt, "echo", &args, HashAlgorithm::Sha256);
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let meta = json!({"arkavo": {"permit": b64(&cwt), "pop": b64(&proof)}});

    let gate = DispatchGate::new(GateConfig {
        policy_bundle_hash: vec![7; 32],
        hash: HashAlgorithm::Sha256,
        clock: arkavo_dispatch_gate::unix_now,
    });
    let (mut client, handle) = start_proxy(fixture_config(), Arc::new(PermitPolicy::new(gate)));
    client.handshake().await;

    let allowed = client
        .request("tools/call", Some(json!({"name": "echo", "arguments": args, "_meta": meta})))
        .await;
    assert!(allowed.get("error").is_none(), "first call must pass: {allowed}");

    let replay = client
        .request("tools/call", Some(json!({"name": "echo", "arguments": args, "_meta": meta})))
        .await;
    assert_eq!(replay["error"]["code"], POLICY_DENIED);
    assert!(replay["error"]["message"].as_str().unwrap().contains("budget"));

    let tampered = client
        .request("tools/call", Some(json!({"name": "echo", "arguments": {"n": 2}, "_meta": meta})))
        .await;
    assert_eq!(tampered["error"]["code"], POLICY_DENIED);
    assert!(tampered["error"]["message"].as_str().unwrap().contains("authn"));

    let bare = client
        .request("tools/call", Some(json!({"name": "echo", "arguments": {"n": 3}})))
        .await;
    assert_eq!(bare["error"]["code"], POLICY_DENIED);
    assert!(bare["error"]["message"].as_str().unwrap().contains("no permit"));

    drop(client);
    handle.await.unwrap().unwrap();
}
```

Add to the proxy's `[dev-dependencies]`: `arkavo-crypto = { path = "../arkavo-crypto" }`, `arkavo-permit = { path = "../arkavo-permit" }`, `arkavo-dispatch-gate = { path = "../arkavo-dispatch-gate" }`. Check how the existing tests end a session (whether `drop(client)` is what closes the duplex) and mirror it.

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -q -p arkavo-mcp-proxy --test stdio_passthrough permit_bound`
Expected: compile error, `PermitPolicy` not found.

- [ ] **Step 4: Implement `permit_hook.rs`**

```rust
//! The `PolicyHook` that runs the dispatch gate on every `tools/call` and
//! records its latency on the `dispatch_gate` tracker so the 25ms budget
//! is visible in the AG-UI health panel.

use crate::policy::{CallContext, Decision, PolicyHook};
use arkavo_dispatch_gate::{DispatchGate, GateDecision, GateRequest};
use arkavo_observability::subsystem_timing::global_timing;
use async_trait::async_trait;
use std::time::Instant;

pub struct PermitPolicy {
    gate: DispatchGate,
}

impl PermitPolicy {
    pub fn new(gate: DispatchGate) -> Self {
        Self { gate }
    }
}

#[async_trait]
impl PolicyHook for PermitPolicy {
    async fn evaluate(&self, ctx: &CallContext) -> Decision {
        let started = Instant::now();
        let decision = match (&ctx.permit, &ctx.proof) {
            (Some(permit), Some(proof)) => {
                let request = GateRequest {
                    tool_name: &ctx.tool_name,
                    arguments: &ctx.arguments,
                    permit,
                    proof,
                };
                match self.gate.evaluate(&request) {
                    GateDecision::Allow { .. } => Decision::Allow,
                    GateDecision::Deny { stage, reason } => Decision::Deny { reason: format!("{stage}: {reason}") },
                }
            }
            _ => Decision::Deny { reason: "authn: tools/call carries no permit and proof in _meta.arkavo".into() },
        };
        global_timing()
            .dispatch_gate
            .record(started.elapsed().as_millis() as u64);
        decision
    }
}
```

Add to `[dependencies]`: `arkavo-dispatch-gate = { path = "../arkavo-dispatch-gate" }`, `arkavo-observability = { path = "../arkavo-observability" }`. Register in `lib.rs`:

```rust
mod permit_hook;
pub use permit_hook::PermitPolicy;
```

- [ ] **Step 5: Run the whole proxy crate**

```bash
cargo test -q -p arkavo-mcp-proxy
cargo clippy -q -p arkavo-mcp-proxy --all-targets -- -D warnings
```

Expected: 8 tests pass (5 unit, 3 integration). The `denied_tool_call_never_reaches_upstream` test proves a deny still never reaches the fixture.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/arkavo-mcp-proxy Cargo.lock
git commit -m "Gate every proxied tools/call on a permit and proof-of-possession"
```

### Task C5: `arkavo mcp proxy` command

**Files:**
- Create: `crates/arkavo-cli/src/commands/mcp_proxy.rs`
- Modify: `crates/arkavo-cli/src/commands/mod.rs` (`pub mod mcp_proxy;`)
- Modify: `crates/arkavo-cli/src/lib.rs` (`"mcp"` arm)
- Modify: `crates/arkavo-cli/Cargo.toml` (deps)

**Interfaces:**
- Produces: `arkavo mcp proxy --policy-bundle-hash <64 hex> [--hash sha256|blake3] -- <upstream command> [args...]`. Reads MCP JSON-RPC lines on stdin, writes to stdout, spawns the upstream. Exit code 2 on usage error.
- `pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>>` and `fn parse(args: &[String]) -> Result<ProxyArgs, String>`.

- [ ] **Step 1: Write the failing parse tests**

Create `mcp_proxy.rs` with:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|i| i.to_string()).collect()
    }

    #[test]
    fn parses_bundle_hash_hash_alg_and_upstream() {
        let hex = "07".repeat(32);
        let parsed = parse(&s(&["proxy", "--policy-bundle-hash", &hex, "--hash", "blake3", "--", "python3", "srv.py", "--flag"])).unwrap();
        assert_eq!(parsed.policy_bundle_hash, vec![7u8; 32]);
        assert_eq!(parsed.hash, arkavo_permit::HashAlgorithm::Blake3);
        assert_eq!(parsed.command, "python3");
        assert_eq!(parsed.args, s(&["srv.py", "--flag"]));
    }

    #[test]
    fn defaults_to_sha256() {
        let hex = "07".repeat(32);
        let parsed = parse(&s(&["proxy", "--policy-bundle-hash", &hex, "--", "cmd"])).unwrap();
        assert_eq!(parsed.hash, arkavo_permit::HashAlgorithm::Sha256);
    }

    #[test]
    fn rejects_missing_upstream_and_bad_hash() {
        let hex = "07".repeat(32);
        assert!(parse(&s(&["proxy", "--policy-bundle-hash", &hex])).is_err());
        assert!(parse(&s(&["proxy", "--policy-bundle-hash", "zz", "--", "cmd"])).is_err());
        assert!(parse(&s(&["proxy", "--", "cmd"])).is_err());
        assert!(parse(&s(&["other"])).is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -q -p arkavo-cli mcp_proxy`
Expected: compile error (module not registered / `parse` missing). Add `pub mod mcp_proxy;` to `commands/mod.rs` first if the error is about the module.

- [ ] **Step 3: Implement**

Prepend to `mcp_proxy.rs`:

```rust
//! `arkavo mcp proxy`: stdio MCP relay that admits a `tools/call` only with
//! a valid permit and proof-of-possession. Configuration is flags only;
//! the policy bundle hash pins which bundle the permits must cite.

use arkavo_dispatch_gate::{unix_now, DispatchGate, GateConfig};
use arkavo_mcp_proxy::{McpProxy, PermitPolicy, ProxyConfig};
use arkavo_permit::HashAlgorithm;
use std::sync::Arc;

const USAGE: &str = "usage: arkavo mcp proxy --policy-bundle-hash <64 hex> [--hash sha256|blake3] -- <upstream command> [args...]";

pub struct ProxyArgs {
    pub policy_bundle_hash: Vec<u8>,
    pub hash: HashAlgorithm,
    pub command: String,
    pub args: Vec<String>,
}

pub fn execute(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let parsed = match parse(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("{message}\n{USAGE}");
            std::process::exit(2);
        }
    };
    let runtime = tokio::runtime::Runtime::new()?;
    runtime.block_on(run(parsed))
}

async fn run(parsed: ProxyArgs) -> Result<(), Box<dyn std::error::Error>> {
    let gate = DispatchGate::new(GateConfig {
        policy_bundle_hash: parsed.policy_bundle_hash,
        hash: parsed.hash,
        clock: unix_now,
    });
    let config = ProxyConfig::new(parsed.command, parsed.args);
    let proxy = McpProxy::spawn(config, Arc::new(PermitPolicy::new(gate)))?;
    let stdin = tokio::io::BufReader::new(tokio::io::stdin());
    let stdout = tokio::io::stdout();
    proxy.run(stdin, stdout).await?;
    Ok(())
}

fn parse(args: &[String]) -> Result<ProxyArgs, String> {
    if args.first().map(String::as_str) != Some("proxy") {
        return Err("unknown mcp subcommand; only `proxy` is available".into());
    }
    let mut policy_bundle_hash = None;
    let mut hash = HashAlgorithm::Sha256;
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--policy-bundle-hash" => {
                let value = args.get(index + 1).ok_or("--policy-bundle-hash needs a value")?;
                let bytes = decode_hex(value)?;
                if bytes.len() != 32 {
                    return Err("--policy-bundle-hash must be 32 bytes (64 hex)".into());
                }
                policy_bundle_hash = Some(bytes);
                index += 2;
            }
            "--hash" => {
                let value = args.get(index + 1).ok_or("--hash needs a value")?;
                hash = HashAlgorithm::from_name(value).ok_or_else(|| format!("unknown hash {value}"))?;
                index += 2;
            }
            "--" => {
                let command = args.get(index + 1).ok_or("missing upstream command after --")?.clone();
                let rest = args[index + 2..].to_vec();
                let policy_bundle_hash = policy_bundle_hash.ok_or("--policy-bundle-hash is required")?;
                return Ok(ProxyArgs { policy_bundle_hash, hash, command, args: rest });
            }
            other => return Err(format!("unknown flag {other}")),
        }
    }
    Err("missing `-- <upstream command>`".into())
}

fn decode_hex(text: &str) -> Result<Vec<u8>, String> {
    if text.len() % 2 != 0 {
        return Err("hex has odd length".into());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}
```

Check `ProxyConfig::new`'s signature at `crates/arkavo-mcp-proxy/src/proxy.rs:36` and match it. Add to `crates/arkavo-cli/Cargo.toml`: `arkavo-mcp-proxy = { path = "../arkavo-mcp-proxy" }`, `arkavo-dispatch-gate = { path = "../arkavo-dispatch-gate" }`, `arkavo-permit = { path = "../arkavo-permit" }`.

In `crates/arkavo-cli/src/lib.rs`, add an arm to the `match args[0].as_str()` after `"ui"`:

```rust
        "mcp" => commands::mcp_proxy::execute(&args[1..]),
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -q -p arkavo-cli mcp_proxy`
Expected: 3 passed.

- [ ] **Step 5: Smoke test against the fixture**

```bash
cargo build -q -p arkavo
printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"echo","arguments":{}}}' \
  | ./target/debug/arkavo mcp proxy --policy-bundle-hash $(printf '07%.0s' $(seq 32)) -- python3 crates/arkavo-mcp-proxy/tests/fixtures/echo_mcp_server.py
```

Expected: the first line is an `initialize` result; the second is an error with code `-32000` whose message contains `no permit`.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/arkavo-cli Cargo.lock
git commit -m "Add arkavo mcp proxy, a permit-gated stdio MCP relay"
```

### Task C6: Docs, CI, PR, close #658, update #655

**Files:**
- Create: `docs/dispatch-gate.md`
- Modify: `.github/workflows/feature.yaml` (`protocol` arm gains `arkavo-dispatch-gate` and `arkavo-mcp-proxy`)

- [ ] **Step 1: Write `docs/dispatch-gate.md`**

```markdown
# Dispatch Gate

Every `tools/call` that passes through `arkavo mcp proxy` is admitted by three local stages, in order. No stage does I/O; the p95 budget is 25ms (`docs/gate-latency-baseline.md`), and the observed latency is recorded on `dispatch_gate` in the subsystem timing registry and shown in the AG-UI health panel.

## Wire format

The client places two base64url (no padding) strings under `params._meta.arkavo`:

- `permit`: the CWT permit (`docs/permit-cwt-schema.md`)
- `pop`: the proof-of-possession signature over this invocation (same document, "Proof of Possession per Invocation")

A call without both is refused before any stage runs.

## Stages

| Stage | Checks | Deny message prefix |
|---|---|---|
| authn | permit signature against `cnf`, `nbf`/`exp`/`iat` at now, proof-of-possession over permit, tool, and arguments | `authn:` |
| policy | permit's `policy_bundle_hash` equals the proxy's configured bundle; tool name and argument hash match the permit | `policy:` |
| budget | invocations of this permit (keyed by SHA-256 of the permit bytes) stay below `budget.max_invocations` | `budget:` |

A refused call returns JSON-RPC error `-32000` and never reaches the upstream server.

## Running it

    arkavo mcp proxy --policy-bundle-hash <64 hex> [--hash sha256|blake3] -- <upstream command> [args...]

## Not yet wired

Sequence-integrity (Epic 5.1), step-up approval (3.4), and closure receipts (3.5) attach between the budget stage and `Allow`. Token and cost ceilings in the permit budget are carried but not enforced at dispatch, because the gate has no token counts.
```

- [ ] **Step 2: CI arms**

In `feature.yaml` `protocol` arm (test step): add `cargo test --locked -p arkavo-dispatch-gate` and `cargo test --locked -p arkavo-mcp-proxy`; (clippy step): the matching `cargo clippy --locked -p <crate> --lib --bins -- -D warnings` lines.

- [ ] **Step 3: Full checklist, PR**

```bash
cargo fmt -- --check && cargo build -q && cargo clippy -- -D warnings
cargo test -q -p arkavo-protocol --test security_vulnerabilities
cargo test -q -p arkavo-cli mock_provider
git add docs/dispatch-gate.md .github/workflows/feature.yaml
git commit -m "Document the dispatch gate and run its crates in CI"
git fetch origin main && git show origin/main:Cargo.toml | grep -m1 '^version'   # must be < 0.92.0
git push -u origin feature/mcp-proxy-permit-gate
gh pr create --repo arkavo-org/arkavo-edge --title "Permit-bound dispatch gate in arkavo mcp proxy" --body-file - <<'EOF'
Lands #658's proxy skeleton on main and makes it enforce permits:

- `arkavo-permit`: proof-of-possession over an invocation (`pop.rs`)
- `arkavo-dispatch-gate`: authn → policy → budget, local crypto only, 7 tests including the 25ms p95 check and the three 3.3 BDD cases (replay with different args, cross-agent reuse, expired permit)
- `arkavo-mcp-proxy`: `PermitPolicy` hook, `_meta.arkavo.{permit,pop}` transport, latency on `dispatch_gate`
- `arkavo mcp proxy` command

Docs: `docs/dispatch-gate.md`. Tracking: #655 Epics 1.2, 3.2, 3.3.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
NEW=$(gh pr view --repo arkavo-org/arkavo-edge --json number --jq .number)
gh pr close 658 --repo arkavo-org/arkavo-edge --comment "Superseded by #$NEW (skeleton merged onto main plus the permit gate)."
```

- [ ] **Step 4: Update the tracking issue**

Post a comment on #655 with: the four PR numbers and versions, checkboxes to flip (1.1 health endpoints, 1.2 stdio proxy, 3.1, 3.2 first slice, 3.3), and the explicit list of what remains (SEQ stage, 3.4, 3.5, 2.1, Helm, streamable-HTTP transport). Edit the issue body to tick 3.1, 3.3, and mark 1.1/1.2/3.2 partial with the PR numbers.

---

## Self-review notes

- Spec coverage: Epic 1.1 (health probes: A6; Helm out of scope, stated), 1.2 (stdio proxy: C1, C4, C5; HTTP transport and OAuth out of scope, stated), 3.1 (B3), 3.2 (C3, C4, latency in C4), 3.3 (C2, C3 tests, C4 integration test). Finding 1 of the 2026-09-01 review (four verifiers): B1, B2, B3, B4. Finding 2 (bind address): A1. Finding 4 (competing gate locations): B4 comment on #659.
- Type consistency: `PermitVerifier(pub arkavo_cwt::VerifyingKey)` in B3 is what C2's `verify_invocation_proof` calls `.verify(algorithm, data, sig)` on and what C3's gate reads via `arkavo_permit::verify`. `GateRequest` field names match between C3 and C4. `Decision::Deny { reason }` is the skeleton's existing shape. `HashAlgorithm::from_name` exists on the #662 branch and is used in C5.
- Every version bump is guarded by a fetch-and-compare step because the DLP track keeps moving `main`.
