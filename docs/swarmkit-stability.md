# SwarmKit Stability

This document declares which fields in the public `arkavo-swarmkit` API surface are stable, evolving, or experimental. Producers writing kits today can rely on stable fields; evolving and experimental fields may shift between spec drafts.

The current published spec drafts are `swarmkit-spec-draft-00` and `swarmkit-spec-draft-01`. This document tracks fields against draft-01. Future drafts will update both the spec and this document; the [SwarmKit CHANGELOG](https://github.com/arkavo-org/specifications/blob/main/swarmkit/CHANGELOG.md) records each change.

## Tier definitions

| Tier | Meaning | Producer assumes |
|---|---|---|
| **stable** | Field name, type, and semantics will not change in a SemVer-MAJOR-bumping way before the spec hits 1.0 | Safe to encode in long-lived YAML manifests |
| **evolving** | Field exists and is supported, but name/type/semantics may shift between draft-NN versions | Safe to use today; check changelog when upgrading |
| **experimental** | Field is shipped behind a stability fence — may be removed entirely or substantially redesigned | Use only for prototyping |

## Manifest top-level (`Manifest`)

| Field | Tier | Note |
|---|---|---|
| `spec_version` | stable | Required; semver. Major bumps imply incompatibility. |
| `kit` (KitMetadata) | stable | See § KitMetadata. |
| `objective` (Objective) | stable | Producers write goals + success_criteria. |
| `inputs` (Vec<InputSpec>) | stable | |
| `deliverables` (Vec<DeliverableSpec>) | stable | |
| `roles` (Vec<RoleSpec>) | stable | At least one required. |
| `coordination` (CoordinationSpec) | stable | |
| `constraints` (ConstraintsSpec) | stable | |
| `evaluation` (Option<EvaluationSpec>) | stable | |
| `completion` (CompletionSpec) | stable | |
| `provenance` (ProvenanceSpec) | stable | |

## KitMetadata

| Field | Tier | Note |
|---|---|---|
| `id` | stable | BLAKE3 of canonical manifest (§9.1 / SK-003); content-addressed. |
| `name` | stable | |
| `version` | stable | Producer-defined semver. |
| `description` | stable | Optional. |
| `authors` (Vec<Author>) | stable | At least one required. |
| `created` | stable | RFC 3339. |
| `expires` | stable | RFC 3339; optional. SwarmFlight-side expiry-vs-now check is aspirational (§4.1-MUST-1 in v2 audit). |
| `nonce` | stable | Replay-prevention token. |

## Author

| Field | Tier | Note |
|---|---|---|
| `did` | stable | Parser accepts any string. did:web resolution is wired only for skill signers via DidWebPublicKeyResolver; kit-author DID resolution is aspirational (§9.3-MUST-1). |
| `name` | stable | Optional human-readable name. |

## Objective

| Field | Tier | Note |
|---|---|---|
| `goal` | stable | |
| `success_criteria` (Vec<String>) | stable | |

## InputSpec / DeliverableSpec / PayloadType

| Field | Tier | Note |
|---|---|---|
| `name` | stable | |
| `type` (PayloadType) | stable | Enum: text, json, tdf-ref. tdf-ref input enforcement is parse-only; runtime decryption is wired via tdf.rs but role-side consumption isn't tested per kit. |
| `required` | stable | |
| `classification` | stable | Free-form string per SK-009. |

## RoleSpec

| Field | Tier | Note |
|---|---|---|
| `id` | stable | Producer-defined; must be unique within `roles[]`. |
| `role_type` | stable | Free-form per spec §4.3 / SK-006. |
| `description` | stable | Optional. |
| `agent_provisioning` (AgentProvisioning) | stable | See § AgentProvisioning. |
| `skills` (Vec<Skill>) | evolving | Skill struct gained `signature` + `signed_by` in Phase 2; spec promotion landed in draft-01. |
| `mcp_tools` (Vec<McpToolGrant>) | evolving | Field shape stable; runtime grant-issuance machinery (§11-MUST-ORCH-5) is aspirational. |
| `tdf_attribute_release_policy` | stable | Per-role attribute set; SK-053 wired role_policy() construction. |
| `handoffs` (Vec<Handoff>) | stable | Producer-defined transitions. |
| `context_scope` (Option<ContextScope>) | evolving | can_read/can_write enforcement at runtime not yet specified normatively. |

## AgentProvisioning

| Field | Tier | Note |
|---|---|---|
| `model` (Option<Model>) | stable | |
| `inference` (Option<Inference>) | stable | |
| `budget` (Option<Budget>) | stable | SK-002 covers per-role-vs-global cap. |
| `tool_use` (Option<ToolUse>) | evolving | Field shape stable; on_error/retry_policy semantics not exercised. |
| `context` (Option<ContextBlock>) | stable | |
| `observability` (Option<Observability>) | experimental | metrics_endpoint never hooked to a runtime collector. |
| `isolation` (Option<Isolation>) | stable | network_egress wired (SK §5.1-MUST-5). |
| `failure` (Option<Failure>) | evolving | fallback_role parses, but auto-recovery paths not exercised end-to-end. |

## Model / ModelFallback

| Field | Tier | Note |
|---|---|---|
| `Model.family` | stable | |
| `Model.size` | stable | Optional. |
| `Model.quantization` | stable | Optional. |
| `Model.backend` | stable | Optional. |
| `Model.fallback` (Option<ModelFallback>) | experimental | Parse-only. Runtime fallback selection isn't wired. |
| `ModelFallback.family` | experimental | |
| `ModelFallback.size` | experimental | Optional. |

## Inference

| Field | Tier | Note |
|---|---|---|
| `max_tokens` | stable | Producer-driven. Spec §5.1-MUST-1 (max_tokens vs window) is aspirational at validator level. |
| `temperature` | stable | |
| `top_p` | stable | Optional. |
| `top_k` | stable | Optional. |
| `thinking` | stable | Optional. |
| `stop_sequences` (Vec<String>) | stable | |
| `seed` | stable | Optional. |

## Budget

| Field | Tier | Note |
|---|---|---|
| `max_inference_calls` | stable | Optional. |
| `max_wallclock_ms` | stable | Optional; SK-002 covers vs global cap. |
| `max_total_tokens` | stable | Optional; SK-002 covers vs global cap. |

## ToolUse / ToolUseOnError / RetryPolicy

| Field | Tier | Note |
|---|---|---|
| `ToolUse.max_calls` | stable | |
| `ToolUse.max_parallel` | stable | |
| `ToolUse.on_error` (Option<ToolUseOnError>) | evolving | Enum stable; runtime semantics (Retry, Skip, Abort) not exercised across all kits. |
| `ToolUse.retry_policy` (Option<RetryPolicy>) | evolving | |
| `ToolUseOnError` (enum: Retry, Skip, Abort) | stable | |
| `RetryPolicy.max_attempts` | evolving | |
| `RetryPolicy.backoff_ms` | evolving | |

## ContextBlock / CompactionStrategy / Persistence

| Field | Tier | Note |
|---|---|---|
| `ContextBlock.max_context_tokens` | stable | |
| `ContextBlock.kv_cache_id` | evolving | Spec §10.1-MUST-4 (flight-scoping) is aspirational. |
| `ContextBlock.compaction_strategy` (Option<CompactionStrategy>) | stable | |
| `ContextBlock.persistence` (Option<Persistence>) | stable | |
| `CompactionStrategy` (enum: ToolResult, Summary, None) | stable | |
| `Persistence` (enum: Ephemeral, Session, Kit) | stable | Session and Kit modes not exercised per kit; field shape stable. |

## Observability / LogLevel

| Field | Tier | Note |
|---|---|---|
| `Observability.trace_required` | experimental | Parse-only. |
| `Observability.log_level` (Option<LogLevel>) | experimental | Parse-only. |
| `Observability.metrics_endpoint` | experimental | Parse-only; no runtime collector hooked up. |
| `LogLevel` (enum) | stable | |

## Isolation / Sandbox

| Field | Tier | Note |
|---|---|---|
| `Isolation.sandbox` (Option<Sandbox>) | stable | |
| `Isolation.fs_writable` (Vec<String>) | evolving | Producer-declared; runtime enforcement not specified normatively. |
| `Isolation.network_egress` | stable | SK §5.1-MUST-5 wired. |
| `Sandbox` (enum: Process, Container, etc.) | stable | |

## Failure / OnTimeout / CircuitBreaker

| Field | Tier | Note |
|---|---|---|
| `Failure.on_timeout` (Option<OnTimeout>) | experimental | Parse-only. |
| `Failure.fallback_role` | evolving | Parses, validates handoff target exists; auto-recovery path not exercised. |
| `Failure.circuit_breaker` (Option<CircuitBreaker>) | experimental | Parse-only. |
| `OnTimeout` (enum) | experimental | |
| `CircuitBreaker.threshold` | experimental | |
| `CircuitBreaker.window_ms` | experimental | |

## Skill / SkillSource

| Field | Tier | Note |
|---|---|---|
| `Skill.id` | stable | Producer-defined opaque identifier. |
| `Skill.version` | stable | Producer-defined semver. |
| `Skill.source` (SkillSource) | stable | Enum: Inline, Registry, TdfRef. |
| `Skill.payload` | stable | Inline source's content. |
| `Skill.signature` | evolving | Phase 2 invented the ed25519/BLAKE3 protocol; draft-01 promotes it to normative. |
| `Skill.signed_by` | evolving | DID of signer; Phase 2 wired did:web resolution. |
| `SkillSource` (enum: Inline, Registry, TdfRef) | stable | TdfRef returns explicit `TdfRefNotImplemented` error (roadmap). |

## SkillContent / SkillResource

| Field | Tier | Note |
|---|---|---|
| `SkillContent.name` | evolving | Phase 2 invented the SkillContent shape; draft-01 promotes to normative JSON schema. |
| `SkillContent.description` | evolving | |
| `SkillContent.instructions` | evolving | |
| `SkillContent.resources` (Vec<SkillResource>) | experimental | No kit ships skill resources; encoding (`bytes_base64`) hasn't been exercised end-to-end. |
| `SkillResource.name` | experimental | |
| `SkillResource.mime` | experimental | |
| `SkillResource.bytes_base64` | experimental | Base64url, no padding. |

## McpToolGrant / AuthMode

| Field | Tier | Note |
|---|---|---|
| `McpToolGrant.server` | stable | |
| `McpToolGrant.tools` (Vec<String>) | stable | Producer-defined allowlist (SK-016 covers the extensibility claim). Wildcards SHOULD NOT be used per spec §4.3-SHOULD-2 / §8.2-SHOULD-1; not enforced. |
| `McpToolGrant.auth` (AuthMode) | stable | |
| `AuthMode` (enum: Delegated, Passthrough, None) | stable | Runtime grant issuance is aspirational; field set stable. |

## TdfAttributeReleasePolicy / TdfReleaseRule

| Field | Tier | Note |
|---|---|---|
| `TdfAttributeReleasePolicy.attributes` (Vec<String>) | stable | Free-form FQN strings (SK-008). Each must be `<fqn>/<value>` form. |
| `TdfAttributeReleasePolicy.rule` (TdfReleaseRule) | stable | |
| `TdfReleaseRule` (enum: AllOf, AnyOf, Hierarchy) | stable | Hierarchy semantics evaluated KAS-side at attribute-definition time. Renamed from `ArpRule` (it is a TDF attribute-release rule, not Agent Runtime Policy); wire format unchanged — only camelCase variant names are serialized. A deprecated `ArpRule` type alias remains for one release. |

## Handoff / ContextScope

| Field | Tier | Note |
|---|---|---|
| `Handoff.to` | stable | Must resolve to a known role_id. |
| `Handoff.on` | stable | Trigger condition string (e.g., "always", "success", "failure"). |
| `ContextScope.can_read` (Vec<String>) | evolving | Producer-declared; runtime read-gate not specified normatively. |
| `ContextScope.can_write` (Vec<String>) | evolving | Same. |

## CoordinationSpec / Topology / Routing / RoutingStrategy / ContextSharing / ContextStore / CompactionSpec

| Field | Tier | Note |
|---|---|---|
| `CoordinationSpec.topology` (Topology) | stable | |
| `CoordinationSpec.protocol` | stable | Producer-declared; A2A wire is aspirational. |
| `CoordinationSpec.routing` (Routing) | stable | |
| `CoordinationSpec.context_sharing` (Option<ContextSharing>) | evolving | |
| `Topology` (enum: HubSpoke, Pipeline, Mesh, etc.) | stable | |
| `Routing.strategy` (RoutingStrategy) | stable | |
| `Routing.parameters` (Option<Value>) | experimental | Free-form bag; no spec for parameter contents. |
| `RoutingStrategy` (enum: Static, Dynamic, RoundRobin, etc.) | stable | |
| `ContextSharing.store` (ContextStore) | evolving | |
| `ContextSharing.compaction` (Option<CompactionSpec>) | evolving | |
| `ContextStore` (enum: PerRole, Shared, etc.) | stable | |
| `CompactionSpec.strategy` | evolving | |

## ConstraintsSpec / GlobalBudget / NetworkConstraints

| Field | Tier | Note |
|---|---|---|
| `ConstraintsSpec.global_budget` (GlobalBudget) | stable | |
| `ConstraintsSpec.data_classifications` (Vec<String>) | stable | Free-form (SK-009 covers extensibility). |
| `ConstraintsSpec.jurisdiction` (Vec<String>) | stable | Free-form. |
| `ConstraintsSpec.network` (NetworkConstraints) | stable | |
| `GlobalBudget.max_wallclock_seconds` | stable | |
| `GlobalBudget.max_total_tokens` | stable | |
| `GlobalBudget.max_cost_usd` | stable | |
| `NetworkConstraints.egress_allowed` | stable | |
| `NetworkConstraints.egress_allowlist` (Vec<String>) | evolving | Field stable; runtime allowlist enforcement not specified normatively. |

## EvaluationSpec / EvaluationRubric / EvaluationDimension

| Field | Tier | Note |
|---|---|---|
| `EvaluationSpec.rubric` (EvaluationRubric) | stable | |
| `EvaluationSpec.critic_role` | stable | Must resolve to a known role_id. |
| `EvaluationSpec.sample_size` | stable | Optional. |
| `EvaluationRubric.reference` | stable | Optional URI/fragment. |
| `EvaluationRubric.dimensions` (Vec<EvaluationDimension>) | stable | |
| `EvaluationDimension.name` | stable | Producer-defined (SK-007 covers extensibility). |
| `EvaluationDimension.weight` | stable | SK-005 covers sum-to-1.0. |
| `EvaluationDimension.threshold` | stable | |

## CompletionSpec / OnFailure

| Field | Tier | Note |
|---|---|---|
| `CompletionSpec.rules` (Vec<String>) | stable | Free-form completion conditions. |
| `CompletionSpec.on_failure` (OnFailure) | stable | |
| `CompletionSpec.max_retries` | stable | |
| `OnFailure` (enum: Retry, Abort, Escalate, Partial) | stable | |

## ProvenanceSpec / Signature / C2paAssertion

| Field | Tier | Note |
|---|---|---|
| `ProvenanceSpec.signatures` (Vec<Signature>) | stable | Field shape stable; manifest-level signature *verification* is aspirational (§11-MUST-PROD-2). |
| `ProvenanceSpec.c2pa_assertions` (Vec<C2paAssertion>) | experimental | Parse-only; no CAWG validation. |
| `Signature.signer_did` | stable | |
| `Signature.algorithm` | stable | |
| `Signature.signature` | stable | Field shape stable; verification is aspirational. |
| `C2paAssertion` (struct fields not yet fixed) | experimental | Spec §9.3-MUST-2 (CAWG conformance) aspirational. |

## ValidationError variants

| Variant | Tier | Note |
|---|---|---|
| `NoRoles` / `DuplicateRoleId` / `UnresolvedHandoff` / `UnresolvedCriticRole` / `UnresolvedFallbackRole` | stable | Cross-block validation, all wired. |
| `BudgetExceedsGlobal` | stable | SK-002. |
| `NetworkEgressDenied` | stable | |
| `EmptyRubricDimensions` / `RubricWeightsDoNotSumToOne` | stable | SK-005. |
| `ExpiryHorizonTooLarge` / `ExpiryBeforeCreated` | stable | SK-004. |
| `KitIdHashMismatch` | stable | SK-003. |
| `CanonicalSerializationFailed` | stable | |
| `SpecVersionUnparseable` / `SpecVersionMismatch` | stable | §9.4. |
| `InvalidTimestamp` | stable | |

## ParseError variants

The top-level error returned by `parse_json` and `parse_yaml`.

| Variant | Tier | Note |
|---|---|---|
| `Json(serde_json::Error)` | stable | JSON deserialization failure surfaces verbatim. |
| `Yaml(serde_yaml::Error)` | stable | YAML deserialization failure surfaces verbatim. |
| `Validation(ValidationError)` | stable | Wraps any cross-block ValidationError raised by `validate()` after deserialization. |

## Versioning policy

- A field's tier changes only via a CHANGELOG entry tied to a spec draft bump.
- Breaking changes to **stable** fields imply a SemVer MAJOR bump on `spec_version`.
- **evolving** fields can change between draft-NN versions with a CHANGELOG entry; producers using them subscribe to the changelog.
- **experimental** fields can disappear entirely; producers do so at their own risk.

The full v2 audit at [`swarmkit-launch-audit-2026-05-08.md`](swarmkit-launch-audit-2026-05-08.md) is the authoritative source for which spec MUSTs are wired vs aspirational.
