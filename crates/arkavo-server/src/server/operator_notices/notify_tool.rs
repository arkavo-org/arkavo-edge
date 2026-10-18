//! notify_operator: the model's way to bring something to the operator.

use arkavo_mcp_tools::server::{Tool, error_response};
use arkavo_mcp_tools::{Result, ToolRegistry, ToolSchema};
use arkavo_protocol::operator_notice::{NoticeFocus, NoticeSeverity};
use async_trait::async_trait;
use serde_json::{Value, json};

use super::{Anomaly, NOTIFY_OPERATOR, OperatorNotices};

/// The workspace views a notice may point to.
const VIEWS: [&str; 8] = [
    "overview",
    "tasks",
    "agents",
    "decisions",
    "security",
    "telemetry",
    "health",
    "budget",
];

/// The workspace view `name` means, by its id or the label the workspace
/// shows for it, in any case.
fn workspace_view(name: &str) -> Option<&'static str> {
    let name = name.trim().to_lowercase();
    VIEWS
        .iter()
        .copied()
        .find(|view| *view == name)
        .or(match name.as_str() {
            "data & trust" | "data and trust" | "trust" | "data" => Some("security"),
            "evidence" | "events" => Some("telemetry"),
            "swarm" | "agent" => Some("agents"),
            "missions" | "task" => Some("tasks"),
            "runtime" => Some("health"),
            "resources" => Some("budget"),
            _ => None,
        })
}

/// Register notify_operator, raising notices on `hub`.
pub fn register(registry: &mut ToolRegistry, hub: &'static OperatorNotices) {
    registry.register(NOTIFY_OPERATOR, Box::new(NotifyOperator::new(hub)));
}

struct NotifyOperator {
    hub: &'static OperatorNotices,
    schema: ToolSchema,
}

impl NotifyOperator {
    fn new(hub: &'static OperatorNotices) -> Self {
        Self {
            hub,
            schema: ToolSchema {
                name: NOTIFY_OPERATOR.into(),
                aliases: None,
                description: "Bring something to the human operator watching the Arkavo \
                    Operations workspace: a failure you cannot fix, a security or policy concern, \
                    or work that is blocked. Not for routine progress or results; the requester \
                    gets those in your reply."
                    .into(),
                parameters: json!({
                    "type": "object",
                    "properties": {
                        "severity": {"type": "string", "enum": ["info", "warning", "critical"]},
                        "title": {"type": "string", "description": "A few words, at most 80 characters"},
                        "text": {"type": "string", "description": "What happened and what it affects, at most 500 characters"},
                        "view": {"type": "string", "enum": VIEWS, "description": "The workspace view to look at"}
                    },
                    "required": ["severity", "title", "text"]
                }),
            },
        }
    }
}

#[async_trait]
impl Tool for NotifyOperator {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, params: Value) -> Result<Value> {
        // Small models name and spell these loosely; what they mean is clear
        // enough to act on, and a refused notice is one the operator never
        // sees.
        let severity = match params
            .get("severity")
            .and_then(Value::as_str)
            .map(str::to_lowercase)
            .as_deref()
        {
            Some("info") => NoticeSeverity::Info,
            Some("critical" | "high" | "urgent") => NoticeSeverity::Critical,
            _ => NoticeSeverity::Warning,
        };
        let first_of = |fields: &[&str], limit: usize| {
            fields.iter().find_map(|field| {
                params
                    .get(*field)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|text| !text.is_empty())
                    .map(|text| text.chars().take(limit).collect::<String>())
            })
        };
        let title = first_of(&["title", "summary", "subject"], 80);
        let text = first_of(&["text", "message", "description", "details", "body"], 500);
        let (title, text) = match (title, text) {
            (Some(title), Some(text)) => (title, text),
            (Some(title), None) => (title.clone(), title),
            (None, Some(text)) => (text.chars().take(80).collect(), text),
            (None, None) => {
                tracing::warn!(?params, "notify_operator called without a title or text");
                return Ok(error_response("give the notice a title and text"));
            }
        };
        // A notice that names no view the workspace has points at the agent
        // that raised it, so a critical one still takes the operator
        // somewhere useful.
        let view = params
            .get("view")
            .and_then(Value::as_str)
            .and_then(workspace_view)
            .unwrap_or("agents");
        let focus = Some(NoticeFocus {
            view: view.into(),
            agent: None,
            task: None,
        });
        // The same title again is the same concern: it updates its notice.
        let key = format!("model:{}", title.to_lowercase());
        self.hub.raise(Anomaly {
            key,
            severity,
            title,
            text,
            focus,
            details: Vec::new(),
        });
        Ok(json!({ "success": true, "status": "the operator has been notified" }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SRV-016: the model raises a notice with its own words, bounded, and
    /// a view outside the workspace is dropped rather than followed.
    #[arkavo_test_macros::spec("SRV-016")]
    #[tokio::test]
    #[allow(clippy::disallowed_methods)]
    async fn the_model_raises_a_bounded_notice() {
        let hub: &'static OperatorNotices = Box::leak(Box::default());
        let tool = NotifyOperator::new(hub);

        let raised = tool
            .execute(json!({
                "severity": "Critical", "title": "x".repeat(200), "text": "Policy breach", "view": "elsewhere"
            }))
            .await
            .unwrap();
        let loose = tool
            .execute(json!({"message": "The build server is unreachable."}))
            .await
            .unwrap();
        let refused = tool.execute(json!({"severity": "info"})).await.unwrap();

        assert_eq!(raised["success"], true);
        assert_eq!(loose["success"], true);
        assert_eq!(refused["success"], false);
        let open = hub.open();
        assert_eq!(open.len(), 2);
        assert_eq!(open[0].title.chars().count(), 80);
        assert_eq!(open[0].severity, NoticeSeverity::Critical);
        assert_eq!(
            open[0].focus.as_ref().map(|f| f.view.as_str()),
            Some("agents"),
            "a view outside the workspace points at the agent instead"
        );
        assert_eq!(
            open[1].severity,
            NoticeSeverity::Warning,
            "an unnamed severity is a warning"
        );
        assert_eq!(open[1].title, "The build server is unreachable.");
        assert_eq!(workspace_view("Data & Trust"), Some("security"));
        assert_eq!(workspace_view("Security"), Some("security"));
        assert_eq!(workspace_view("Evidence"), Some("telemetry"));
    }
}
