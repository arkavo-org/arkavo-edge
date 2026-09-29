//! Running a pipeline kit's roles in order, from the entry role's agent.
//!
//! Each role of a kit runs as its own agent. When the kit's topology is
//! `pipeline`, a message to the entry role's agent is a request for the whole
//! pipeline: that agent answers it as its own role, hands the request and the
//! answers so far to each next role in turn, and completes the caller's task
//! with what the last role produced. The other agents need to know nothing
//! about pipelines. They receive a message, answer it, and are polled.
//!
//! [`run`] is the driver. It reaches the roles through a [`PipelineHost`], so
//! it can be run against real agents or against a script. A run either ends
//! with every role's output or fails naming the role and the reason; it never
//! reports the outputs of a run that stopped early as a result.

use std::time::Duration;

use arkavo_mcp_mesh::RequestError;
use arkavo_protocol::types::Message;
use arkavo_swarmkit::pipeline::{CriticGate, PipelinePlan, Verdict, parse_verdict};
use async_trait::async_trait;
use tokio::time::Instant;
use tracing::{info, warn};
use uuid::Uuid;

mod agent;
mod brief;
mod marker;
mod record;
mod step;

#[cfg(test)]
pub(super) use agent::driving_for;
pub(super) use agent::{Driving, drive_task, driving_from_kit};
pub use brief::{RESULT_SCHEMA, Revision, marker, step_text};
pub use marker::{MARKER_KEY, is_marked, step_timeout};
pub use record::{FAILURE_CODE, FAILURE_SCHEMA, FailureKind, RunFailure, RunRecord, StepRecord};
pub(super) use step::answer_step;

/// What happened in a run that a caller watching its task should see.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Progress {
    /// A role was given its input.
    Started {
        role: String,
        step: usize,
        steps: usize,
        attempt: u32,
    },
    /// A role answered.
    Finished {
        role: String,
        step: usize,
        steps: usize,
        attempt: u32,
    },
    /// The critic did not pass the work and the role before it will revise.
    Revising {
        role: String,
        critic: String,
        verdict: Verdict,
        revision: u32,
        max_retries: u32,
    },
}

/// How a run reaches the roles and the caller.
#[async_trait]
pub trait PipelineHost: Send + Sync {
    /// Answer `text` as the entry role, in this agent, within `timeout`.
    async fn answer_as_entry(&self, text: String, timeout: Duration) -> Result<String, String>;

    /// Send `message` to the agent running `role_id` and wait up to `timeout`
    /// for its answer.
    async fn send_and_wait(
        &self,
        role_id: &str,
        message: Message,
        timeout: Duration,
    ) -> Result<String, RequestError>;

    /// Tell the caller how far the run has come. Advisory: a report that
    /// cannot be delivered does not stop the run.
    async fn report(&self, progress: Progress);

    /// True when the caller no longer wants the result, so the roles still
    /// to run would work for nobody.
    async fn abandoned(&self) -> bool;
}

/// Run the pipeline `plan` describes for `request`.
///
/// The whole run, revisions included, is bounded by `plan.max_wallclock`.
/// Each role is given whatever is left of that budget to answer in.
///
/// A failure carries every answer the run had collected, so it is boxed to
/// keep the common path's return value small.
pub async fn run(
    plan: &PipelinePlan,
    run_id: Uuid,
    request: &str,
    host: &dyn PipelineHost,
) -> Result<RunRecord, Box<RunFailure>> {
    let mut run = Run {
        plan,
        request,
        host,
        deadline: Instant::now() + plan.max_wallclock,
        record: RunRecord::new(run_id, plan),
    };
    match run.roles_in_order().await {
        Ok(()) => Ok(run.record),
        Err(stopped) => Err(Box::new(run.record.failed(plan, stopped))),
    }
}

/// Why a run stopped, before it is joined with what the run had produced.
pub(crate) struct Stopped {
    pub kind: FailureKind,
    pub step: usize,
    pub reason: String,
}

struct Run<'a> {
    plan: &'a PipelinePlan,
    request: &'a str,
    host: &'a dyn PipelineHost,
    deadline: Instant,
    record: RunRecord,
}

