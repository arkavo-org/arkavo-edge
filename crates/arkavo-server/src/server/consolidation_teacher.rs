//! Consolidation teacher block (§7.4, #611): the first consumer of the ARP
//! `feedback_loops.consolidation` config.
//!
//! The teacher is the offline model that distills episodes into action-named
//! lessons and typed tightening proposals. Its contract is enforced post-hoc
//! regardless of the producing model: a lesson must name an action present in
//! the batch evidence; a tightening must deserialize into the closed
//! `TighteningEffect` enum and pass value sanity. Violations are recorded as
//! rejects, never silently dropped — the accept/reject split is the
//! contract-conformance rate.
//!
//! Budget discipline: `budget.per_layer.consolidation.limit_usd` is the hard
//! ceiling for one run. It is required when the teacher is non-local;
//! exhaustion stops between batches with partial output and a
//! `budget_exhausted` BudgetEvent trace. The teacher never borrows from
//! runtime budgets.
//!
//! Attribution: accepted lessons are signed by the **consolidation runner's
//! identity** (ed25519 over the canonical lesson JSON), not attributed to the
//! remote model — the teacher informs, the runner vouches.

use arkavo_arp::observability::{TraceDecision, TraceEventType, TraceLayer, TraceOutcome};
use arkavo_arp::proposal::{
    BlastRadius, ProposalOrigin, ProposalState, TighteningEffect, TighteningProposal, TraceRef,
};
use arkavo_observability::decision_trace::DecisionTrace;
use arkavo_router::learning::{Episode, Lesson, LessonPattern};
use async_trait::async_trait;
use base64::Engine;
use ed25519_dalek::{Signer, SigningKey};
use serde_json::{Value, json};

use super::synthesis::is_action_tool;

/// Fable 5 list price, USD per million tokens.
const FABLE_INPUT_USD_PER_MTOK: f64 = 10.0;
const FABLE_OUTPUT_USD_PER_MTOK: f64 = 50.0;

/// One teacher completion: text plus the cost it incurred.
#[derive(Debug, Clone)]
pub struct TeacherReply {
    pub text: String,
    pub cost_usd: f64,
}

/// A consolidation teacher model. Implementations: Fable via the Anthropic
/// provider (non-local — per-layer budget required), the local router path
/// (offline-safe fallback), and mocks in tests.
#[async_trait]
pub trait TeacherModel: Send + Sync {
    async fn complete(&self, system: &str, user: &str) -> Result<TeacherReply, String>;
    /// Model identity, recorded for audit (never used as the signer).
    fn identity(&self) -> String;
    /// Non-local teachers require a consolidation layer budget.
    fn is_local(&self) -> bool;
}

/// Fable teacher over the arkavo-llm Anthropic provider.
pub struct FableTeacher {
    provider: arkavo_llm::providers::anthropic::AnthropicProvider,
    model: String,
}

impl FableTeacher {
    /// Construct from the environment (`ANTHROPIC_API_KEY`), honoring the
    /// config's `model_preference`.
    pub fn from_env(model_preference: &str) -> Result<Self, String> {
        let provider = arkavo_llm::providers::anthropic::AnthropicProvider::from_env_with_model(
            model_preference,
        )
        .map_err(|e| format!("Fable teacher unavailable: {e}"))?;
        Ok(Self {
            provider,
            model: model_preference.to_string(),
        })
    }
}

#[async_trait]
impl TeacherModel for FableTeacher {
    async fn complete(&self, system: &str, user: &str) -> Result<TeacherReply, String> {
        let messages = vec![
            arkavo_llm::Message::system(system),
            arkavo_llm::Message::user(user),
        ];
        let (text, usage) = self
            .provider
            .complete_with_usage(messages)
            .await
            .map_err(|e| e.to_string())?;
        let cost_usd = f64::from(usage.input_tokens) / 1e6 * FABLE_INPUT_USD_PER_MTOK
            + f64::from(usage.output_tokens) / 1e6 * FABLE_OUTPUT_USD_PER_MTOK;
        Ok(TeacherReply { text, cost_usd })
    }

