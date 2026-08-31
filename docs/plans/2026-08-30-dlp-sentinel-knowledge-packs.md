# DLP Sentinel and Knowledge Packs — Implementation Plan

**Status:** accepted · **Date:** 2026-08-30
**Closes toward:** #548 (taint-aware egress), #549 (taint tagging/propagation), #556 (sequence graph), #552 (baselines, partial)
**Specs:** `sequence-integrity.spec.yaml` (SEQ-001..017), `knowledge-pack.spec.yaml` (KP-001..017), `sentinel.spec.yaml` (SENT-001..016), `tdf-security.spec.yaml`, `docs/gguf-tdf/opentdf-gguf-profile-design.md`
**Builds on:** 0.90.0 GGUF-TDF (`arkavo-gguf-tdf`), the small-stories fine-tune draft PR

## Thesis

One distillation pipeline produces a **sealed knowledge pack**: policy-scoped knowledge adapters, a sentinel classifier, keyed reference indices, and a signed manifest. Pre-generation ABAC (existing per-role TDF release machinery, extended to the weights channel) is the security boundary; the sentinel and egress gates are defense-in-depth that implement the already-specified SEQ scenarios. The sentinel **labels; the PDP authorizes** — it is the pluggable "classification level inferred" step of SEQ-001 and the classifier behind SEQ-003, never a second authorization engine.

## Ground rules

1. This repo is spec-driven: every behavior change traces to a scenario id in `specs/arkavo-edge/*.spec.yaml`, tests carry `#[spec("SEQ-003")]` (see `arkavo-test-macros`), and unimplemented spec behavior is documented by `#[should_panic]` **tripwire tests** (e.g. `crates/arkavo-validation/tests/sequence_integrity_test.rs`). Flip a tripwire only when the behavior is genuinely green; never delete one to make CI pass.
2. Invariants that must never regress:
   - Sequence checks add **<50µs per tool call** (SEQ invariant). Anything heavier runs off the hot path (async, holdback, or build-time).
   - **ABAC evaluated before key release** (`tdf-security.spec.yaml`); keys zeroized (`gguf-tdf/src/key.rs` pattern).
   - Conservative classification on ambiguity; **highest classification propagates** on merge; upstream taint inherited (SEQ-001/002 edge cases). Inferred labels may ADD restrictions, never remove known ones.
   - Attribute policies are conjunctive across definitions (`PolicyBuilder::attribute_single` chaining, `swarmkit-runtime/src/tdf.rs`).
3. All new behavior lands behind cargo features `taint`, `sentinel`, `knowledge-pack` until GA. No secrets, corpora, or plaintext intermediate artifacts committed — ever.
4. Phase gate (run before closing any phase): `cargo fmt --check` · `cargo clippy -- -D warnings` · `cargo deny check` · spec schema validation against `specs/schema.json` · `ARKAVO_MOCK_PROVIDER=1 ./tests/dlp_pii_security_test.sh` · `./tests/e2e_security_test.sh` · crate benches within budget.
5. One PR series per phase, titles referencing SEQ/KP/SENT ids. Update each spec's `refs:` and `changed:` fields alongside the implementation.
6. Surface open decisions to the maintainer instead of choosing silently.

## Phase 0 — Specs and tripwires (no behavior change)

**Goal:** the whole initiative exists as reviewable spec surface before code.

- `specs/arkavo-edge/knowledge-pack.spec.yaml` (KP-001..017): pack = adapter(s) + sentinel + indices, each TDF-wrapped **separately**; signed manifest binds corpus snapshot digest, taxonomy-map version, tokenizer, calibration thresholds, artifact digests, parent lineage, eval-evidence digest. High-water-mark rule: a merged/mixed-corpus model carries the max classification of its training corpus. Egress services may receive sentinel and index without the knowledge model.
- `specs/arkavo-edge/sentinel.spec.yaml` (SENT-001..016): evidence contract `{labels, calibrated_confidence, detector_version, taxonomy_version, signals, source_families?}`; invariants — sentinel never authorizes; monotonic union with known labels; oracle protections (token-bucket rate limit, no raw scores across trust boundary, generic denial text); declassification is a signed human workflow only.
- `sequence-integrity.spec.yaml`: `refs:` extended to the crates this plan touches; `wip: true` retained until each flip.
- `schemas/taxonomy-map.v1.schema.json` and the v1 instance `schemas/taxonomy-map.v1.json`: versioned taxonomy-label → OpenTDF attribute mapping (namespace extends `https://attr.arkavo.com/`; clearance stays hierarchical; departments and projects as separate conjunctive definitions; legal-hold modeled as obligation/assertion, **not** a decrypt entitlement).
- Tripwire tests for SENT and KP ids mirroring the SEQ pattern.

