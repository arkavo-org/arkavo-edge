//! What a role is sent: the text it works from, and the marker filled in
//! for its step.
//!
//! The text is the only thing a role's model sees of the pipeline, so it has
//! to carry everything the role may read and nothing else.

use std::fmt::Write as _;
use std::time::Duration;

use arkavo_protocol::types::{Message, MessagePart};
use arkavo_swarmkit::pipeline::{PipelinePlan, VERDICT_FAIL_LINE, VERDICT_PASS_LINE};
use serde_json::{Value, json};

use super::marker::MARKER_KEY;
use super::record::RunRecord;

/// Schema of the data part a finished run's task result carries.
pub const RESULT_SCHEMA: &str = "urn:arkavo:pipeline:result:v1";

/// A draft the critic did not pass, on its way back to the role that wrote
/// it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Revision {
    /// Which revision this is, counting from 1.
    pub number: u32,
    /// `completion.max_retries`.
    pub of: u32,
    /// The draft that was reviewed.
    pub draft: String,
    /// The role that reviewed it.
    pub critic: String,
    /// Everything the critic answered.
    pub feedback: String,
}

/// The metadata sent with the message for `step`.
pub fn marker(
    record: &RunRecord,
    plan: &PipelinePlan,
    step: usize,
    attempt: u32,
    timeout: Duration,
) -> Value {
    json!({
        "source": "pipeline",
        "task_type": "delegated",
        MARKER_KEY: {
            "run_id": record.run_id.to_string(),
            "step": step + 1,
            "steps": plan.steps.len(),
            "role": plan.role_at(step),
            "entry_role": plan.entry_role(),
            "attempt": attempt,
            "timeout_ms": u64::try_from(timeout.as_millis()).unwrap_or(u64::MAX),
        }
    })
}

pub(super) fn message(text: String, metadata: Value) -> Message {
    Message {
        parts: vec![MessagePart::Text { content: text }],
        metadata: Some(metadata),
    }
}

/// The text the role at `step` works from.
///
/// The entry role's first input is the caller's request as it arrived.
/// Every other input opens with a heading that names the step, so no input
/// the driver writes can begin with words a receiving agent reads as
/// something other than a task.
pub fn step_text(
    plan: &PipelinePlan,
    step: usize,
    request: &str,
    record: &RunRecord,
    revision: Option<&Revision>,
) -> String {
    if step == 0 && revision.is_none() {
        return request.to_string();
    }
    let role = plan.role_at(step);
    let steps = plan.steps.len();
    let is_critic = plan.critic.is_some_and(|gate| gate.step == step);

    let mut text = format!("## Pipeline step {} of {steps}: {role}", step + 1);
    if let Some(revision) = revision {
        let _ = write!(text, " (revision {} of {})", revision.number, revision.of);
    }
    text.push_str("\n\n");
    match revision {
        Some(revision) => {
            let _ = write!(
                text,
                "The role `{}` reviewed your previous draft and did not pass it. Revise the \
                 draft to address the feedback, and answer with the complete revised work.",
                revision.critic
            );
        }
        None => text.push_str(
            "This is one step of a pipeline run. Do the work of your role for the original \
             request. The outputs of the earlier roles you may read follow it.",
        ),
    }
    section(&mut text, "Original request", request);
    for earlier in &plan.steps[step].reads {
        let output = plan
            .steps
            .iter()
            .position(|s| s.role_id == *earlier)
            .and_then(|at| record.output_of(at));
        if let Some(output) = output {
            section(&mut text, &format!("Output from role: {earlier}"), &output);
        }
    }
    if let Some(revision) = revision {
        section(
            &mut text,
            &format!("Your previous draft (role: {role})"),
            &revision.draft,
        );
        section(
            &mut text,
            &format!("Feedback from role: {}", revision.critic),
            &revision.feedback,
        );
    }
    if is_critic {
        section(
            &mut text,
            "Verdict",
            &format!(
                "End your answer with one line that is exactly `{VERDICT_PASS_LINE}` or \
                 `{VERDICT_FAIL_LINE}`, with nothing else on that line. An answer without \
                 that line counts as a failing verdict."
            ),
        );
    }
    text
}

fn section(text: &mut String, heading: &str, body: &str) {
    let _ = write!(text, "\n\n## {heading}\n\n{}", body.trim());
}