impl Run<'_> {
    async fn roles_in_order(&mut self) -> Result<(), Stopped> {
        let steps = self.plan.steps.len();
        let mut step = 0;
        let mut revision: Option<Revision> = None;
        while step < steps {
            let output = self.run_step(step, revision.take()).await?;
            match self.plan.critic {
                Some(gate) if gate.step == step => {
                    let verdict = parse_verdict(&output);
                    self.record.verdict = Some(verdict);
                    if verdict.passed() {
                        step += 1;
                    } else {
                        revision = Some(self.send_back(gate, verdict, output).await?);
                        step = gate.step - 1;
                    }
                }
                _ => step += 1,
            }
        }
        Ok(())
    }

    /// Give one role its input and record what it answered.
    async fn run_step(
        &mut self,
        step: usize,
        revision: Option<Revision>,
    ) -> Result<String, Stopped> {
        let role = self.plan.role_at(step).to_string();
        let steps = self.plan.steps.len();
        let stopped = |kind, reason: String| Stopped { kind, step, reason };

        if self.host.abandoned().await {
            return Err(stopped(
                FailureKind::Canceled,
                "the caller canceled the task".to_string(),
            ));
        }
        let left = self.deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Err(stopped(FailureKind::BudgetExhausted, self.out_of_time()));
        }

        let attempt = self.record.attempts_of(step) + 1;
        let text = step_text(
            self.plan,
            step,
            self.request,
            &self.record,
            revision.as_ref(),
        );
        self.host
            .report(Progress::Started {
                role: role.clone(),
                step: step + 1,
                steps,
                attempt,
            })
            .await;
        info!(
            run_id = %self.record.run_id,
            role = %role,
            step = step + 1,
            steps,
            attempt,
            budget_left_secs = left.as_secs(),
            "Pipeline step started"
        );
        let started = Instant::now();

        // The host is asked to keep to the budget itself. The timer here is
        // what holds the run to it when the host does not.
        let answered = tokio::time::timeout_at(self.deadline, async {
            if step == 0 {
                self.host
                    .answer_as_entry(text, left)
                    .await
                    .map_err(|e| (FailureKind::EntryFailed, e))
            } else {
                let message =
                    brief::message(text, marker(&self.record, self.plan, step, attempt, left));
                self.host
                    .send_and_wait(&role, message, left)
                    .await
                    .map_err(|e| (FailureKind::from(&e), e.to_string()))
            }
        })
        .await;
        let output = match answered {
            Err(_) => return Err(stopped(FailureKind::BudgetExhausted, self.out_of_time())),
            Ok(Err((kind, reason))) => return Err(stopped(kind, reason)),
            Ok(Ok(output)) => output,
        };
        if output.trim().is_empty() {
            return Err(stopped(
                FailureKind::NoOutput,
                "the role finished without any output".to_string(),
            ));
        }

        info!(
            run_id = %self.record.run_id,
            role = %role,
            step = step + 1,
            steps,
            attempt,
            chars = output.len(),
            elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "Pipeline step finished"
        );
        self.record.answered(step, &role, output.clone());
        self.host
            .report(Progress::Finished {
                role,
                step: step + 1,
                steps,
                attempt,
            })
            .await;
        Ok(output)
    }

    /// Decide what a verdict that did not pass leads to: a revision while
    /// retries remain, and the end of the run once they are used up.
    async fn send_back(
        &mut self,
        gate: CriticGate,
        verdict: Verdict,
        feedback: String,
    ) -> Result<Revision, Stopped> {
        let critic = self.plan.role_at(gate.step).to_string();
        let drafter = self.plan.role_at(gate.step - 1).to_string();
        if self.record.retries_used >= gate.max_retries {
            warn!(
                run_id = %self.record.run_id,
                critic = %critic,
                verdict = verdict.as_str(),
                retries_used = self.record.retries_used,
                "Pipeline critic did not pass the work and no retries are left"
            );
            return Err(Stopped {
                kind: match verdict {
                    Verdict::Missing => FailureKind::VerdictMissing,
                    Verdict::Pass | Verdict::Fail => FailureKind::VerdictFail,
                },
                step: gate.step,
                reason: record::verdict_reason(&critic, verdict, self.record.retries_used, gate),
            });
        }

        let draft = self.record.output_of(gate.step - 1).unwrap_or_default();
        self.record.retries_used += 1;
        self.record.supersede(gate.step - 1);
        self.record.supersede(gate.step);
        info!(
            run_id = %self.record.run_id,
            critic = %critic,
            drafter = %drafter,
            verdict = verdict.as_str(),
            revision = self.record.retries_used,
            max_retries = gate.max_retries,
            "Pipeline critic did not pass the work; asking for a revision"
        );
        self.host
            .report(Progress::Revising {
                role: drafter,
                critic: critic.clone(),
                verdict,
                revision: self.record.retries_used,
                max_retries: gate.max_retries,
            })
            .await;
        Ok(Revision {
            number: self.record.retries_used,
            of: gate.max_retries,
            draft,
            critic,
            feedback,
        })
    }

    fn out_of_time(&self) -> String {
        format!(
            "the run's time budget of {}s \
             (constraints.global_budget.max_wallclock_seconds) was used up",
            self.plan.max_wallclock.as_secs()
        )
    }
}