**Accept:** specs validate against `schema.json`; new tripwires red-by-design; zero runtime diffs.

## Phase 1 — Taint substrate (SEQ-001, SEQ-002, SEQ-004-minimal · #549, #556)

**Touch:** `arkavo-protocol` (types), `arkavo-session` (tracker; it owns `conversation.rs`), `arkavo-events` (ledger entries).

- Extend `arkavo-protocol/src/data_classification.rs`: `TaintLabel { source_id, categories: Set<DataCategory>, sensitivity: SensitivityLevel, hops: Vec<ProvenanceHop> }`, `TaintSet` with monotonic-union ops (`union` = max sensitivity, ∪ categories, concat provenance), serde for the sequence ledger.
- `DataTaintTracker` in `arkavo-session`: tag at ingestion (tool results, A2A receive, file reads), propagate through transformations, persist to ledger. Minimal `SequenceGraphBuilder` (SEQ-004): nodes per tool call, edges on output→input flow, params hash and taint labels in node metadata.
- Define trait `ClassificationInferencer` — the SEQ-001 "classification level inferred" seam. First impl: `RegexInferencer` wrapping the existing `DatumType` detector. (Phase 4 plugs the sentinel into this same seam.)
- LLM I/O rule (SEQ-002 edge case): output of inference inherits the union of input taints ∪ the serving model's classification ceiling (ceiling arrives in Phase 5 metadata; until then, config-supplied per model).

**Accept:** SEQ-001/002 tripwires flipped for implemented paths; propagation unit tests incl. encode-does-not-strip (base64/JSON); tracker overhead benched <50µs per call.

## Phase 2 — Taint-aware egress, actions, audit (SEQ-003, SEQ-014, SEQ-015 · #548)

**Touch:** `arkavo-validation` (gate, beside `EgressFilter` in `url.rs`), `arkavo-mcp-runtime/src/server.rs::execute_tool` (dispatch seam), A2A send path, file-write tools, `arkavo-observability/src/decision_trace.rs`, `arkavo-security/src/rate_limit.rs`.

- `EgressTaintGate`: evaluates payload `TaintSet` × destination policy. Extend actions beyond `DlpAction::{Allow,Block,Redact}` with `Wrap { attributes }` and `Hold` (quarantine). Auto-wrap four-case semantics:
  1. requester entitled and transport needs protection → wrap (attrs from taxonomy map) and deliver;
  2. destination unresolved → seal and hold pending entitlement resolution;
  3. requester not entitled → block/quarantine (wrapping never rescues an unauthorized disclosure);
  4. destination cannot consume TDF → block; never silently downgrade to plaintext.
- Wire the gate at: tool dispatch (`execute_tool`), outbound A2A envelopes, file writes to non-workspace paths. Credential category blocked unconditionally (matches existing DLP tests).
- SEQ-014: provenance chain in `EgressError` and denial audit events; SEQ-015: sequence evidence into `decision_trace`. External-facing denials stay generic (oracle protection); full provenance goes to audit only.
- Rate-limit the gate with the existing token bucket.
- Static v1 taxonomy map loaded from `schemas/taxonomy-map.v1.json`; wrap via `arkavo-tdf` `PolicyBuilder` (conjunctive attrs plus requester DID dissemination, per `swarmkit-runtime/src/tdf.rs` patterns).

