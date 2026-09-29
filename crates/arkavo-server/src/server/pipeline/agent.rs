//! The pipeline as the entry role's agent takes part in it: deciding
//! whether a message starts a run, and driving the run against the mesh.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use arkavo_mcp_mesh::RequestError;
use arkavo_protocol::types::{Message, TaskProgress, TaskStatus};
use arkavo_swarmkit::Manifest;
use arkavo_swarmkit::pipeline::{PipelineError, PipelinePlan, plan_from};
use async_trait::async_trait;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::{PipelineHost, Progress, run};
use crate::server::handlers::messaging::direct::{DirectExecution, Request};

/// Whether a message to this agent starts a pipeline run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::server) enum Driving {
    /// The agent answers for itself, as it does without a pipeline.
    No,
    /// The agent runs the entry role of this pipeline.
    Plan(Box<PipelinePlan>),
    /// The kit could not be read, so it is not known whether the agent is
    /// an entry role. The message is failed with this reason: answering it
    /// alone could pass one role's output off as the pipeline's.
    Unknown(String),
}

/// What `manifest` makes of the agent running `role_id`.
pub(in crate::server) fn driving_for(manifest: &Manifest, role_id: &str) -> Driving {
    match plan_from(manifest, role_id) {
        Ok(plan) => Driving::Plan(Box::new(plan)),
        Err(
            reason @ (PipelineError::NotPipeline { .. }
            | PipelineError::NotEntry { .. }
            | PipelineError::NotInPipeline(_)),
        ) => {
            debug!(role = role_id, "Not driving a pipeline: {reason}");
            Driving::No
        }
        Err(reason @ PipelineError::UnknownRole(_)) => {
            warn!(role = role_id, "Not driving a pipeline: {reason}");
            Driving::No
        }
        Err(
            reason @ (PipelineError::FanOut { .. }
            | PipelineError::FanIn { .. }
            | PipelineError::Cycle { .. }),
        ) => Driving::Unknown(format!("the kit's pipeline cannot be run: {reason}")),
    }
}

/// The last decision read from the kit, kept for when the kit cannot be
/// read. A kit file caught halfway through an edit should not change what a
/// running agent does with its messages.
fn last_known() -> &'static Mutex<Option<(String, Driving)>> {
    static LAST_KNOWN: OnceLock<Mutex<Option<(String, Driving)>>> = OnceLock::new();
    LAST_KNOWN.get_or_init(|| Mutex::new(None))
}

/// What the kit this process was started from makes of `role_id`.
///
/// The kit is found the way the agent found it at startup and read again for
/// each message, so an edited kit is followed as it stands.
pub(in crate::server) async fn driving_from_kit(role_id: Option<&str>) -> Driving {
    let Some(role_id) = role_id else {
        return Driving::No;
    };
    let loaded = tokio::task::spawn_blocking(|| {
        let cwd = std::env::current_dir().map_err(|e| e.to_string())?;
        arkavo_swarmkit::load_discovered_kit(&cwd)
            .map(|kit| kit.manifest)
            .map_err(|e| e.to_string())
    })
    .await
    .unwrap_or_else(|e| Err(format!("reading the kit was interrupted: {e}")));
    decide(loaded, role_id, last_known())
}

/// The decision for `role_id` from a kit that was read, or from the last
/// one that could be when this one could not.
fn decide(
    loaded: Result<Manifest, String>,
    role_id: &str,
    last_known: &Mutex<Option<(String, Driving)>>,
) -> Driving {
    let failure = match loaded {
        Ok(manifest) => {
            let driving = driving_for(&manifest, role_id);
            if let Ok(mut known) = last_known.lock() {
                *known = Some((role_id.to_string(), driving.clone()));
            }
            return driving;
        }
        Err(failure) => failure,
    };

    let remembered = last_known.lock().ok().and_then(|known| known.clone());
    match remembered {
        Some((role, driving)) if role == role_id => {
            warn!(
                role = role_id,
                "Kit could not be read ({failure}); keeping the pipeline decision last read from it"
            );
            driving
        }
        _ => Driving::Unknown(format!(
            "this agent runs role {role_id:?} of a kit that cannot be read ({failure}), so \
             whether the message starts a pipeline is not known"
        )),
    }
}

