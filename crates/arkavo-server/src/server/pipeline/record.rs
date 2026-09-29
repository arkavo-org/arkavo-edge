//! What a run produced, and the shapes it is handed to the caller in.
//!
//! A caller reads a finished run from its task. The text part is for a
//! person or a client that only reads text: the final answer first, then
//! every other role's output under a heading that names the role. The data
//! part holds the same run as structured data, because headings in text
//! cannot be told from headings a role wrote in its own output.

use std::fmt::Write as _;

use arkavo_mcp_mesh::RequestError;
use arkavo_protocol::types::{Message, MessagePart, TaskError};
use arkavo_swarmkit::OnFailure;
use arkavo_swarmkit::pipeline::{CriticGate, PipelinePlan, Verdict};
use serde::Serialize;
use serde_json::{Value, json};
use uuid::Uuid;

use super::Stopped;
use super::brief::RESULT_SCHEMA;

/// Error code of a task whose pipeline run failed.
pub const FAILURE_CODE: &str = "PIPELINE_FAILED";

/// Schema named in the details of a failed run's task error.
pub const FAILURE_SCHEMA: &str = "urn:arkavo:pipeline:failure:v1";

/// One answer a role gave during a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepRecord {
    /// Position of the role in the pipeline, counting from 1.
    pub step: usize,
    pub role: String,
    /// Which of the role's answers this is, counting from 1.
    pub attempt: u32,
    /// True for a draft that was sent back and for the review that sent it
    /// back: a later answer from the same role replaced it.
    pub superseded: bool,
    pub output: String,
}

/// Everything the roles answered, in the order they answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunRecord {
    pub run_id: Uuid,
    /// The pipeline's roles in running order.
    pub roles: Vec<String>,
    pub steps: Vec<StepRecord>,
    /// Revisions asked for so far.
    pub retries_used: u32,
    /// The critic's latest verdict, once it has reviewed anything.
    pub verdict: Option<Verdict>,
}

impl RunRecord {
    pub(super) fn new(run_id: Uuid, plan: &PipelinePlan) -> Self {
        Self {
            run_id,
            roles: plan.steps.iter().map(|s| s.role_id.clone()).collect(),
            steps: Vec::new(),
            retries_used: 0,
            verdict: None,
        }
    }

    /// How many times the role at `step` (counting from 0) has answered.
    pub(super) fn attempts_of(&self, step: usize) -> u32 {
        let answers = self.steps.iter().filter(|s| s.step == step + 1).count();
        u32::try_from(answers).unwrap_or(u32::MAX)
    }

    pub(super) fn answered(&mut self, step: usize, role: &str, output: String) {
        let attempt = self.attempts_of(step) + 1;
        self.steps.push(StepRecord {
            step: step + 1,
            role: role.to_string(),
            attempt,
            superseded: false,
            output,
        });
    }

    /// Mark the standing answer of the role at `step` as replaced.
    pub(super) fn supersede(&mut self, step: usize) {
        if let Some(standing) = self
            .steps
            .iter_mut()
            .rev()
            .find(|s| s.step == step + 1 && !s.superseded)
        {
            standing.superseded = true;
        }
    }

    /// The standing answer of the role at `step` (counting from 0).
    pub fn output_of(&self, step: usize) -> Option<String> {
        self.standing()
            .find(|s| s.step == step + 1)
            .map(|s| s.output.clone())
    }

    /// The answers that stand, one per role that has answered, in pipeline
    /// order.
    pub fn standing(&self) -> impl Iterator<Item = &StepRecord> {
        let mut standing: Vec<&StepRecord> = self.steps.iter().filter(|s| !s.superseded).collect();
        standing.sort_by_key(|s| s.step);
        standing.into_iter()
    }

    /// The last role's answer, once the run has reached it.
    pub fn final_answer(&self) -> Option<&StepRecord> {
        self.standing().find(|s| s.step == self.roles.len())
    }

    pub(super) fn failed(self, plan: &PipelinePlan, stopped: Stopped) -> RunFailure {
        RunFailure {
            kind: stopped.kind,
            role: plan.role_at(stopped.step).to_string(),
            step: stopped.step + 1,
            reason: stopped.reason,
            gate: plan.critic,
            record: self,
        }
    }

    /// The result a caller's task is completed with.
    pub fn to_result(&self) -> Message {
        let steps = self.roles.len();
        let mut text = String::new();
        let last = self.final_answer();
        if let Some(last) = last {
            let _ = write!(
                text,
                "## Final answer (role: {}, step {steps} of {steps})\n\n{}",
                last.role,
                last.output.trim()
            );
        }
        for answer in self.standing().filter(|s| s.step != steps) {
            let revised = if answer.attempt > 1 {
                format!(", revision {}", answer.attempt - 1)
            } else {
                String::new()
            };
            let _ = write!(
                text,
                "\n\n## Output from role: {} (step {} of {steps}{revised})\n\n{}",
                answer.role,
                answer.step,
                answer.output.trim()
            );
        }
        let _ = write!(text, "\n\nPipeline run {}", self.run_id);

        Message {
            parts: vec![
                MessagePart::Text { content: text },
                MessagePart::Data {
                    schema: RESULT_SCHEMA.to_string(),
                    content: json!({
                        "run_id": self.run_id.to_string(),
                        "status": "completed",
                        "roles": self.roles,
                        "final_role": last.map(|s| s.role.as_str()),
                        "final": last.map(|s| s.output.as_str()),
                        "verdict": self.verdict.map(Verdict::as_str),
                        "retries_used": self.retries_used,
                        "steps": self.steps,
                    }),
                },
            ],
            metadata: None,
        }
    }
}