**Accept:** SEQ-003/014/015 tripwires flipped; `dlp_pii_security_test.sh` extended with taint cases incl. encode-to-evade (SEQ-003 edge case); split-across-requests documented as deferred to sequence-ledger work (#552/#556 full), tripwire retained.

## Phase 3 — Keyed reference index (fast tier)

**New crate:** `arkavo-fingerprint`.

- Normalized shingling → **HMAC-keyed** exact hashes (tenant key provisioned via the `arkavo-config-encryption` KAS-backed pattern; raw hashes forbidden — dictionary-confirmation resistance) → tenant-keyed MinHash/SimHash signatures for near-dup → suppression index for boilerplate, public, and common code.
- Build-time CLI: `arkavo pack index --corpus <dir> --taxonomy schemas/taxonomy-map.v1.json --out index.tdf` (new subcommand in `arkavo-cli`); output TDF-wrapped via `arkavo-tdf`.
- Runtime: lookup tier registered ahead of the inferencer seam; embeddings tier explicitly deferred (see Open decisions).

**Accept:** hash-tier lookup ≤50µs p99 in crate bench; index round-trips through TDF; suppression list demonstrably kills a seeded boilerplate false positive.

## Phase 4 — Sentinel runtime, cascade, holdback (fills the SEQ-001 inference seam)

**New crate:** `arkavo-sentinel`. **Touch:** `arkavo-llm` (stream path), `arkavo-critic` (pipeline), `arkavo-gguf-tdf` (reader reuse).

- Loader: sentinel classifier ships as TDF-GGUF, opened through the existing `arkavo-gguf-tdf` streaming reader (KAS-gated, zeroized) — the DLP model is protected by the mechanism it enforces.
- `SentinelInferencer: ClassificationInferencer` emitting the SENT evidence contract. Calibrated per-label thresholds come from the pack manifest, never hardcoded. Output feeds `TaintSet` via monotonic union — it can only add.
- Cascade orchestration (per outbound span): keyed-hash tier → SimHash tier → sentinel, with sentinel **off the <50µs hot path**: async scoring against a holdback buffer.
- Streaming semantics in `arkavo-llm`: sentence/sliding-window holdback with overlap before token release; whole-field inspection of tool-call arguments before execution; models whose ceiling ≥ Confidential stream nothing partial (config per model classification). A completion cannot be unstreamed — this is the threat model, not a nicety.
- `SentinelCheck` added to `CriticPipeline` after `CircuitCheck`, returning evidence (not verdicts) for the policy layer.
- Oracle protections active: gate and sentinel share the token-bucket budget; raw scores never cross the trust boundary.

**Accept:** SENT tripwires flipped; holdback latency p50/p95/p99 published in bench; e2e test proves a seeded canary in a completion is caught pre-release under mock provider.

## Phase 5 — Knowledge pack format, signing, adapter channel

**Touch:** `arkavo-gguf-tdf` (metadata), `arkavo-llama-cpp` / `arkavo-llama-cpp-sys` (new adapter API), `arkavo-identity`/`arkavo-attestation` (signing), `arkavo-cli` (`arkavo pack build`).

- Pack layout (`.knowpack.tdf`): `adapter-<compartment>.gguf.tdf` (0..n) plus `sentinel.gguf.tdf`, `index.tdf`, `manifest.json`, `manifest.sig`. Components wrapped **separately** so an egress node can hold sentinel and index without the knowledge model. Manifest signed against the org's did:webvh anchor; verify with the embedded-policy pattern from `swarmkit-runtime/src/tdf.rs` (`unwrap_manifest_kas_gated`).
- `arkavo-llama-cpp-sys`: bind `llama_adapter_lora_init/free/set/clear` (currently unexposed). Load-time adapter selection by the session's entitlement set; adapters partition by clearance level only; compartments within a level ride TDF-RAG capsules through the existing per-role attribute release. Refuse mixed-level adapter stacking unless the session accepts the high-water-mark ceiling.
- `arkavo-gguf-tdf`: add classification-ceiling metadata field; Phase 1's LLM I/O rule reads it from here instead of config.
- Revocation story stays honest in docs: revocable and volatile facts belong in TDF-RAG (key revocation is instant); fine-tuning is for stable terminology, schemas, procedures, domain behavior.

**Accept:** KP tripwires flipped; pack build and verify round-trip in e2e; adapter selection integration test with two clearance levels; tampered manifest rejected.

## Phase 6 — Distillation and training pipeline (extends the small-stories draft PR)

**New:** `crates/arkavo-distill` (orchestration) plus `scripts/distill/` (Python has precedent in `scripts/*.py`).

- Ingest and chunk with source metadata retained (repo, folder, and DMS labels seed taxonomy labels).
- Derivation pass via `arkavo-llm` against a **local** provider: paraphrases, summaries, Q&A per chunk — shared between fine-tune data and detector positives.
- Negatives assembly: public plus industry-adjacent plus **internal benign/declassified** text (without the last, the sentinel learns "written by us," not "sensitive").
- Split discipline: train and test by original **source family**, never by synthetic example; eval derivations generated by a **different** model or method than training derivations (do not learn one generator's artifacts).
- Train the sentinel head, temperature-scale calibration on a held-out set, emit thresholds into the manifest.
- Reviewer-feedback loop: corrections carry provenance plus signed approval before entering the next cycle (active-learning labels are a poisoning path).
- Every intermediate artifact (corpus, derivations, checkpoints, tmp GGUFs) TDF-wrapped or shredded; nothing plaintext survives the run.

**Accept:** one command produces a verifiable pack from a sample corpus; leakage check confirms no source family spans train and eval; pipeline runs fully offline.

## Phase 7 — Eval, red team, CI

**Touch:** `tests/golden_dataset`, `tests/*.sh`, `arkavo-agent-benchmark` (companion repo), CI.

- Golden sets: synthetic canaries **plus** natural held-out secrets; paraphrase, translation, and encoding variants; multi-turn fragmentation; structured tool-call smuggling; low-and-slow sequences (SEQ-003 edge cases are the seed list).
- Metrics harness: recall at fixed operational FPR, per-tag F1, severity-weighted leakage, extraction-rate delta on the knowledge model with the gate on and off, latency p50/p95/p99. Publish into the pack manifest's eval-evidence digest.
- `arkavo-agent-benchmark`: add extraction-attack scenarios; wire as a recurring job.
- CI: phase-gate commands plus golden-set thresholds as merge gates; regression alarms on FPR drift.

**Accept:** dashboarded numbers a buyer can read; benchmark job green; thresholds enforced in CI.

## Open decisions

Resolved:

- **Pack naming:** `.knowpack.tdf`, with components named `adapter-<compartment>.gguf.tdf`, `sentinel.gguf.tdf`, `index.tdf`, `manifest.json`, `manifest.sig`. A pack is a multi-component archive with its own lifecycle; `.swarmkit.tdf` stays what it is today, a role-policy kit.
- **Training stack** and **embedding tier:** the plan's stated defaults are recorded as advisory defaults, not spec invariants — a Rust-first training path with GGUF export, and a hash-plus-sentinel cascade with no embedding tier. KP and SENT scenarios specify outcomes (calibrated thresholds bound into the manifest, split by source family, cascade tier ordering and budget), so swapping either default is an implementation change, not a spec change.

Open:

- **Sentinel base:** model family and size target (≤1B) plus license compatibility with `deny.toml` and `THIRD-PARTY-LICENSES.md`.
- **Release mapping proposal:** 0.91 → P0–P2 · 0.92 → P3–P4 · 0.93 → P5 · 0.94 → P6–P7.

## Milestone map

| Phase | Spec ids | Issues | Crates touched | Tripwires flipped |
|---|---|---|---|---|
| 0 | KP-*, SENT-* (new, wip) | — | specs, schemas | none (new reds added) |
| 1 | SEQ-001/002/004 | #549 #556 | protocol, session, events | validation and protocol SEQ-001/002 |
| 2 | SEQ-003/014/015 | #548 | validation, mcp-runtime, observability, security, tdf | SEQ-003/014/015 |
| 3 | KP-009..011 | — | arkavo-fingerprint (new), cli, tdf | KP index reds |
| 4 | SENT-*, SEQ-001 seam | #549 | arkavo-sentinel (new), llm, critic | SENT reds |
| 5 | KP-001..008 | — | gguf-tdf, llama-cpp(-sys), identity, cli | KP pack reds |
| 6 | KP-012..017 | — | arkavo-distill (new), scripts | pipeline e2e |
| 7 | eval gates | #552 partial | tests, agent-benchmark, CI | — |
