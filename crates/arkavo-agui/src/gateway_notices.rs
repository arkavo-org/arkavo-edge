//! Notices brought to the operator, relayed to every open view (#759).
//!
//! Agents report their open notices in every metrics snapshot until the
//! operator acknowledges them; the relay forwards a notice when it is new or
//! changed and says when one closed, so views see each change once. The
//! gateway raises notices of its own about the tasks it dispatched, since
//! only it knows them by the id the workspace shows and can submit one
//! again.

use std::collections::HashMap;
use std::sync::Arc;

use arkavo_protocol::operator_notice::{
    NoticeAction, NoticeActionKind, NoticeDetail, NoticeFocus, NoticeSeverity, OperatorNotice,
};
use tokio::sync::RwLock;

use crate::agent_connection::AgentConnection;
use crate::gateway::ConnectionInfo;
use crate::gateway_routing::broadcast_event;
use crate::types::AgUiEvent;

/// The most notices the gateway itself keeps open.
const MAX_GATEWAY_NOTICES: usize = 20;

/// The events that turn `open` into `reported`: each new or changed notice,
/// and each one that is no longer reported. `open` becomes `reported`.
pub(crate) fn changes(
    open: &mut Vec<OperatorNotice>,
    reported: Vec<OperatorNotice>,
) -> Vec<AgUiEvent> {
    let mut events = Vec::new();
    for notice in &reported {
        let unchanged = open
            .iter()
            .any(|known| known.id == notice.id && known.revision == notice.revision);
        if !unchanged {
            events.push(AgUiEvent::OperatorNotice {
                notice: notice.clone(),
            });
        }
    }
    for gone in open
        .iter()
        .filter(|known| !reported.iter().any(|n| n.id == known.id))
    {
        events.push(AgUiEvent::OperatorNoticeResolved {
            agent_id: gone.agent_id.clone(),
            notice_id: gone.id.clone(),
        });
    }
    *open = reported;
    events
}

/// The notices the gateway raised.
pub type GatewayNotices = Arc<RwLock<Vec<OperatorNotice>>>;

/// Tell every view that a task the workspace submitted failed, with a way
/// to submit it again.
pub(crate) async fn task_failed(
    notices: &GatewayNotices,
    connections: &Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    task_id: &str,
    agent_id: &str,
    why: &str,
) {
    let notice = OperatorNotice {
        id: uuid::Uuid::new_v4().to_string(),
        key: format!("task-failed:{task_id}"),
        agent_id: agent_id.to_string(),
        severity: NoticeSeverity::Warning,
        title: "Task failed".into(),
        text: why.chars().take(500).collect(),
        focus: Some(task_focus(task_id)),
        details: vec![NoticeDetail {
            label: "Agent".into(),
            value: agent_id.to_string(),
        }],
        actions: vec![
            NoticeAction {
                id: "inspect".into(),
                label: "Open task".into(),
                kind: NoticeActionKind::Focus {
                    focus: task_focus(task_id),
                },
            },
            NoticeAction {
                id: "retry".into(),
                label: "Retry task".into(),
                kind: NoticeActionKind::RetryTask {
                    task: task_id.to_string(),
                },
            },
            NoticeAction {
                id: "acknowledge".into(),
                label: "Dismiss".into(),
                kind: NoticeActionKind::Acknowledge,
            },
        ],
        count: 1,
        revision: 1,
        raised_at: chrono::Utc::now().to_rfc3339(),
    };
    {
        let mut open = notices.write().await;
        open.retain(|n| n.key != notice.key);
        open.push(notice.clone());
        let excess = open.len().saturating_sub(MAX_GATEWAY_NOTICES);
        open.drain(..excess);
    }
    tracing::info!(task_id, agent_id, "gateway notice raised: task failed");
    broadcast_event(&AgUiEvent::OperatorNotice { notice }, connections).await;
}

fn task_focus(task_id: &str) -> NoticeFocus {
    NoticeFocus {
        view: "tasks".into(),
        agent: None,
        task: Some(task_id.to_string()),
    }
}

/// Every notice open now: the gateway's and each connected agent's.
pub(crate) async fn all_open(
    notices: &GatewayNotices,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
) -> Vec<OperatorNotice> {
    let mut all = notices.read().await.clone();
    let connections: Vec<_> = agent_connections.read().await.values().cloned().collect();
    for connection in connections {
        all.extend(connection.open_notices().await);
    }
    all
}