/// The way a run failed, as a word a client can branch on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// The entry role's own answer could not be produced.
    EntryFailed,
    AgentNotFound,
    Unreachable,
    Refused,
    StepFailed,
    StepCanceled,
    StepRejected,
    StepStalled,
    TimedOut,
    /// A role finished and had written nothing.
    NoOutput,
    /// The run's own time budget ran out.
    BudgetExhausted,
    /// The critic answered `VERDICT: FAIL` and no retries were left.
    VerdictFail,
    /// The critic gave no verdict line and no retries were left.
    VerdictMissing,
    /// The caller canceled its task.
    Canceled,
}

impl FailureKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::EntryFailed => "entry_failed",
            Self::AgentNotFound => "agent_not_found",
            Self::Unreachable => "unreachable",
            Self::Refused => "refused",
            Self::StepFailed => "step_failed",
            Self::StepCanceled => "step_canceled",
            Self::StepRejected => "step_rejected",
            Self::StepStalled => "step_stalled",
            Self::TimedOut => "timed_out",
            Self::NoOutput => "no_output",
            Self::BudgetExhausted => "budget_exhausted",
            Self::VerdictFail => "verdict_fail",
            Self::VerdictMissing => "verdict_missing",
            Self::Canceled => "canceled",
        }
    }

    const fn is_verdict(self) -> bool {
        matches!(self, Self::VerdictFail | Self::VerdictMissing)
    }
}

impl From<&RequestError> for FailureKind {
    fn from(error: &RequestError) -> Self {
        match error {
            RequestError::AgentNotFound { .. } => Self::AgentNotFound,
            RequestError::Unreachable { .. } => Self::Unreachable,
            RequestError::Refused { .. } => Self::Refused,
            RequestError::Failed { .. } => Self::StepFailed,
            RequestError::Canceled { .. } => Self::StepCanceled,
            RequestError::Rejected { .. } => Self::StepRejected,
            RequestError::Stalled { .. } => Self::StepStalled,
            RequestError::TimedOut { .. } => Self::TimedOut,
        }
    }
}

/// A run that did not finish, with everything it had produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunFailure {
    pub kind: FailureKind,
    /// The role that was running, or the critic whose verdict ended the run.
    pub role: String,
    /// Position of that role in the pipeline, counting from 1.
    pub step: usize,
    pub reason: String,
    /// The kit's critic gate, when it has one.
    pub gate: Option<CriticGate>,
    pub record: RunRecord,
}

impl RunFailure {
    /// One line that names the role and the reason.
    pub fn summary(&self) -> String {
        let steps = self.record.roles.len();
        if self.kind.is_verdict() {
            format!(
                "pipeline run {} failed: {}",
                self.record.run_id, self.reason
            )
        } else {
            format!(
                "pipeline run {} failed at role {:?} (step {} of {steps}): {}",
                self.record.run_id, self.role, self.step, self.reason
            )
        }
    }

    /// The error a caller's task is failed with.
    ///
    /// A run the critic ended carries the last draft and the review in the
    /// message itself: they are what the caller needs to act on, and a
    /// client that prints only the message must still show them. Every
    /// answer of the run is in the details either way.
    pub fn to_task_error(&self) -> TaskError {
        let mut message = self.summary();
        if self.kind.is_verdict() {
            for answer in self
                .record
                .standing()
                .filter(|s| s.step == self.step || s.step + 1 == self.step)
            {
                let _ = write!(
                    message,
                    "\n\n## Last output from role: {}\n\n{}",
                    answer.role,
                    answer.output.trim()
                );
            }
        }
        TaskError {
            code: FAILURE_CODE.to_string(),
            message,
            details: Some(self.details()),
        }
    }

    fn details(&self) -> Value {
        json!({
            "schema": FAILURE_SCHEMA,
            "run_id": self.record.run_id.to_string(),
            "status": "failed",
            "reason": self.kind.as_str(),
            "role": self.role,
            "step": self.step,
            "roles": self.record.roles,
            "verdict": self.record.verdict.map(Verdict::as_str),
            "retries_used": self.record.retries_used,
            "max_retries": self.gate.map(|g| g.max_retries),
            "on_failure": self.gate.map(|g| on_failure_name(g.on_failure)),
            "steps": self.record.steps,
        })
    }
}

/// Why a verdict ended the run, naming the verdict and what the kit says
/// about failure.
///
/// Only `abort` is carried out. The manifest format also accepts `retry`,
/// `escalate` and `partial`, and nothing in this repository says what a
/// pipeline is to do for them, so a run that fails its review ends the same
/// way under each of them and the reason says which was declared.
pub(super) fn verdict_reason(
    critic: &str,
    verdict: Verdict,
    retries_used: u32,
    gate: CriticGate,
) -> String {
    let said = match verdict {
        Verdict::Missing => {
            format!("critic {critic:?} gave no verdict line, which counts as VERDICT: FAIL")
        }
        Verdict::Pass | Verdict::Fail => {
            format!("critic {critic:?} gave VERDICT: {}", verdict.as_str())
        }
    };
    let retries = match gate.max_retries {
        0 => "completion.max_retries is 0".to_string(),
        max => format!("{retries_used} of {max} retries were used"),
    };
    let declared = on_failure_name(gate.on_failure);
    let applied = if gate.on_failure == OnFailure::Abort {
        format!("completion.on_failure is {declared}")
    } else {
        format!("completion.on_failure is {declared}, which is carried out as abort")
    };
    format!("{said}; {retries}; {applied}")
}

fn on_failure_name(on_failure: OnFailure) -> &'static str {
    match on_failure {
        OnFailure::Retry => "retry",
        OnFailure::Abort => "abort",
        OnFailure::Escalate => "escalate",
        OnFailure::Partial => "partial",
    }
}