    fn identity(&self) -> String {
        self.model.clone()
    }

    fn is_local(&self) -> bool {
        false
    }
}

/// A lesson accepted by the contract, signed by the runner identity.
#[derive(Debug, Clone)]
pub struct SignedLesson {
    pub lesson: Lesson,
    /// Base64url(ed25519 signature over the canonical lesson-pattern JSON).
    pub signature_b64: String,
    /// The consolidation runner's identity — never the teacher model.
    pub signed_by: String,
}

/// A contract violation, recorded with its reason.
#[derive(Debug, Clone)]
pub struct TeacherReject {
    pub category: String,
    pub reason: String,
    pub payload: Value,
}

/// Outcome of one teacher run.
#[derive(Debug, Default)]
pub struct TeacherRun {
    pub lessons: Vec<SignedLesson>,
    pub proposals: Vec<TighteningProposal>,
    pub rejects: Vec<TeacherReject>,
    pub total_cost_usd: f64,
    /// True when the per-layer ceiling stopped the run early; output is
    /// partial by design.
    pub budget_exhausted: bool,
    pub categories_consolidated: u32,
    pub categories_budget_stopped: u32,
}

/// Runner identity: the signing key plus the attribution string.
pub struct RunnerIdentity {
    pub signing_key: SigningKey,
    pub runner_did: String,
}

const SYSTEM_PROMPT: &str = "\
You are the offline consolidation teacher for an agent swarm. You receive \
action->outcome evidence from episodes in one task category, with read-only \
observe/list calls already removed. Reason about WHICH ACTION produces good \
outcomes under WHICH CONDITION. Do not discuss observe, list, or tool \
sequencing.\n\n\
Return ONLY a JSON object:\n\
{\n\
  \"lesson\": {\"condition\": \"...\", \"action\": \"tool named in evidence\", \
\"expected_outcome\": \"...\", \"confidence\": 0.0} | null,\n\
  \"tightenings\": [\n\
    {\"effect\": {\"kind\": \"deny_tool\", \"tool\": \"...\"} | \
{\"kind\": \"raise_quality_gate_threshold\", \"new_threshold\": 0.0} | \
{\"kind\": \"lower_task_ceiling\", \"new_ceiling_usd\": 0.0} | \
{\"kind\": \"deny_egress\", \"host\": \"...\"},\n\
     \"rationale\": \"grounded in the outcomes above\",\n\
     \"blast_radius\": \"single_entity\" | \"single_agent\"}\n\
  ]\n\
}\n\
Every effect must RESTRICT behavior. A lesson MUST name an action from the \
evidence; use null when no pattern is clear.";

/// Run the teacher over per-category episode batches under the given budget
/// ceiling. `limit_usd` is `budget.per_layer.consolidation.limit_usd`; it is
/// mandatory for non-local teachers.
pub async fn run_teacher(
    teacher: &dyn TeacherModel,
    batches: &[(String, Vec<Episode>)],
    limit_usd: Option<f64>,
    identity: &RunnerIdentity,
    agent_id: &str,
    swarm_id: &str,
    trace: Option<&DecisionTrace>,
) -> Result<TeacherRun, String> {
    let limit = match limit_usd {
        Some(l) => l,
        None if teacher.is_local() => f64::INFINITY,
        None => {
            return Err(
                "budget.per_layer.consolidation.limit_usd is required for a non-local teacher"
                    .to_string(),
            );
        }
    };

    let mut run = TeacherRun::default();
    for (idx, (category, episodes)) in batches.iter().enumerate() {
        if run.total_cost_usd >= limit {
            run.budget_exhausted = true;
            run.categories_budget_stopped = (batches.len() - idx) as u32;
            emit_budget_exhausted_trace(trace, agent_id, run.total_cost_usd, limit);
            break;
        }
        let user = build_user_prompt(category, episodes);
        let reply = teacher.complete(SYSTEM_PROMPT, &user).await?;
        run.total_cost_usd += reply.cost_usd;
        run.categories_consolidated += 1;

        let evidence_actions = evidence_action_set(episodes);
        let episode_ids: Vec<String> = episodes.iter().map(|e| e.id.to_string()).collect();
        parse_and_enforce(
            category,
            &reply.text,
            &evidence_actions,
            &episode_ids,
            episodes.len() as u32,
            identity,
            agent_id,
            swarm_id,
            &mut run,
        );
    }
    Ok(run)
}

