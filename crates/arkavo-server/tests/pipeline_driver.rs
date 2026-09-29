//! The pipeline driver, run against roles that are a script.
//!
//! Every test derives its plan from a kit manifest the way an agent does, and
//! hands the driver a host whose roles answer from a queue. What the driver
//! sent, to whom, in what order and with how much time is then a fact the
//! test can read back.

// The Tokio test entrypoint owns its runtime.
#![allow(clippy::disallowed_methods)]

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use arkavo_mcp_mesh::RequestError;
use arkavo_protocol::types::{Message, MessagePart};
use arkavo_server::server::pipeline::{
    FAILURE_CODE, FAILURE_SCHEMA, FailureKind, PipelineHost, Progress, RESULT_SCHEMA, RunFailure,
    RunRecord, run,
};
use arkavo_swarmkit::pipeline::{PipelinePlan, plan_from};
use async_trait::async_trait;
use serde_json::{Value, json};
use uuid::Uuid;

const REQUEST: &str = "Write the launch copy for the spring campaign.";

/// A kit of `roles`, each `(id, hands off to, can_read)`.
fn kit(roles: &[(&str, &str, &str)], tail: &str) -> String {
    let mut yaml = String::from(
        r#"
spec_version: "1.0.0"
kit:
  id: ""
  name: "fixture"
  version: "0.1.0"
  authors:
    - did: "did:web:example.com"
  created: "2026-04-29T00:00:00Z"
  nonce: "thz1Cz8aWOUURbyQQfvA0Q"
objective:
  goal: "ship the campaign"
roles:
"#,
    );
    for (id, to, can_read) in roles {
        let handoffs = if to.is_empty() {
            "[]".to_string()
        } else {
            format!("[{{to: {to}, on: always}}]")
        };
        yaml.push_str(&format!(
            "  - id: {id}\n    role_type: specialist\n    agent_provisioning: {{}}\n    \
             handoffs: {handoffs}\n    context_scope: {{can_read: {can_read}, can_write: [self]}}\n"
        ));
    }
    yaml.push_str(
        r#"coordination:
  topology: pipeline
  protocol: a2a-jsonrpc-2.0
  routing:
    strategy: static
provenance:
  signatures: []
"#,
    );
    yaml.push_str(tail);
    yaml
}

fn tail(budget_secs: u64, critic: Option<&str>, max_retries: u32, on_failure: &str) -> String {
    let evaluation = critic.map_or_else(String::new, |critic| {
        format!(
            "evaluation:\n  critic_role: {critic}\n  rubric:\n    dimensions:\n      \
             - {{name: quality, weight: 1.0, threshold: 0.5}}\n"
        )
    });
    format!(
        "constraints:\n  global_budget:\n    max_wallclock_seconds: {budget_secs}\n    \
         max_total_tokens: 100000\n    max_cost_usd: 1.0\n  network:\n    egress_allowed: false\n\
         {evaluation}completion:\n  rules: [\"done\"]\n  on_failure: {on_failure}\n  \
         max_retries: {max_retries}\n"
    )
}

const FOUR_ROLES: [(&str, &str, &str); 4] = [
    ("analyst", "copy", "[self]"),
    ("copy", "editor", "[analyst, self]"),
    ("editor", "critic", "[copy, self]"),
    ("critic", "", "[analyst, copy, editor, self]"),
];

fn plan(yaml: &str, entry: &str) -> PipelinePlan {
    let manifest = arkavo_swarmkit::parse_yaml(yaml).expect("the fixture kit is valid");
    plan_from(&manifest, entry).expect("the entry role drives")
}

fn four_roles(critic: Option<&str>, max_retries: u32, on_failure: &str) -> PipelinePlan {
    plan(
        &kit(&FOUR_ROLES, &tail(300, critic, max_retries, on_failure)),
        "analyst",
    )
}