/// Run the pipeline for the message behind `task_id` and write the outcome
/// to that task.
///
/// The run is the only writer of the task's outcome. A task the caller
/// canceled meanwhile is left canceled.
pub(in crate::server) async fn drive_task(
    direct: DirectExecution,
    plan: PipelinePlan,
    task_id: Uuid,
    request: String,
    images: Option<Vec<String>>,
) {
    let run_id = Uuid::new_v4();
    let executor = direct.task_executor.clone();
    info!(
        run_id = %run_id,
        task_id = %task_id,
        roles = ?plan.steps.iter().map(|s| s.role_id.as_str()).collect::<Vec<_>>(),
        critic = ?plan.critic.map(|gate| plan.role_at(gate.step)),
        budget_secs = plan.max_wallclock.as_secs(),
        "Pipeline run started"
    );
    if let Err(e) = executor
        .update_task_status(&task_id, TaskStatus::Working)
        .await
    {
        warn!("Failed to update task {task_id} to Working: {e}");
    }

    let host = AgentHost {
        direct,
        task_id,
        run_id,
        images,
    };
    let outcome = run(&plan, run_id, &request, &host).await;
    if host.abandoned().await {
        info!(run_id = %run_id, task_id = %task_id, "Pipeline run ended for a canceled task");
        return;
    }
    match outcome {
        Ok(record) => {
            let result =
                serde_json::to_value(record.to_result()).unwrap_or(serde_json::Value::Null);
            match executor.complete_task(&task_id, result).await {
                Ok(()) => info!(
                    run_id = %run_id,
                    task_id = %task_id,
                    retries_used = record.retries_used,
                    "Pipeline run completed"
                ),
                Err(e) => warn!("Failed to complete task {task_id}: {e}"),
            }
        }
        Err(failure) => {
            warn!(
                run_id = %run_id,
                task_id = %task_id,
                reason = failure.kind.as_str(),
                role = %failure.role,
                "Pipeline run failed: {}",
                failure.summary()
            );
            if let Err(e) = executor.fail_task(&task_id, failure.to_task_error()).await {
                warn!("Failed to mark task {task_id} as failed: {e}");
            }
        }
    }
}

/// A run's view of the agent it runs in.
struct AgentHost {
    direct: DirectExecution,
    task_id: Uuid,
    run_id: Uuid,
    /// Images that came with the caller's message, for the entry role.
    images: Option<Vec<String>>,
}

#[async_trait]
impl PipelineHost for AgentHost {
    async fn answer_as_entry(&self, text: String, timeout: Duration) -> Result<String, String> {
        let request = Request {
            images: self.images.clone(),
            ..Request::isolated(text, None)
        };
        tokio::time::timeout(timeout, self.direct.answer(request))
            .await
            .unwrap_or_else(|_| {
                Err(format!(
                    "the entry role did not answer within {}s",
                    timeout.as_secs()
                ))
            })
    }

    async fn send_and_wait(
        &self,
        role_id: &str,
        message: Message,
        timeout: Duration,
    ) -> Result<String, RequestError> {
        let Some(mesh) = &self.direct.mesh_state else {
            return Err(RequestError::Unreachable {
                agent_id: role_id.to_string(),
                reason: "this agent was started without mesh access".to_string(),
            });
        };
        arkavo_mcp_mesh::send_and_wait(mesh, role_id, message, timeout)
            .await
            .map(|answer| answer.text)
    }

    async fn report(&self, progress: Progress) {
        let (message, done, of) = match progress {
            Progress::Started {
                role,
                step,
                steps,
                attempt,
            } => (
                format!(
                    "pipeline run {}: {role} is working (step {step} of {steps}{})",
                    self.run_id,
                    nth_attempt(attempt)
                ),
                step - 1,
                steps,
            ),
            Progress::Finished {
                role,
                step,
                steps,
                attempt,
            } => (
                format!(
                    "pipeline run {}: {role} finished (step {step} of {steps}{})",
                    self.run_id,
                    nth_attempt(attempt)
                ),
                step,
                steps,
            ),
            Progress::Revising {
                role,
                critic,
                verdict,
                revision,
                max_retries,
            } => {
                // Percentage is left as it was: a revision is not progress
                // toward the end, and the message says what is happening.
                self.write_progress(
                    format!(
                        "pipeline run {}: {critic} gave VERDICT: {}; {role} is revising \
                         (revision {revision} of {max_retries})",
                        self.run_id,
                        verdict.as_str()
                    ),
                    None,
                )
                .await;
                return;
            }
        };
        let percentage = u8::try_from(done * 100 / of.max(1)).unwrap_or(100);
        self.write_progress(message, Some(percentage.min(100)))
            .await;
    }

    async fn abandoned(&self) -> bool {
        matches!(
            self.direct
                .task_executor
                .get_task_status(&self.task_id)
                .await,
            Ok(TaskStatus::Canceled)
        )
    }
}