/// Action-name set from the batch, observe calls stripped per the
/// lesson-synthesis rule (single in-crate definition in `synthesis`).
fn evidence_action_set(episodes: &[Episode]) -> Vec<String> {
    let mut out: Vec<String> = episodes
        .iter()
        .flat_map(|e| e.observation.tools_used.iter())
        .filter(|t| is_action_tool(t))
        .map(|t| t.to_lowercase())
        .collect();
    out.sort();
    out.dedup();
    out
}

fn build_user_prompt(category: &str, episodes: &[Episode]) -> String {
    let outcomes: Vec<Value> = episodes
        .iter()
        .map(|e| {
            let actions: Vec<&String> = e
                .observation
                .tools_used
                .iter()
                .filter(|t| is_action_tool(t))
                .collect();
            json!({
                "actions": actions,
                "success": e.outcome.success,
                "quality": e.outcome.quality_metrics.correctness,
            })
        })
        .collect();
    let success = episodes.iter().filter(|e| e.outcome.success).count();
    format!(
        "Category: {}\n\nAction->outcome pairs ({} successes, {} failures):\n{}",
        category,
        success,
        episodes.len() - success,
        serde_json::to_string_pretty(&outcomes).unwrap_or_default()
    )
}

/// True when the lesson's leading action token names an evidence action
/// (namespace-tolerant in both directions).
fn names_evidence_action(action: &str, evidence: &[String]) -> bool {
    let token = action
        .trim()
        .split(|c: char| c.is_whitespace() || c == '(')
        .next()
        .unwrap_or("")
        .trim_end_matches([':', ',', '.'])
        .to_lowercase();
    if token.is_empty() {
        return false;
    }
    evidence.iter().any(|e| {
        e == &token || e.ends_with(&format!(":{token}")) || token.ends_with(&format!(":{e}"))
    })
}

