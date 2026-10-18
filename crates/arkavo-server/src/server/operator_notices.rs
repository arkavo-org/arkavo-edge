//! The notices this agent has open for the operator (#759).
//!
//! Anomalies are raised here from wherever the agent meets them: image work
//! that failed, a spent compute budget, data stopped at the egress gate, or
//! the model calling `notify_operator`. Open notices go out in every metrics
//! snapshot (`system.metrics.subscribe`) until the operator acknowledges one
//! with `operator.notices.respond`, so a gateway that reconnects, or a view
//! that opens later, still sees them.
//!
//! The hub is one per process, as an agent is: the places that raise
//! notices are spread across the loop, the tools and the asset service, and
//! threading a handle through each of them would buy nothing a test cannot
//! get from its own [`OperatorNotices`].

use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use arkavo_protocol::operator_notice::{
    NoticeAction, NoticeActionKind, NoticeDetail, NoticeFocus, NoticeResponse, NoticeSeverity,
    OperatorNotice,
};

/// The tool the model raises notices with.
///
/// Every role may call it: telling the operator something is wrong is never
/// a privilege a role's grants withhold, and it sends nothing beyond the
/// operator's own gateway.
pub const NOTIFY_OPERATOR: &str = "notify_operator";

/// The most notices kept open at once; the oldest closes first.
const MAX_OPEN: usize = 20;

/// A notice nobody answered closes after this long.
const EXPIRY: Duration = Duration::from_mins(30);

/// What a place in the agent knows about an anomaly.
#[derive(Debug, Clone)]
pub struct Anomaly {
    pub key: String,
    pub severity: NoticeSeverity,
    pub title: String,
    pub text: String,
    pub focus: Option<NoticeFocus>,
    pub details: Vec<NoticeDetail>,
}

struct Open {
    notice: OperatorNotice,
    raised: Instant,
}

/// Open notices, newest last.
#[derive(Default)]
pub struct OperatorNotices {
    agent_id: OnceLock<String>,
    open: Mutex<VecDeque<Open>>,
}

impl OperatorNotices {
    /// The process's hub.
    pub fn global() -> &'static Self {
        static HUB: OnceLock<OperatorNotices> = OnceLock::new();
        HUB.get_or_init(Self::default)
    }

    /// Name the agent the notices come from; the first name given stays.
    pub fn identify(&self, agent_id: &str) {
        let _ = self.agent_id.set(agent_id.to_string());
    }

    /// Bring `anomaly` to the operator. Another of the same kind while one
    /// is open updates that notice, counting the repeat and keeping the
    /// higher severity, rather than raising a second.
    pub fn raise(&self, anomaly: Anomaly) {
        let agent_id = self
            .agent_id
            .get()
            .cloned()
            .unwrap_or_else(|| "agent".into());
        let mut open = lock(&self.open);
        expire(&mut open);
        if let Some(existing) = open.iter_mut().find(|o| o.notice.key == anomaly.key) {
            let notice = &mut existing.notice;
            notice.count += 1;
            notice.revision += 1;
            notice.severity = notice.severity.max(anomaly.severity);
            notice.title = anomaly.title;
            notice.text = anomaly.text;
            notice.details = anomaly.details;
            notice.focus = anomaly.focus.or_else(|| notice.focus.take());
            notice.raised_at = chrono::Utc::now().to_rfc3339();
            existing.raised = Instant::now();
            tracing::info!(key = %anomaly.key, count = notice.count, "operator notice repeated");
            return;
        }
        let notice = notice_for(&agent_id, anomaly);
        tracing::info!(key = %notice.key, severity = ?notice.severity, title = %notice.title, "operator notice raised");
        open.push_back(Open {
            notice,
            raised: Instant::now(),
        });
        while open.len() > MAX_OPEN {
            open.pop_front();
        }
    }

    /// The notices open now, for the metrics snapshot.
    pub fn open(&self) -> Vec<OperatorNotice> {
        let mut open = lock(&self.open);
        expire(&mut open);
        open.iter().map(|o| o.notice.clone()).collect()
    }

    /// Record the operator's answer. Acknowledging closes the notice; an
    /// action the notice does not offer is refused.
    pub fn respond(&self, response: &NoticeResponse) -> Result<(), String> {
        let mut open = lock(&self.open);
        let index = open
            .iter()
            .position(|o| o.notice.id == response.notice_id)
            .ok_or_else(|| format!("no open notice {}", response.notice_id))?;
        let action = open[index]
            .notice
            .actions
            .iter()
            .find(|a| a.id == response.action_id)
            .ok_or_else(|| {
                format!(
                    "notice {} offers no action {}",
                    response.notice_id, response.action_id
                )
            })?;
        tracing::info!(
            notice = %response.notice_id,
            action = %response.action_id,
            key = %open[index].notice.key,
            "operator answered a notice"
        );
        if action.kind == NoticeActionKind::Acknowledge {
            open.remove(index);
        }
        Ok(())
    }
}

fn notice_for(agent_id: &str, mut anomaly: Anomaly) -> OperatorNotice {
    // A notice about this agent points at it, so the operator lands on the
    // agent that raised it rather than on the whole swarm.
    if let Some(focus) = anomaly.focus.as_mut()
        && focus.view == "agents"
        && focus.agent.is_none()
    {
        focus.agent = Some(agent_id.to_string());
    }
    let mut actions = Vec::new();
    if let Some(focus) = &anomaly.focus {
        actions.push(NoticeAction {
            id: "inspect".into(),
            label: inspect_label(&focus.view).into(),
            kind: NoticeActionKind::Focus {
                focus: focus.clone(),
            },
        });
    }
    actions.push(NoticeAction {
        id: "acknowledge".into(),
        label: "Acknowledge".into(),
        kind: NoticeActionKind::Acknowledge,
    });
    OperatorNotice {
        id: uuid::Uuid::new_v4().to_string(),
        key: anomaly.key,
        agent_id: agent_id.to_string(),
        severity: anomaly.severity,
        title: anomaly.title,
        text: anomaly.text,
        focus: anomaly.focus,
        details: anomaly.details,
        actions,
        count: 1,
        revision: 1,
        raised_at: chrono::Utc::now().to_rfc3339(),
    }
}

fn inspect_label(view: &str) -> &'static str {
    match view {
        "agents" => "Inspect agent",
        "tasks" => "Open task",
        "budget" => "Open resources",
        "security" => "Open Data & Trust",
        _ => "Open",
    }
}

fn expire(open: &mut VecDeque<Open>) {
    open.retain(|o| o.raised.elapsed() < EXPIRY);
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// One labelled detail.
pub fn detail(label: &str, value: impl Into<String>) -> NoticeDetail {
    NoticeDetail {
        label: label.to_string(),
        value: value.into(),
    }
}

mod notify_tool;
pub use notify_tool::register;

#[cfg(test)]
mod tests;