impl AgentHost {
    async fn write_progress(&self, message: String, percentage: Option<u8>) {
        let progress = TaskProgress {
            message: Some(message),
            percentage,
            eta_seconds: None,
        };
        if let Err(e) = self
            .direct
            .task_executor
            .update_task_progress(&self.task_id, progress)
            .await
        {
            debug!("Progress not recorded for task {}: {e}", self.task_id);
        }
    }
}

fn nth_attempt(attempt: u32) -> String {
    if attempt > 1 {
        format!(", revision {}", attempt - 1)
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KIT: &str = r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "two-step"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "write and review"
roles:
  - id: writer
    role_type: specialist
    agent_provisioning: {}
    handoffs: [{to: reviewer, on: always}]
  - id: reviewer
    role_type: critic
    agent_provisioning: {}
coordination:
  topology: pipeline
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
constraints:
  global_budget:
    max_wallclock_seconds: 60
    max_total_tokens: 100000
    max_cost_usd: 1.0
  network:
    egress_allowed: false
completion:
  rules: ["done"]
  on_failure: abort
  max_retries: 0
provenance:
  signatures: []
"#;

    fn kit() -> Manifest {
        arkavo_swarmkit::parse_yaml(KIT).expect("the fixture kit is valid")
    }

    fn nothing_known() -> Mutex<Option<(String, Driving)>> {
        Mutex::new(None)
    }

    #[test]
    fn the_entry_role_drives_and_the_others_do_not() {
        let Driving::Plan(plan) = driving_for(&kit(), "writer") else {
            panic!("the entry role drives");
        };
        assert_eq!(plan.entry_role(), "writer");
        assert_eq!(plan.final_role(), "reviewer");
        assert_eq!(driving_for(&kit(), "reviewer"), Driving::No);
        assert_eq!(driving_for(&kit(), "stranger"), Driving::No);
    }

    #[tokio::test]
    async fn an_agent_started_without_a_kit_role_does_not_drive() {
        assert_eq!(driving_from_kit(None).await, Driving::No);
    }

    /// A pipeline the kit cannot describe is not a reason to answer alone:
    /// the caller asked for the pipeline and is told it cannot be run.
    #[test]
    fn handoffs_that_cannot_be_run_are_unknown_not_no() {
        // Built past `validate`, which refuses this kit; a kit edited on
        // disk after the agent started can still reach the driver.
        let mut broken = kit();
        broken.roles[1].handoffs = broken.roles[0].handoffs.clone();
        broken.roles[1].handoffs[0].to = "writer".to_string();

        let Driving::Unknown(reason) = driving_for(&broken, "writer") else {
            panic!("a cycle is not a plan");
        };
        assert_eq!(
            reason,
            "the kit's pipeline cannot be run: handoffs form a cycle: writer -> reviewer -> writer"
        );
    }

    #[test]
    fn a_kit_that_was_read_decides_and_is_remembered() {
        let known = nothing_known();

        let decided = decide(Ok(kit()), "writer", &known);

        assert!(matches!(decided, Driving::Plan(_)));
        let remembered = known.lock().expect("lock").clone();
        assert_eq!(remembered, Some(("writer".to_string(), decided)));
    }

    /// Regression guard: a kit caught halfway through an edit must not turn
    /// an entry role into an agent that answers alone, which would hand the
    /// caller one role's output as if it were the pipeline's.
    #[test]
    fn an_unreadable_kit_keeps_the_last_decision() {
        let known = nothing_known();
        let before = decide(Ok(kit()), "writer", &known);

        let during = decide(
            Err("parse kit.yaml: mapping values".to_string()),
            "writer",
            &known,
        );

        assert_eq!(during, before);
    }

    #[test]
    fn an_unreadable_kit_with_nothing_remembered_is_unknown() {
        let decided = decide(
            Err("read kit.yaml: No such file or directory".to_string()),
            "writer",
            &nothing_known(),
        );

        assert_eq!(
            decided,
            Driving::Unknown(
                "this agent runs role \"writer\" of a kit that cannot be read (read kit.yaml: \
                 No such file or directory), so whether the message starts a pipeline is not \
                 known"
                    .to_string()
            )
        );
    }

    #[test]
    fn a_decision_remembered_for_another_role_is_not_used() {
        let known = nothing_known();
        decide(Ok(kit()), "reviewer", &known);

        let decided = decide(Err("unreadable".to_string()), "writer", &known);

        assert!(matches!(decided, Driving::Unknown(_)), "{decided:?}");
    }

    #[test]
    fn revisions_are_named_in_progress() {
        assert_eq!(nth_attempt(1), "");
        assert_eq!(nth_attempt(2), ", revision 1");
        assert_eq!(nth_attempt(3), ", revision 2");
    }
}