#[allow(clippy::too_many_arguments)] // internal plumbing fn, not API surface
fn parse_and_enforce(
    category: &str,
    reply: &str,
    evidence: &[String],
    episode_ids: &[String],
    episode_count: u32,
    identity: &RunnerIdentity,
    agent_id: &str,
    swarm_id: &str,
    run: &mut TeacherRun,
) {
    let parsed = extract_json(reply).and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let Some(root) = parsed else {
        run.rejects.push(TeacherReject {
            category: category.to_string(),
            reason: "reply does not contain a parseable JSON object".into(),
            payload: Value::String(reply.chars().take(2000).collect()),
        });
        return;
    };

    // Lesson: action-named, regardless of producing model.
    match root.get("lesson") {
        None | Some(Value::Null) => {}
        Some(lesson_val) => {
            let condition = lesson_val.get("condition").and_then(Value::as_str);
            let action = lesson_val.get("action").and_then(Value::as_str);
            match (condition, action) {
                (Some(c), Some(a)) if !c.trim().is_empty() && !a.trim().is_empty() => {
                    if names_evidence_action(a, evidence) {
                        let expected = lesson_val
                            .get("expected_outcome")
                            .and_then(Value::as_str)
                            .unwrap_or("improved_outcome");
                        let confidence = lesson_val
                            .get("confidence")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.5)
                            .clamp(0.0, 1.0);
                        let lesson = Lesson::new(
                            agent_id.to_string(),
                            swarm_id.to_string(),
                            category.to_string(),
                            LessonPattern::new(c.to_string(), a.to_string(), expected.to_string()),
                            confidence,
                            episode_count,
                        );
                        run.lessons.push(sign_lesson(lesson, identity));
                    } else {
                        run.rejects.push(TeacherReject {
                            category: category.to_string(),
                            reason: format!("lesson action {a:?} does not name an evidence action"),
                            payload: lesson_val.clone(),
                        });
                    }
                }
                _ => run.rejects.push(TeacherReject {
                    category: category.to_string(),
                    reason: "lesson missing condition or action".into(),
                    payload: lesson_val.clone(),
                }),
            }
        }
    }

    // Tightenings: must deserialize into the closed effect enum.
    if let Some(arr) = root.get("tightenings").and_then(Value::as_array) {
        for item in arr {
            let effect: Result<TighteningEffect, _> =
                serde_json::from_value(item.get("effect").cloned().unwrap_or(Value::Null));
            let effect = match effect {
                Ok(e) => e,
                Err(e) => {
                    run.rejects.push(TeacherReject {
                        category: category.to_string(),
                        reason: format!("effect is not a valid tightening: {e}"),
                        payload: item.clone(),
                    });
                    continue;
                }
            };
            if let Some(violation) = effect.value_violation() {
                run.rejects.push(TeacherReject {
                    category: category.to_string(),
                    reason: violation,
                    payload: item.clone(),
                });
                continue;
            }
            let radius = item
                .get("blast_radius")
                .cloned()
                .and_then(|v| serde_json::from_value::<BlastRadius>(v).ok())
                .unwrap_or(BlastRadius::SingleEntity);
            run.proposals.push(TighteningProposal {
                id: uuid::Uuid::new_v4().to_string(),
                origin: ProposalOrigin::Consolidation,
                state: ProposalState::Proposed,
                effect,
                rationale: item
                    .get("rationale")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                blast_radius: radius,
                evidence: vec![TraceRef {
                    trace_id: format!("consolidation:{category}"),
                    episode_ids: episode_ids.to_vec(),
                }],
                created_at: chrono::Utc::now().to_rfc3339(),
                applied_at: None,
                reviewed_by: None,
                disposition_reason: None,
            });
        }
    }
}

/// Sign the lesson pattern with the runner identity. Attribution is the
/// runner, never the remote model.
fn sign_lesson(mut lesson: Lesson, identity: &RunnerIdentity) -> SignedLesson {
    let canonical = serde_json::to_vec(&lesson.pattern).unwrap_or_default();
    let signature = identity.signing_key.sign(&canonical);
    lesson.signature = signature.to_bytes().to_vec();
    SignedLesson {
        lesson,
        signature_b64: base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(signature.to_bytes()),
        signed_by: identity.runner_did.clone(),
    }
}

fn extract_json(content: &str) -> Option<String> {
    let start = content.find('{')?;
    let end = content.rfind('}')?;
    if end < start {
        return None;
    }
    Some(
        content[start..=end]
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect(),
    )
}