/// What a scripted role does when it is asked.
enum Answer {
    Text(&'static str),
    /// Answer after a delay that ignores the time it was given.
    Late(Duration, &'static str),
    Fails(RequestError),
    EntryFails(&'static str),
}

/// One request the driver made of a role.
#[derive(Debug, Clone)]
struct Asked {
    role: String,
    text: String,
    /// `None` for the entry role, which is asked in-process.
    metadata: Option<Value>,
    timeout: Duration,
}

#[derive(Default)]
struct Roles {
    entry: String,
    answers: Mutex<HashMap<String, VecDeque<Answer>>>,
    asked: Mutex<Vec<Asked>>,
    reports: Mutex<Vec<Progress>>,
    canceled: AtomicBool,
}

impl Roles {
    fn new(entry: &str, script: Vec<(&str, Vec<Answer>)>) -> Self {
        Self {
            entry: entry.to_string(),
            answers: Mutex::new(
                script
                    .into_iter()
                    .map(|(role, answers)| (role.to_string(), answers.into()))
                    .collect(),
            ),
            ..Default::default()
        }
    }

    fn asked(&self) -> Vec<Asked> {
        self.asked.lock().expect("asked").clone()
    }

    fn order(&self) -> Vec<String> {
        self.asked().into_iter().map(|a| a.role).collect()
    }

    fn reports(&self) -> Vec<Progress> {
        self.reports.lock().expect("reports").clone()
    }

    fn next(&self, role: &str) -> Answer {
        self.answers
            .lock()
            .expect("answers")
            .get_mut(role)
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| panic!("the script has no answer left for {role}"))
    }

    async fn ask(&self, asked: Asked) -> Result<String, RequestError> {
        let role = asked.role.clone();
        self.asked.lock().expect("asked").push(asked);
        match self.next(&role) {
            Answer::Text(text) => Ok(text.to_string()),
            Answer::Late(delay, text) => {
                tokio::time::sleep(delay).await;
                Ok(text.to_string())
            }
            Answer::Fails(error) => Err(error),
            Answer::EntryFails(reason) => Err(RequestError::Refused {
                agent_id: role,
                reason: reason.to_string(),
            }),
        }
    }
}

#[async_trait]
impl PipelineHost for Roles {
    async fn answer_as_entry(&self, text: String, timeout: Duration) -> Result<String, String> {
        self.ask(Asked {
            role: self.entry.clone(),
            text,
            metadata: None,
            timeout,
        })
        .await
        .map_err(|e| match e {
            RequestError::Refused { reason, .. } => reason,
            other => other.to_string(),
        })
    }

    async fn send_and_wait(
        &self,
        role_id: &str,
        message: Message,
        timeout: Duration,
    ) -> Result<String, RequestError> {
        let text = match message.parts.as_slice() {
            [MessagePart::Text { content }] => content.clone(),
            parts => panic!("a step is one text part, got {parts:?}"),
        };
        self.ask(Asked {
            role: role_id.to_string(),
            text,
            metadata: message.metadata,
            timeout,
        })
        .await
    }

    async fn report(&self, progress: Progress) {
        self.reports.lock().expect("reports").push(progress);
    }

    async fn abandoned(&self) -> bool {
        self.canceled.load(Ordering::SeqCst)
    }
}

async fn drive(plan: &PipelinePlan, roles: &Roles) -> Result<RunRecord, Box<RunFailure>> {
    run(plan, Uuid::new_v4(), REQUEST, roles).await
}

fn section(role: &str) -> String {
    format!("## Output from role: {role}\n\n")
}

const ANALYSIS: &str = "Three selling points: range, price, warranty.";
const COPY: &str = "Go further for less, with five years of cover.";
const EDITED: &str = "Go further. Pay less. Covered for five years.";
const REVIEW_PASS: &str = "Every claim traces to the analysis.\nVERDICT: PASS";
const REVIEW_FAIL: &str = "\"Pay less\" is not supported by the analysis.\nVERDICT: FAIL";

fn happy_script() -> Vec<(&'static str, Vec<Answer>)> {
    vec![
        ("analyst", vec![Answer::Text(ANALYSIS)]),
        ("copy", vec![Answer::Text(COPY)]),
        ("editor", vec![Answer::Text(EDITED)]),
        ("critic", vec![Answer::Text(REVIEW_PASS)]),
    ]
}

#[tokio::test]
async fn four_roles_run_in_order_each_with_what_it_may_read() {
    let plan = four_roles(Some("critic"), 1, "abort");
    let roles = Roles::new("analyst", happy_script());

    let record = drive(&plan, &roles).await.expect("the run finished");

    assert_eq!(roles.order(), ["analyst", "copy", "editor", "critic"]);
    let asked = roles.asked();

    // The entry role works from the caller's request as it arrived.
    assert_eq!(asked[0].text, REQUEST);
    assert_eq!(asked[0].metadata, None);

    let copy = &asked[1].text;
    assert!(
        copy.starts_with("## Pipeline step 2 of 4: copy\n"),
        "{copy}"
    );
    assert!(
        copy.contains(&format!("## Original request\n\n{REQUEST}")),
        "{copy}"
    );
    assert!(
        copy.contains(&format!("{}{ANALYSIS}", section("analyst"))),
        "{copy}"
    );
    assert!(
        !copy.contains("VERDICT"),
        "only the critic is asked for one"
    );

    // `editor` declares can_read: [copy, self]. The analyst's output exists
    // and is not shown to it.
    let editor = &asked[2].text;
    assert!(
        editor.contains(&format!("{}{COPY}", section("copy"))),
        "{editor}"
    );
    assert!(!editor.contains(ANALYSIS), "{editor}");
    assert!(!editor.contains(&section("analyst")), "{editor}");

    let critic = &asked[3].text;
    for (role, output) in [("analyst", ANALYSIS), ("copy", COPY), ("editor", EDITED)] {
        assert!(
            critic.contains(&format!("{}{output}", section(role))),
            "{critic}"
        );
    }
    let analyst_at = critic.find(&section("analyst")).expect("analyst section");
    let editor_at = critic.find(&section("editor")).expect("editor section");
    assert!(analyst_at < editor_at, "sections follow pipeline order");
    assert!(
        critic.contains("exactly `VERDICT: PASS` or `VERDICT: FAIL`"),
        "{critic}"
    );

    assert_eq!(record.retries_used, 0);
    assert_eq!(record.final_answer().expect("final").output, REVIEW_PASS);
}

#[tokio::test]
async fn every_message_sent_carries_the_marker_of_its_step() {
    let plan = four_roles(Some("critic"), 1, "abort");
    let roles = Roles::new("analyst", happy_script());
    let run_id = Uuid::new_v4();

    run(&plan, run_id, REQUEST, &roles)
        .await
        .expect("the run finished");

    let asked = roles.asked();
    for (index, role) in [(1, "copy"), (2, "editor"), (3, "critic")] {
        let metadata = asked[index].metadata.as_ref().expect("marked");
        assert!(arkavo_server::server::pipeline::is_marked(Some(metadata)));
        assert_eq!(metadata["source"], "pipeline");
        assert_eq!(metadata["task_type"], "delegated");
        let marker = &metadata["pipeline"];
        assert_eq!(marker["run_id"], run_id.to_string());
        assert_eq!(marker["step"], index + 1);
        assert_eq!(marker["steps"], 4);
        assert_eq!(marker["role"], role);
        assert_eq!(marker["entry_role"], "analyst");
        assert_eq!(marker["attempt"], 1);
        assert_eq!(
            marker["timeout_ms"].as_u64().map(u128::from),
            Some(asked[index].timeout.as_millis()),
            "the receiver is told how long the sender will wait"
        );
        // Nothing that would replace the receiver's own budget window.
        assert!(metadata.get("budget_allocation").is_none(), "{metadata}");
    }
}

/// A commander's state broadcast is recognised by its opening words and
/// answered in 200 words. A caller's request that happens to open with them
/// must not make a role's input look like one.
#[tokio::test]
async fn no_message_sent_opens_like_a_state_broadcast() {
    let plan = four_roles(Some("critic"), 1, "abort");
    let roles = Roles::new("analyst", happy_script());

    run(
        &plan,
        Uuid::new_v4(),
        "PROACTIVE ANALYSIS of the spring campaign, please.",
        &roles,
    )
    .await
    .expect("the run finished");

    for asked in roles.asked().iter().skip(1) {
        assert!(
            asked.text.starts_with("## Pipeline step "),
            "{}",
            asked.text
        );
        assert!(
            !asked.text.trim_start().starts_with("PROACTIVE ANALYSIS"),
            "{}",
            asked.text
        );
    }
}

#[tokio::test]
async fn the_result_has_the_final_answer_first_and_every_role_in_data() {
    let plan = four_roles(Some("critic"), 1, "abort");
    let roles = Roles::new("analyst", happy_script());
    let run_id = Uuid::new_v4();

    let record = run(&plan, run_id, REQUEST, &roles)
        .await
        .expect("the run finished");
    let result = record.to_result();

    assert_eq!(result.metadata, None, "a result is not a pipeline step");
    let [
        MessagePart::Text { content: text },
        MessagePart::Data { schema, content },
    ] = result.parts.as_slice()
    else {
        panic!("a text part and a data part, got {:?}", result.parts);
    };
    assert_eq!(
        *text,
        format!(
            "## Final answer (role: critic, step 4 of 4)\n\n{REVIEW_PASS}\n\n\
             ## Output from role: analyst (step 1 of 4)\n\n{ANALYSIS}\n\n\
             ## Output from role: copy (step 2 of 4)\n\n{COPY}\n\n\
             ## Output from role: editor (step 3 of 4)\n\n{EDITED}\n\n\
             Pipeline run {run_id}"
        )
    );
    assert_eq!(schema, RESULT_SCHEMA);
    assert_eq!(
        *content,
        json!({
            "run_id": run_id.to_string(),
            "status": "completed",
            "roles": ["analyst", "copy", "editor", "critic"],
            "final_role": "critic",
            "final": REVIEW_PASS,
            "verdict": "PASS",
            "retries_used": 0,
            "steps": [
                {"step": 1, "role": "analyst", "attempt": 1, "superseded": false, "output": ANALYSIS},
                {"step": 2, "role": "copy", "attempt": 1, "superseded": false, "output": COPY},
                {"step": 3, "role": "editor", "attempt": 1, "superseded": false, "output": EDITED},
                {"step": 4, "role": "critic", "attempt": 1, "superseded": false, "output": REVIEW_PASS},
            ],
        })
    );
}

#[tokio::test]
async fn progress_names_each_role_and_its_step() {
    let plan = four_roles(None, 0, "abort");
    let roles = Roles::new("analyst", happy_script());

    drive(&plan, &roles).await.expect("the run finished");

    let expected: Vec<Progress> = ["analyst", "copy", "editor", "critic"]
        .iter()
        .enumerate()
        .flat_map(|(index, role)| {
            [
                Progress::Started {
                    role: (*role).to_string(),
                    step: index + 1,
                    steps: 4,
                    attempt: 1,
                },
                Progress::Finished {
                    role: (*role).to_string(),
                    step: index + 1,
                    steps: 4,
                    attempt: 1,
                },
            ]
        })
        .collect();
    assert_eq!(roles.reports(), expected);
}

#[tokio::test]
async fn a_failing_verdict_sends_the_draft_back_and_a_passing_one_ends_the_run() {
    const REVISED: &str = "Go further. Covered for five years.";
    let plan = four_roles(Some("critic"), 2, "abort");
    let roles = Roles::new(
        "analyst",
        vec![
            ("analyst", vec![Answer::Text(ANALYSIS)]),
            ("copy", vec![Answer::Text(COPY)]),
            ("editor", vec![Answer::Text(EDITED), Answer::Text(REVISED)]),
            (
                "critic",
                vec![Answer::Text(REVIEW_FAIL), Answer::Text(REVIEW_PASS)],
            ),
        ],
    );

    let record = drive(&plan, &roles).await.expect("the revision passed");

    assert_eq!(
        roles.order(),
        ["analyst", "copy", "editor", "critic", "editor", "critic"]
    );
    let asked = roles.asked();

    // The role before the critic gets its draft and the whole review.
    let revision = &asked[4].text;
    assert!(
        revision.starts_with("## Pipeline step 3 of 4: editor (revision 1 of 2)\n"),
        "{revision}"
    );
    assert!(
        revision.contains(&format!(
            "## Your previous draft (role: editor)\n\n{EDITED}"
        )),
        "{revision}"
    );
    assert!(
        revision.contains(&format!("## Feedback from role: critic\n\n{REVIEW_FAIL}")),
        "{revision}"
    );
    assert!(
        revision.contains(&format!("{}{COPY}", section("copy"))),
        "{revision}"
    );
    assert!(!revision.contains(ANALYSIS), "still outside its can_read");
    assert_eq!(
        asked[4].metadata.as_ref().expect("marked")["pipeline"]["attempt"],
        2
    );

    // The critic reviews the revision, not the draft it already failed.
    let second_review = &asked[5].text;
    assert!(
        second_review.contains(&format!("{}{REVISED}", section("editor"))),
        "{second_review}"
    );
    assert!(!second_review.contains(EDITED), "{second_review}");

    assert_eq!(record.retries_used, 1);
    assert_eq!(record.final_answer().expect("final").output, REVIEW_PASS);
    assert_eq!(record.output_of(2).as_deref(), Some(REVISED));
    let history: Vec<(usize, u32, bool)> = record
        .steps
        .iter()
        .map(|s| (s.step, s.attempt, s.superseded))
        .collect();
    assert_eq!(
        history,
        [
            (1, 1, false),
            (2, 1, false),
            (3, 1, true),
            (4, 1, true),
            (3, 2, false),
            (4, 2, false),
        ]
    );
    assert!(roles.reports().contains(&Progress::Revising {
        role: "editor".to_string(),
        critic: "critic".to_string(),
        verdict: arkavo_swarmkit::Verdict::Fail,
        revision: 1,
        max_retries: 2,
    }));

    let result = record.to_result();
    let MessagePart::Text { content } = &result.parts[0] else {
        panic!("text first");
    };
    assert!(
        content.contains(&format!(
            "## Output from role: editor (step 3 of 4, revision 1)\n\n{REVISED}"
        )),
        "{content}"
    );
    assert!(
        !content.contains(EDITED),
        "a replaced draft is not a result"
    );
}

#[tokio::test]
async fn a_verdict_that_keeps_failing_aborts_with_the_verdict_and_the_last_outputs() {
    const REVISED: &str = "Go further. Pay less than last year.";
    const SECOND_FAIL: &str = "\"Less than last year\" is not in the analysis.\nVERDICT: FAIL";
    let plan = four_roles(Some("critic"), 1, "abort");
    let roles = Roles::new(
        "analyst",
        vec![
            ("analyst", vec![Answer::Text(ANALYSIS)]),
            ("copy", vec![Answer::Text(COPY)]),
            ("editor", vec![Answer::Text(EDITED), Answer::Text(REVISED)]),
            (
                "critic",
                vec![Answer::Text(REVIEW_FAIL), Answer::Text(SECOND_FAIL)],
            ),
        ],
    );

    let failure = drive(&plan, &roles).await.expect_err("never passed");

    assert_eq!(
        roles.order(),
        ["analyst", "copy", "editor", "critic", "editor", "critic"],
        "one retry, then no more"
    );
    assert_eq!(failure.kind, FailureKind::VerdictFail);
    assert_eq!(failure.role, "critic");
    assert_eq!(failure.step, 4);
    assert_eq!(
        failure.reason,
        "critic \"critic\" gave VERDICT: FAIL; 1 of 1 retries were used; \
         completion.on_failure is abort"
    );

    let error = failure.to_task_error();
    assert_eq!(error.code, FAILURE_CODE);
    assert_eq!(
        error.message,
        format!(
            "pipeline run {} failed: {}\n\n\
             ## Last output from role: editor\n\n{REVISED}\n\n\
             ## Last output from role: critic\n\n{SECOND_FAIL}",
            failure.record.run_id, failure.reason
        )
    );
    let details = error.details.expect("details");
    assert_eq!(details["schema"], FAILURE_SCHEMA);
    assert_eq!(details["status"], "failed");
    assert_eq!(details["reason"], "verdict_fail");
    assert_eq!(details["role"], "critic");
    assert_eq!(details["step"], 4);
    assert_eq!(details["verdict"], "FAIL");
    assert_eq!(details["retries_used"], 1);
    assert_eq!(details["max_retries"], 1);
    assert_eq!(details["on_failure"], "abort");
    assert_eq!(details["steps"].as_array().map(Vec::len), Some(6));
    assert_eq!(details["steps"][5]["output"], SECOND_FAIL);
}

/// The pipeline fails closed: a review that never says PASS has not passed.
#[tokio::test]
async fn a_review_without_a_verdict_line_fails_the_run() {
    const NO_VERDICT: &str = "The copy reads well and I would ship it.";
    let plan = four_roles(Some("critic"), 0, "abort");
    let roles = Roles::new(
        "analyst",
        vec![
            ("analyst", vec![Answer::Text(ANALYSIS)]),
            ("copy", vec![Answer::Text(COPY)]),
            ("editor", vec![Answer::Text(EDITED)]),
            ("critic", vec![Answer::Text(NO_VERDICT)]),
        ],
    );

    let failure = drive(&plan, &roles).await.expect_err("no verdict");

    assert_eq!(failure.kind, FailureKind::VerdictMissing);
    assert_eq!(
        failure.reason,
        "critic \"critic\" gave no verdict line, which counts as VERDICT: FAIL; \
         completion.max_retries is 0; completion.on_failure is abort"
    );
    let error = failure.to_task_error();
    let details = error.details.expect("details");
    assert_eq!(details["reason"], "verdict_missing");
    assert_eq!(details["verdict"], "missing");
    assert!(error.message.contains(NO_VERDICT), "{}", error.message);
    assert_eq!(roles.order(), ["analyst", "copy", "editor", "critic"]);
}

/// A missing verdict is a failing one, so it earns a revision like any other.
#[tokio::test]
async fn a_missing_verdict_is_retried_like_a_failing_one() {
    let plan = four_roles(Some("critic"), 1, "abort");
    let roles = Roles::new(
        "analyst",
        vec![
            ("analyst", vec![Answer::Text(ANALYSIS)]),
            ("copy", vec![Answer::Text(COPY)]),
            ("editor", vec![Answer::Text(EDITED), Answer::Text(EDITED)]),
            (
                "critic",
                vec![Answer::Text("Unclear."), Answer::Text(REVIEW_PASS)],
            ),
        ],
    );

    let record = drive(&plan, &roles).await.expect("passed on review two");

    assert_eq!(record.retries_used, 1);
    assert_eq!(record.verdict, Some(arkavo_swarmkit::Verdict::Pass));
}

/// `retry`, `escalate` and `partial` parse, and nothing in the repository
/// says what a pipeline does for them. They end a run as `abort` does and
/// the reason says which was declared.
#[tokio::test]
async fn other_on_failure_values_are_carried_out_as_abort() {
    for declared in ["retry", "escalate", "partial"] {
        let plan = four_roles(Some("critic"), 0, declared);
        let roles = Roles::new(
            "analyst",
            vec![
                ("analyst", vec![Answer::Text(ANALYSIS)]),
                ("copy", vec![Answer::Text(COPY)]),
                ("editor", vec![Answer::Text(EDITED)]),
                ("critic", vec![Answer::Text(REVIEW_FAIL)]),
            ],
        );

        let failure = drive(&plan, &roles).await.expect_err("failed review");

        assert_eq!(failure.kind, FailureKind::VerdictFail, "{declared}");
        assert!(
            failure.reason.ends_with(&format!(
                "completion.on_failure is {declared}, which is carried out as abort"
            )),
            "{}",
            failure.reason
        );
        let details = failure.to_task_error().details.expect("details");
        assert_eq!(details["on_failure"], declared);
    }
}

fn not_found(role: &str) -> RequestError {
    RequestError::AgentNotFound {
        agent_id: role.to_string(),
        known: vec!["analyst".to_string(), "critic".to_string()],
    }
}

#[tokio::test]
async fn a_role_whose_agent_is_not_found_fails_the_run_naming_the_role() {
    let plan = four_roles(Some("critic"), 1, "abort");
    let roles = Roles::new(
        "analyst",
        vec![
            ("analyst", vec![Answer::Text(ANALYSIS)]),
            ("copy", vec![Answer::Fails(not_found("copy"))]),
        ],
    );

    let failure = drive(&plan, &roles).await.expect_err("copy is missing");

    assert_eq!(roles.order(), ["analyst", "copy"], "no role after it ran");
    assert_eq!(failure.kind, FailureKind::AgentNotFound);
    assert_eq!(failure.role, "copy");
    assert_eq!(failure.step, 2);
    let error = failure.to_task_error();
    assert_eq!(
        error.message,
        format!(
            "pipeline run {} failed at role \"copy\" (step 2 of 4): agent \"copy\" was not \
             found; agents known here: analyst, critic",
            failure.record.run_id
        )
    );
    let details = error.details.expect("details");
    assert_eq!(details["reason"], "agent_not_found");
    // What had been produced is reported as what it is: part of a failed run.
    assert_eq!(details["status"], "failed");
    assert_eq!(details["steps"].as_array().map(Vec::len), Some(1));
    assert_eq!(details["steps"][0]["output"], ANALYSIS);
}

#[tokio::test]
async fn a_role_that_times_out_fails_the_run_naming_the_role() {
    let plan = four_roles(Some("critic"), 1, "abort");
    let roles = Roles::new(
        "analyst",
        vec![
            ("analyst", vec![Answer::Text(ANALYSIS)]),
            ("copy", vec![Answer::Text(COPY)]),
            (
                "editor",
                vec![Answer::Fails(RequestError::TimedOut {
                    agent_id: "editor".to_string(),
                    task_id: Some("7f1f4a2e-6f0e-4c3b-9d67-2f6c1f0a9b11".to_string()),
                    waited: Duration::from_secs(90),
                })],
            ),
        ],
    );

    let failure = drive(&plan, &roles).await.expect_err("editor timed out");

    assert_eq!(roles.order(), ["analyst", "copy", "editor"]);
    assert_eq!(failure.kind, FailureKind::TimedOut);
    assert_eq!(failure.role, "editor");
    assert_eq!(
        failure.summary(),
        format!(
            "pipeline run {} failed at role \"editor\" (step 3 of 4): agent \"editor\" did \
             not answer within 90s",
            failure.record.run_id
        )
    );
}

#[tokio::test]
async fn each_way_a_role_can_fail_is_told_apart() {
    let task_id = "7f1f4a2e-6f0e-4c3b-9d67-2f6c1f0a9b11".to_string();
    let copy = "copy".to_string();
    for (error, reason) in [
        (
            RequestError::Failed {
                agent_id: copy.clone(),
                task_id: task_id.clone(),
                code: "AGENT_CYCLE_FAILED".to_string(),
                message: "compute budget exhausted".to_string(),
            },
            "step_failed",
        ),
        (
            RequestError::Canceled {
                agent_id: copy.clone(),
                task_id: task_id.clone(),
            },
            "step_canceled",
        ),
        (
            RequestError::Rejected {
                agent_id: copy.clone(),
                task_id: task_id.clone(),
            },
            "step_rejected",
        ),
        (
            RequestError::Unreachable {
                agent_id: copy.clone(),
                reason: "connection refused".to_string(),
            },
            "unreachable",
        ),
        (
            RequestError::Refused {
                agent_id: copy.clone(),
                reason: "-32001: Blocked by policy: block-pii".to_string(),
            },
            "refused",
        ),
    ] {
        let plan = four_roles(None, 0, "abort");
        let said = error.to_string();
        let roles = Roles::new(
            "analyst",
            vec![
                ("analyst", vec![Answer::Text(ANALYSIS)]),
                ("copy", vec![Answer::Fails(error)]),
            ],
        );

        let failure = drive(&plan, &roles).await.expect_err("copy failed");

        assert_eq!(failure.kind.as_str(), reason);
        assert_eq!(failure.role, "copy");
        assert_eq!(failure.reason, said);
    }
}

#[tokio::test]
async fn an_entry_role_that_cannot_answer_fails_the_run() {
    let plan = four_roles(None, 0, "abort");
    let roles = Roles::new(
        "analyst",
        vec![(
            "analyst",
            vec![Answer::EntryFails("Budget exceeded: no inferences left")],
        )],
    );

    let failure = drive(&plan, &roles).await.expect_err("entry failed");

    assert_eq!(failure.kind, FailureKind::EntryFailed);
    assert_eq!(failure.role, "analyst");
    assert_eq!(failure.step, 1);
    assert_eq!(failure.reason, "Budget exceeded: no inferences left");
    assert!(failure.record.steps.is_empty());
}

#[tokio::test]
async fn a_role_that_writes_nothing_fails_the_run() {
    let plan = four_roles(None, 0, "abort");
    let roles = Roles::new(
        "analyst",
        vec![
            ("analyst", vec![Answer::Text(ANALYSIS)]),
            ("copy", vec![Answer::Text("  \n")]),
        ],
    );

    let failure = drive(&plan, &roles).await.expect_err("nothing written");

    assert_eq!(failure.kind, FailureKind::NoOutput);
    assert_eq!(failure.role, "copy");
    assert_eq!(roles.order(), ["analyst", "copy"]);
}

fn two_roles(budget_secs: u64) -> PipelinePlan {
    plan(
        &kit(
            &[
                ("writer", "reviewer", "[self]"),
                ("reviewer", "", "[writer]"),
            ],
            &tail(budget_secs, None, 0, "abort"),
        ),
        "writer",
    )
}

/// The run is held to the kit's budget even by a role that ignores the time
/// it was given.
#[tokio::test]
async fn a_run_that_outlasts_the_kit_budget_fails_at_the_role_it_was_in() {
    let plan = two_roles(1);
    let roles = Roles::new(
        "writer",
        vec![
            ("writer", vec![Answer::Text("A first draft.")]),
            (
                "reviewer",
                vec![Answer::Late(Duration::from_secs(30), "Too late to matter.")],
            ),
        ],
    );

    let started = std::time::Instant::now();
    let failure = drive(&plan, &roles).await.expect_err("out of time");
    let took = started.elapsed();

    assert!(took >= Duration::from_secs(1), "ended after {took:?}");
    assert!(took < Duration::from_secs(10), "ended after {took:?}");
    assert_eq!(failure.kind, FailureKind::BudgetExhausted);
    assert_eq!(failure.role, "reviewer");
    assert_eq!(failure.step, 2);
    assert_eq!(
        failure.reason,
        "the run's time budget of 1s (constraints.global_budget.max_wallclock_seconds) \
         was used up"
    );
    let details = failure.to_task_error().details.expect("details");
    assert_eq!(details["reason"], "budget_exhausted");
    assert_eq!(details["steps"][0]["output"], "A first draft.");
}

/// A role is never started with no time to answer in.
#[tokio::test]
async fn no_role_is_started_once_the_budget_is_gone() {
    let plan = plan(
        &kit(
            &[
                ("writer", "reviewer", "[self]"),
                ("reviewer", "publisher", "[writer]"),
                ("publisher", "", "[reviewer]"),
            ],
            &tail(1, None, 0, "abort"),
        ),
        "writer",
    );
    let roles = Roles::new(
        "writer",
        vec![
            ("writer", vec![Answer::Text("A first draft.")]),
            (
                "reviewer",
                vec![Answer::Late(Duration::from_secs(30), "Too late.")],
            ),
        ],
    );

    let failure = drive(&plan, &roles).await.expect_err("out of time");

    assert_eq!(failure.kind, FailureKind::BudgetExhausted);
    assert_eq!(roles.order(), ["writer", "reviewer"], "publisher never ran");
}

#[tokio::test]
async fn each_role_is_given_what_is_left_of_the_budget() {
    let plan = two_roles(60);
    let roles = Roles::new(
        "writer",
        vec![
            (
                "writer",
                vec![Answer::Late(Duration::from_millis(300), "A first draft.")],
            ),
            ("reviewer", vec![Answer::Text("Reads well.")]),
        ],
    );

    drive(&plan, &roles).await.expect("the run finished");

    let asked = roles.asked();
    let budget = Duration::from_secs(60);
    assert!(asked[0].timeout <= budget, "{:?}", asked[0].timeout);
    assert!(
        asked[0].timeout > budget - Duration::from_secs(5),
        "{:?}",
        asked[0].timeout
    );
    assert!(
        asked[1].timeout <= asked[0].timeout - Duration::from_millis(300),
        "the writer's time came out of the reviewer's: {:?} then {:?}",
        asked[0].timeout,
        asked[1].timeout
    );
}

#[tokio::test]
async fn a_canceled_task_stops_the_run_before_the_next_role() {
    let plan = two_roles(60);
    let roles = Roles::new("writer", vec![]);
    roles.canceled.store(true, Ordering::SeqCst);

    let failure = drive(&plan, &roles).await.expect_err("canceled");

    assert_eq!(failure.kind, FailureKind::Canceled);
    assert!(roles.order().is_empty(), "nobody was asked");
}

/// With the critic second, the role it sends back to is the entry role, and
/// the entry role revises in its own agent.
#[tokio::test]
async fn a_critic_right_after_the_entry_role_sends_the_entry_role_back() {
    let plan = plan(
        &kit(
            &[
                ("writer", "critic", "[self]"),
                ("critic", "publisher", "[writer]"),
                ("publisher", "", "[writer, critic]"),
            ],
            &tail(300, Some("critic"), 1, "abort"),
        ),
        "writer",
    );
    let roles = Roles::new(
        "writer",
        vec![
            (
                "writer",
                vec![Answer::Text("Draft one."), Answer::Text("Draft two.")],
            ),
            (
                "critic",
                vec![
                    Answer::Text("Too short.\nVERDICT: FAIL"),
                    Answer::Text("Better.\nVERDICT: PASS"),
                ],
            ),
            ("publisher", vec![Answer::Text("Published draft two.")]),
        ],
    );

    let record = drive(&plan, &roles).await.expect("the run finished");

    assert_eq!(
        roles.order(),
        ["writer", "critic", "writer", "critic", "publisher"]
    );
    let asked = roles.asked();
    assert_eq!(
        asked[2].metadata, None,
        "the entry role is asked in-process"
    );
    assert!(
        asked[2]
            .text
            .starts_with("## Pipeline step 1 of 3: writer (revision 1 of 1)\n"),
        "{}",
        asked[2].text
    );
    assert!(
        asked[2]
            .text
            .contains("## Feedback from role: critic\n\nToo short.\nVERDICT: FAIL"),
        "{}",
        asked[2].text
    );
    let publisher = &asked[4].text;
    assert!(publisher.contains("Draft two."), "{publisher}");
    assert!(!publisher.contains("Draft one."), "{publisher}");
    assert!(
        !publisher.contains("exactly `VERDICT"),
        "only the critic is asked for a verdict: {publisher}"
    );
    assert_eq!(
        record.final_answer().expect("final").output,
        "Published draft two."
    );
}
