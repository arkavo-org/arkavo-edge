//! What an agent tells the operator watching `arkavo ui` (#759).
//!
//! An agent that detects an anomaly raises a notice: how serious it is,
//! what happened, where in the workspace to look, the details behind it,
//! and what the operator can do. Notices travel in the agent's metrics
//! snapshot until the operator acknowledges them, so a workspace that
//! reconnects still sees what is open.

use serde::{Deserialize, Serialize};

/// How serious a notice is. A critical notice may take the operator's view
/// to where it points; the others wait to be opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeSeverity {
    Info,
    Warning,
    Critical,
}

/// Where in the workspace a notice points: a view, and optionally the agent
/// or task in it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeFocus {
    pub view: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,
}

/// One labelled fact behind a notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeDetail {
    pub label: String,
    pub value: String,
}

/// What an action does when the operator chooses it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NoticeActionKind {
    /// Open a place in the workspace; nothing leaves the browser.
    Focus { focus: NoticeFocus },
    /// Tell the agent the operator has seen it; the notice closes.
    Acknowledge,
    /// Submit the task's objective again, as a new task.
    RetryTask { task: String },
}

/// One thing the operator can do about a notice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeAction {
    pub id: String,
    pub label: String,
    #[serde(flatten)]
    pub kind: NoticeActionKind,
}

/// An anomaly an agent brings to the operator.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorNotice {
    pub id: String,
    /// What kind of anomaly this is. Another of the same kind from the same
    /// agent updates this notice instead of raising a second one.
    pub key: String,
    #[serde(rename = "agentId")]
    pub agent_id: String,
    pub severity: NoticeSeverity,
    pub title: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<NoticeFocus>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<NoticeDetail>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<NoticeAction>,
    /// How many times the anomaly occurred while this notice was open.
    pub count: u32,
    /// Raised by one each time the notice changes, so a relay forwards the
    /// change and drops the repeats of an unchanged snapshot.
    pub revision: u32,
    #[serde(rename = "raisedAt")]
    pub raised_at: String,
}

/// An operator's answer to a notice, as the agent receives it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoticeResponse {
    #[serde(rename = "noticeId")]
    pub notice_id: String,
    #[serde(rename = "actionId")]
    pub action_id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_reads_as_the_workspace_expects() {
        let notice = OperatorNotice {
            id: "n1".into(),
            key: "image-generation-failed".into(),
            agent_id: "artist".into(),
            severity: NoticeSeverity::Warning,
            title: "Image generation failed".into(),
            text: "The GPU ran out of memory.".into(),
            focus: Some(NoticeFocus {
                view: "agents".into(),
                agent: Some("artist".into()),
                task: None,
            }),
            details: vec![NoticeDetail {
                label: "Code".into(),
                value: "generation_failed".into(),
            }],
            actions: vec![
                NoticeAction {
                    id: "ack".into(),
                    label: "Acknowledge".into(),
                    kind: NoticeActionKind::Acknowledge,
                },
                NoticeAction {
                    id: "retry".into(),
                    label: "Retry task".into(),
                    kind: NoticeActionKind::RetryTask { task: "t1".into() },
                },
            ],
            count: 1,
            revision: 1,
            raised_at: "2026-10-18T00:00:00Z".into(),
        };
        let json = serde_json::to_value(&notice).unwrap();
        assert_eq!(json["agentId"], "artist");
        assert_eq!(json["severity"], "warning");
        assert_eq!(json["focus"]["view"], "agents");
        assert_eq!(json["actions"][0]["kind"], "acknowledge");
        assert_eq!(json["actions"][1]["kind"], "retry_task");
        assert_eq!(json["actions"][1]["task"], "t1");
        let back: OperatorNotice = serde_json::from_value(json).unwrap();
        assert_eq!(back, notice);
    }
}
