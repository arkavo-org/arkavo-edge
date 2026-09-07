use std::sync::Arc;

use arkavo_mcp::ToolSchema;
use arkavo_mcp_tools::{ToolError, ToolRegistry, server::Tool};
use async_trait::async_trait;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::CodexWorker;

struct WorkerTool {
    worker: Arc<CodexWorker>,
    schema: ToolSchema,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RunArgs {
    prompt: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyArgs {}

#[async_trait]
impl Tool for WorkerTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, args: Value) -> arkavo_mcp_tools::Result<Value> {
        let result = match self.schema.name.as_str() {
            "codex_run" => {
                let args: RunArgs = serde_json::from_value(args).map_err(ToolError::Json)?;
                self.worker
                    .run(&args.prompt)
                    .await
                    .and_then(|outcome| Ok(serde_json::to_value(outcome)?))
            }
            "codex_cancel" => {
                let _: EmptyArgs = serde_json::from_value(args).map_err(ToolError::Json)?;
                self.worker.cancel();
                Ok(json!({"cancellation_requested": true}))
            }
            _ => {
                let _: EmptyArgs = serde_json::from_value(args).map_err(ToolError::Json)?;
                self.worker
                    .session()
                    .and_then(|session| Ok(serde_json::to_value(session)?))
            }
        };
        result.map_err(|e| ToolError::Execution(e.to_string()))
    }
}

/// Register only after the host has authorized the worker's whole sandbox.
/// Codex shell/file operations do not pass through Edge's per-tool policy hooks.
pub fn register_tools(registry: &mut ToolRegistry, worker: Arc<CodexWorker>) {
    for (name, description, parameters) in [
        (
            "codex_run",
            "Delegate a coding task to the authorized Codex worker; continue its saved session. Returns completion status, file changes and usage.",
            json!({
                "type":"object", "properties":{"prompt":{"type":"string","minLength":1,"maxLength":1_048_576}},
                "required":["prompt"], "additionalProperties":false
            }),
        ),
        (
            "codex_status",
            "Get the worker session binding and accounting recovery status.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
        ),
        (
            "codex_cancel",
            "Cancel the active Codex worker attempt.",
            json!({"type":"object","properties":{},"additionalProperties":false}),
        ),
    ] {
        registry.register(
            name,
            Box::new(WorkerTool {
                worker: worker.clone(),
                schema: ToolSchema {
                    name: name.into(),
                    aliases: None,
                    description: description.into(),
                    parameters,
                },
            }),
        );
    }
}

#[cfg(test)]
#[allow(
    clippy::disallowed_methods,
    reason = "Tokio test macros create a runtime from a synchronous test entry point"
)]
mod tests {
    use super::*;
    use std::time::Duration;

    use arkavo_budget::{BudgetConfig, BudgetTracker, CloudPolicy, PricingEntry, TokenCost};
    use tempfile::TempDir;

    async fn worker_fixture() -> (TempDir, Arc<CodexWorker>) {
        let root = tempfile::tempdir().expect("temporary root");
        let workspace = root.path().join("workspace");
        std::fs::create_dir(&workspace).expect("workspace");
        let config = crate::CodexConfig {
            executable: std::env::current_exe().expect("test executable"),
            workspace,
            agent_id: "tools-test".into(),
            model: "gpt-6-astra".into(),
            sandbox: crate::Sandbox::ReadOnly,
            timeout: Duration::from_secs(1),
            max_output_bytes: 1024,
        };
        let approval = crate::SpendApproval {
            policy: CloudPolicy::CloudWithinCap,
            user_confirmed: false,
            projected_cost: TokenCost::from_cents(1),
            pricing: PricingEntry {
                model_id: config.model.clone(),
                provider: "openai".into(),
                input_cents_per_mtok: 1000,
                output_cents_per_mtok: 5000,
                cached_input_cents_per_mtok: None,
                cache_write_cents_per_mtok: None,
                context_window: None,
                max_output_tokens: None,
            },
        };
        let budget = Arc::new(
            BudgetTracker::new(BudgetConfig::default())
                .await
                .expect("budget"),
        );
        let worker = CodexWorker::open(config, &root.path().join("session.json"), approval, budget)
            .expect("worker");
        (root, Arc::new(worker))
    }

    #[test]
    fn prompt_cannot_override_permissions_or_session() {
        for field in [
            "sandbox",
            "workspace",
            "thread_id",
            "model",
            "user_confirmed",
            "executable",
        ] {
            let mut args = json!({"prompt":"hello"});
            args[field] = json!("override");
            assert!(serde_json::from_value::<RunArgs>(args).is_err());
        }
    }

    #[tokio::test]
    async fn registration_exposes_exact_schemas_and_worker_operations() {
        let (_root, worker) = worker_fixture().await;
        let mut registry = ToolRegistry::empty();
        register_tools(&mut registry, worker);

        let mut names: Vec<_> = registry
            .list_tools()
            .into_iter()
            .map(|tool| tool.name)
            .collect();
        names.sort();
        assert_eq!(names, ["codex_cancel", "codex_run", "codex_status"]);

        let run = registry.get("codex_run").expect("run tool").schema();
        assert_eq!(run.parameters["additionalProperties"], false);
        assert_eq!(run.parameters["required"], json!(["prompt"]));
        assert_eq!(
            run.parameters["properties"]["prompt"]["maxLength"],
            1_048_576
        );
        assert!(registry.get("codex_status").is_some());
        assert!(registry.get("codex_cancel").is_some());

        let status = registry
            .get("codex_status")
            .expect("status tool")
            .execute(json!({}))
            .await
            .expect("status");
        assert_eq!(status["agent_id"], "tools-test");
        assert_eq!(status["thread_id"], Value::Null);
        assert_eq!(status["accounting_incomplete"], false);

        let cancelled = registry
            .get("codex_cancel")
            .expect("cancel tool")
            .execute(json!({}))
            .await
            .expect("cancel");
        assert_eq!(cancelled, json!({"cancellation_requested": true}));
    }

    #[tokio::test]
    async fn tools_reject_unknown_and_malformed_arguments() {
        let (_root, worker) = worker_fixture().await;
        let mut registry = ToolRegistry::empty();
        register_tools(&mut registry, worker);

        for (name, args) in [
            ("codex_run", json!({"prompt": "hello", "workspace": "/tmp"})),
            ("codex_status", json!({"unexpected": true})),
            ("codex_cancel", json!("not an object")),
        ] {
            let error = registry
                .get(name)
                .expect("registered tool")
                .execute(args)
                .await
                .expect_err("invalid arguments must fail");
            assert!(!error.to_string().is_empty(), "{name} returned empty error");
        }
    }
}