fn emit_budget_exhausted_trace(
    trace: Option<&DecisionTrace>,
    agent_id: &str,
    spent: f64,
    limit: f64,
) {
    if let Some(trace) = trace {
        trace.record(
            TraceLayer::Cognitive,
            TraceEventType::BudgetEvent,
            uuid::Uuid::nil(),
            agent_id,
            TraceDecision {
                chosen: Some("consolidation budget_exhausted".to_string()),
                alternatives_considered: None,
                selection_method: None,
                prior_state: None,
                posterior_state: None,
            },
            TraceOutcome {
                success: Some(false),
                quality_score: None,
                latency_ms: None,
                cost_usd: Some(spent),
                error_type: Some(format!(
                    "consolidation layer budget exhausted: spent {spent:.4} of {limit:.4}"
                )),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    // #[tokio::test] expands to Runtime::block_on; harmless in tests.
    #![allow(clippy::disallowed_methods)]

    use super::*;
    use arkavo_router::learning::{EpisodeOutcome, Observation, QualityMetrics};
    use std::sync::Mutex;

    struct MockTeacher {
        replies: Mutex<Vec<Result<TeacherReply, String>>>,
        local: bool,
    }

    impl MockTeacher {
        fn scripted(replies: Vec<(&str, f64)>, local: bool) -> Self {
            Self {
                replies: Mutex::new(
                    replies
                        .into_iter()
                        .rev()
                        .map(|(text, cost)| {
                            Ok(TeacherReply {
                                text: text.to_string(),
                                cost_usd: cost,
                            })
                        })
                        .collect(),
                ),
                local,
            }
        }
    }

    #[async_trait]
    impl TeacherModel for MockTeacher {
        async fn complete(&self, _system: &str, _user: &str) -> Result<TeacherReply, String> {
            self.replies
                .lock()
                .unwrap()
                .pop()
                .unwrap_or_else(|| Err("mock exhausted".into()))
        }
        fn identity(&self) -> String {
            "mock-teacher".into()
        }
        fn is_local(&self) -> bool {
            self.local
        }
    }

    fn identity() -> RunnerIdentity {
        RunnerIdentity {
            signing_key: SigningKey::from_bytes(&[7u8; 32]),
            runner_did: "did:web:runner.example.com".into(),
        }
    }

    fn episode(tools: &[&str], success: bool) -> Episode {
        Episode::new(
            "agent".into(),
            "swarm".into(),
            uuid::Uuid::new_v4(),
            "colony".into(),
            Observation::new(
                json!({}),
                "acts".into(),
                json!({}),
                tools.iter().map(|t| t.to_string()).collect(),
            ),
            if success {
                EpisodeOutcome::new(true, QualityMetrics::new(0.9, 0.9, 1.0), 100, 0.0)
            } else {
                EpisodeOutcome::new(false, QualityMetrics::new(0.1, 0.1, 1.0), 100, 0.0)
            },
        )
    }

    fn batch() -> Vec<(String, Vec<Episode>)> {
        vec![(
            "colony".to_string(),
            vec![
                episode(&["observe", "set_priority"], true),
                episode(&["game-rl:reset"], false),
                episode(&["set_priority"], true),
            ],
        )]
    }

    const GOOD_REPLY: &str = r#"{
        "lesson": {"condition": "food low", "action": "set_priority(Growing)",
                   "expected_outcome": "food recovers", "confidence": 0.85},
        "tightenings": [
            {"effect": {"kind": "deny_tool", "tool": "game-rl:reset"},
             "rationale": "reset correlated with failures",
             "blast_radius": "single_agent"}
        ]
    }"#;

    #[tokio::test]
    async fn conformant_reply_yields_signed_lesson_and_typed_proposal() {
        let teacher = MockTeacher::scripted(vec![(GOOD_REPLY, 0.05)], false);
        let run = run_teacher(
            &teacher,
            &batch(),
            Some(1.0),
            &identity(),
            "agent",
            "swarm",
            None,
        )
        .await
        .unwrap();

        assert_eq!(run.lessons.len(), 1);
        let signed = &run.lessons[0];
        // Attribution: runner identity, never the model.
        assert_eq!(signed.signed_by, "did:web:runner.example.com");
        assert_ne!(signed.signed_by, teacher.identity());
        // Signature verifies over the canonical pattern bytes.
        use ed25519_dalek::Verifier;
        let canonical = serde_json::to_vec(&signed.lesson.pattern).unwrap();
        let sig = ed25519_dalek::Signature::from_slice(&signed.lesson.signature).unwrap();
        assert!(
            identity()
                .signing_key
                .verifying_key()
                .verify(&canonical, &sig)
                .is_ok()
        );

        assert_eq!(run.proposals.len(), 1);
        let p = &run.proposals[0];
        assert!(matches!(p.effect, TighteningEffect::DenyTool { .. }));
        assert_eq!(p.origin, ProposalOrigin::Consolidation);
        assert_eq!(p.evidence[0].episode_ids.len(), 3);
        assert!(run.rejects.is_empty());
        assert!((run.total_cost_usd - 0.05).abs() < 1e-9);
    }

    #[tokio::test]
    async fn lesson_not_naming_evidence_action_is_rejected_regardless_of_model() {
        let reply = r#"{"lesson": {"condition": "x", "action": "teleport home",
                        "confidence": 0.9}, "tightenings": []}"#;
        let teacher = MockTeacher::scripted(vec![(reply, 0.01)], false);
        let run = run_teacher(
            &teacher,
            &batch(),
            Some(1.0),
            &identity(),
            "agent",
            "swarm",
            None,
        )
        .await
        .unwrap();
        assert!(run.lessons.is_empty());
        assert_eq!(run.rejects.len(), 1);
        assert!(run.rejects[0].reason.contains("evidence action"));
    }

    #[tokio::test]
    async fn loosening_effect_fails_the_closed_enum() {
        let reply = r#"{"lesson": null, "tightenings": [
            {"effect": {"kind": "raise_task_ceiling", "new_ceiling_usd": 99.0},
             "rationale": "more budget"}]}"#;
        let teacher = MockTeacher::scripted(vec![(reply, 0.01)], false);
        let run = run_teacher(
            &teacher,
            &batch(),
            Some(1.0),
            &identity(),
            "agent",
            "swarm",
            None,
        )
        .await
        .unwrap();
        assert!(run.proposals.is_empty());
        assert_eq!(run.rejects.len(), 1);
        assert!(run.rejects[0].reason.contains("not a valid tightening"));
    }

    #[tokio::test]
    async fn non_local_teacher_without_layer_budget_is_refused() {
        let teacher = MockTeacher::scripted(vec![(GOOD_REPLY, 0.05)], false);
        let err = run_teacher(&teacher, &batch(), None, &identity(), "a", "s", None)
            .await
            .unwrap_err();
        assert!(err.contains("per_layer.consolidation"));
    }

    #[tokio::test]
    async fn local_teacher_runs_without_layer_budget() {
        let teacher = MockTeacher::scripted(vec![(GOOD_REPLY, 0.0)], true);
        let run = run_teacher(&teacher, &batch(), None, &identity(), "a", "s", None)
            .await
            .unwrap();
        assert_eq!(run.categories_consolidated, 1);
    }

    #[tokio::test]
    async fn budget_exhaustion_stops_between_batches_with_trace() {
        use arkavo_arp::observability::DecisionTraceConfig;
        let mut batches = batch();
        batches.push(("nav".to_string(), vec![episode(&["move"], true); 3]));
        batches.push(("mine".to_string(), vec![episode(&["dig"], true); 3]));

        // First reply costs 0.06 — over the 0.05 ceiling before batch 2.
        let teacher = MockTeacher::scripted(vec![(GOOD_REPLY, 0.06)], false);
        let trace = DecisionTrace::new(DecisionTraceConfig {
            enabled: Some(true),
            storage: Some("append_only".into()),
            retention_days: None,
            cryptographic_signing: None,
        });
        let run = run_teacher(
            &teacher,
            &batches,
            Some(0.05),
            &identity(),
            "agent",
            "swarm",
            Some(&trace),
        )
        .await
        .unwrap();

        assert!(run.budget_exhausted);
        assert_eq!(run.categories_consolidated, 1);
        assert_eq!(run.categories_budget_stopped, 2);
        // Partial output is real output.
        assert_eq!(run.lessons.len(), 1);
        // budget_exhausted is traced, never silent.
        let entries = trace.snapshot();
        assert_eq!(entries.len(), 1);
        assert!(
            entries[0]
                .outcome
                .error_type
                .as_deref()
                .unwrap()
                .contains("budget exhausted")
        );
    }
}