/// What a chosen action needs from the gateway.
pub(crate) enum Chosen {
    /// The gateway's own notice, answered here.
    Gateway(Box<OperatorNotice>, NoticeAction),
    /// An agent's notice, answered by that agent.
    Agent(Arc<AgentConnection>, NoticeAction),
}

/// Find the notice and action the operator chose.
pub(crate) async fn chosen(
    notices: &GatewayNotices,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    agent_id: &str,
    notice_id: &str,
    action_id: &str,
) -> Result<Chosen, String> {
    let action_of = |notice: &OperatorNotice| {
        notice
            .actions
            .iter()
            .find(|a| a.id == action_id)
            .cloned()
            .ok_or_else(|| format!("the notice offers no action {action_id}"))
    };
    if let Some(notice) = notices.read().await.iter().find(|n| n.id == notice_id) {
        return Ok(Chosen::Gateway(
            Box::new(notice.clone()),
            action_of(notice)?,
        ));
    }
    let connection = agent_connections
        .read()
        .await
        .get(agent_id)
        .cloned()
        .ok_or_else(|| format!("agent {agent_id} is not connected"))?;
    let notice = connection
        .open_notices()
        .await
        .into_iter()
        .find(|n| n.id == notice_id)
        .ok_or_else(|| "the notice is no longer open".to_string())?;
    let action = action_of(&notice)?;
    Ok(Chosen::Agent(connection, action))
}

/// Close one of the gateway's notices and tell every view.
pub(crate) async fn close(
    notices: &GatewayNotices,
    connections: &Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    notice: &OperatorNotice,
) {
    notices.write().await.retain(|n| n.id != notice.id);
    broadcast_event(
        &AgUiEvent::OperatorNoticeResolved {
            agent_id: notice.agent_id.clone(),
            notice_id: notice.id.clone(),
        },
        connections,
    )
    .await;
}

/// What the gateway does after the operator chose an action.
#[derive(Debug)]
pub(crate) enum Acted {
    /// The notice closed.
    Closed,
    /// Submit this objective again as a new task; the notice closed.
    Retry(String),
    /// Nothing beyond the browser: it opened where the action points.
    Opened,
}

/// Carry out the action the operator chose on a notice. Acknowledging an
/// agent's notice goes to that agent; the gateway's own notices are
/// answered here.
pub(crate) async fn act(
    notices: &GatewayNotices,
    agent_connections: &Arc<RwLock<HashMap<String, Arc<AgentConnection>>>>,
    task_store: &Arc<RwLock<HashMap<String, crate::gateway::TrackedTask>>>,
    connections: &Arc<RwLock<HashMap<String, ConnectionInfo>>>,
    agent_id: &str,
    notice_id: &str,
    action_id: &str,
) -> Result<Acted, String> {
    match chosen(notices, agent_connections, agent_id, notice_id, action_id).await? {
        Chosen::Gateway(notice, action) => match action.kind {
            NoticeActionKind::Focus { .. } => Ok(Acted::Opened),
            NoticeActionKind::Acknowledge => {
                close(notices, connections, &notice).await;
                Ok(Acted::Closed)
            }
            NoticeActionKind::RetryTask { task } => {
                let objective = task_store
                    .read()
                    .await
                    .get(&task)
                    .map(|t| t.description.clone())
                    .ok_or_else(|| format!("task {task} is no longer tracked"))?;
                close(notices, connections, &notice).await;
                Ok(Acted::Retry(objective))
            }
        },
        Chosen::Agent(connection, action) => match action.kind {
            NoticeActionKind::Acknowledge => {
                connection
                    .send_request(
                        "operator.notices.respond",
                        serde_json::json!({ "noticeId": notice_id, "actionId": action_id }),
                        "operator-notice",
                    )
                    .await
                    .map_err(|e| format!("the agent did not take the answer: {e}"))?;
                connection.forget_notice(notice_id).await;
                broadcast_event(
                    &AgUiEvent::OperatorNoticeResolved {
                        agent_id: agent_id.to_string(),
                        notice_id: notice_id.to_string(),
                    },
                    connections,
                )
                .await;
                Ok(Acted::Closed)
            }
            NoticeActionKind::Focus { .. } | NoticeActionKind::RetryTask { .. } => {
                Ok(Acted::Opened)
            }
        },
    }
}

#[cfg(test)]
mod tests;
