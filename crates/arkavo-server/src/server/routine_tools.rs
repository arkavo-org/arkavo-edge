//! Pilot adapter: generated routines use the same primitive registry, role
//! grants and session egress guard as ordinary conductor calls.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use arkavo_mcp_tools::{Tool, ToolError, ToolRegistry, ToolSchema};
use arkavo_routines::{Executor, Library, Session};
use async_trait::async_trait;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::egress_guard::EgressGuard;
use super::learning_bus::{LearningBus, LearningEvent};

pub(super) fn init_persistence(bus: &LearningBus, db_path: &std::path::Path) {
    let scope = serde_json::to_vec(&(bus.agent_id(), bus.swarm_id())).unwrap_or_default();
    let path = db_path.with_file_name(format!("routines-{:x}.json", Sha256::digest(scope)));
    let library = match Library::open(&path) {
        Ok(library) => Some(Arc::new(Mutex::new(library))),
        Err(error) => {
            tracing::warn!(%error, "Routine persistence unavailable; pilot disabled for this agent");
            None
        }
    };
    let _ = bus.routine_library.set(library);
}

pub(super) fn with_routines(
    original: Arc<ToolRegistry>,
    bus: &Arc<LearningBus>,
    granted: Option<&HashSet<String>>,
    egress: Arc<EgressGuard>,
) -> Arc<ToolRegistry> {
    // An opt-in build still respects specialized roles' explicit tool grants.
    // Never let the routine meta-tools add authority to a role.
    if granted.is_some_and(|g| {
        !g.contains("routine_run") || !g.contains("routine_learn") || !g.contains("routine_catalog")
    }) || original
        .list_tools()
        .iter()
        .any(|t| t.name.starts_with("routine_"))
    {
        return original;
    }
    let Some(library) = bus
        .routine_library
        .get_or_init(|| Some(Arc::new(Mutex::new(Library::default()))))
    else {
        return original;
    };
    let executor = Arc::new(HostExecutor {
        registry: original.clone(),
        granted: granted.cloned(),
        egress,
        bus: bus.clone(),
    });
    let session = Arc::new(Session::new(library.clone(), executor.clone()));
    // Serialize ordinary calls with complete routine invocations. The parallel
    // conductor must not interleave another action between a check and its use.
    let serial = Arc::new(tokio::sync::Mutex::new(()));
    let mut registry = ToolRegistry::empty();
    for info in original.list_tools() {
        if let Some(tool) = original.get(&info.name) {
            registry.register(
                &info.name,
                Box::new(ObservedTool {
                    schema: tool.schema().clone(),
                    executor: executor.clone(),
                    session: session.clone(),
                    serial: serial.clone(),
                }),
            );
        }
    }
    arkavo_routines::register_tools(
        |name, tool| {
            registry.register(
                name,
                Box::new(RoutineAdapter {
                    tool,
                    serial: serial.clone(),
                }),
            );
        },
        session,
    );
    Arc::new(registry)
}

struct HostExecutor {
    registry: Arc<ToolRegistry>,
    granted: Option<HashSet<String>>,
    egress: Arc<EgressGuard>,
    bus: Arc<LearningBus>,
}

impl HostExecutor {
    async fn dispatch(&self, name: &str, arguments: Value) -> arkavo_mcp_tools::Result<Value> {
        if !self.available(name) {
            return Err(ToolError::PolicyDenied(
                "Routine primitive is unavailable or not granted".into(),
            ));
        }
        self.egress
            .check_call(name, &arguments)
            .map_err(ToolError::PolicyDenied)?;
        let tool = self
            .registry
            .get(name)
            .ok_or_else(|| ToolError::Execution("Tool disappeared".into()))?;
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            tool.execute(arguments.clone()),
        )
        .await
        .map_err(|_| {
            ToolError::Execution(
                "Routine primitive timed out; inspect state before retrying".into(),
            )
        })??;
        self.egress
            .observe_result(name, &arguments, &result.to_string());
        Ok(result)
    }
}

fn successful(result: &Value) -> bool {
    let text = result.to_string();
    super::conductor_tool_loop::detect_semantic_failure(&text).is_none()
        && super::conductor::extract_reward_from_result(&text).is_none_or(|r| r >= 0.0)
        && result.get("isError") != Some(&Value::Bool(true))
        && result.get("error").is_none_or(Value::is_null)
}

#[async_trait]
impl Executor for HostExecutor {
    fn available(&self, name: &str) -> bool {
        !name.starts_with("routine_")
            && self.registry.get(name).is_some()
            && super::conductor_tool_loop::tool_call_permitted(name, self.granted.as_ref())
    }

    async fn execute(&self, name: &str, arguments: Value) -> arkavo_routines::Result<Value> {
        let start = Instant::now();
        let result = self.dispatch(name, arguments.clone()).await;
        let success = result.as_ref().is_ok_and(successful);
        let latency_ms = start.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        arkavo_observability::subsystem_timing::global_timing()
            .mcp_tools
            .record(latency_ms);
        if let Err(error) = self
            .bus
            .sender()
            .send(LearningEvent::ToolCall {
                tool_name: name.into(),
                args: arkavo_router::learning::sanitize_args(&arguments),
                result: result
                    .as_ref()
                    .map(Value::to_string)
                    .unwrap_or_else(|e| e.to_string()),
                success,
                latency_ms,
                decision_trace_id: None,
                step_index: 0,
                model_name: None,
            })
            .await
        {
            tracing::warn!(%error, "Routine primitive learning event was not delivered");
        }
        tracing::info!(
            tool = name,
            success,
            latency_ms,
            "Routine primitive completed"
        );
        match result {
            Ok(value) if success => Ok(value),
            _ => Err("Primitive failed or was denied".into()),
        }
    }
}

struct ObservedTool {
    schema: ToolSchema,
    executor: Arc<HostExecutor>,
    session: Arc<Session>,
    serial: Arc<tokio::sync::Mutex<()>>,
}

#[async_trait]
impl Tool for ObservedTool {
    fn schema(&self) -> &ToolSchema {
        &self.schema
    }

    async fn execute(&self, arguments: Value) -> arkavo_mcp_tools::Result<Value> {
        let _guard = self.serial.lock().await;
        let start = Instant::now();
        let result = self
            .executor
            .dispatch(&self.schema.name, arguments.clone())
            .await;
        if let Ok(value) = &result
            && successful(value)
        {
            self.session
                .observe(&self.schema.name, arguments, value.clone(), start)
                .map_err(ToolError::Execution)?;
        } else {
            self.session
                .invalidate_evidence()
                .map_err(ToolError::Execution)?;
        }
        result
    }
}

struct RoutineAdapter {
    tool: Box<dyn arkavo_mcp::Tool>,
    serial: Arc<tokio::sync::Mutex<()>>,
}

#[async_trait]
impl Tool for RoutineAdapter {
    fn schema(&self) -> &ToolSchema {
        self.tool.schema()
    }

    async fn execute(&self, arguments: Value) -> arkavo_mcp_tools::Result<Value> {
        let _guard = self.serial.lock().await;
        self.tool
            .execute(arguments)
            .await
            .map_err(|e| ToolError::Execution(e.to_string()))
    }
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // Tokio's test macro starts a runtime outside async code.
mod tests {
    use super::*;
    use serde_json::json;

    fn bus() -> Arc<LearningBus> {
        Arc::new(LearningBus::new(
            "routine-test".into(),
            "isolated".into(),
            Arc::new(arkavo_crypto::AgentKeypair::generate()),
            arkavo_gossip::GossipConfig::default(),
        ))
    }

    fn registry() -> Arc<ToolRegistry> {
        let mut registry = ToolRegistry::empty();
        registry.register(
            "filesystem_tools",
            Box::new(arkavo_mcp_tools::filesystem::FileSystemKit::new()),
        );
        Arc::new(registry)
    }

    fn guard() -> Arc<EgressGuard> {
        Arc::new(
            EgressGuard::new("routine-task", "routine-test").with_destination_policy(
                arkavo_protocol::egress_destination::DestinationPolicy::new()
                    .workspace_root(std::env::current_dir().unwrap()),
            ),
        )
    }

    #[test]
    fn specialized_roles_must_explicitly_grant_the_pilot_tools() {
        let original = registry();
        let granted = HashSet::from(["filesystem_tools".into()]);
        let wrapped = with_routines(original.clone(), &bus(), Some(&granted), guard());
        assert!(Arc::ptr_eq(&original, &wrapped));
    }

    #[tokio::test]
    async fn registered_tools_learn_and_replay_real_filesystem_operations() {
        let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = dir.path().join("counter.txt");
        std::fs::write(&path, "one").unwrap();
        let bus = bus();
        let steps = json!({"steps":[
            {"tool":"filesystem_tools","arguments":{"action":{"$input":"inspect"},"file_path":{"$input":"path"}},"check":{"pointer":"/success","equals":true}},
            {"tool":"filesystem_tools","arguments":{"action":{"$input":"read"},"file_path":{"$input":"path"}},"check":{"pointer":"/content","equals":{"$input":"expected"}}}
        ]});
        let inputs = json!({"inspect":"file_info","read":"read_file","path":path,"expected":"one"});
        let mut id = Value::Null;
        for _ in 0..2 {
            let tools = with_routines(registry(), &bus, None, guard());
            for action in ["file_info", "read_file"] {
                tools
                    .get("filesystem_tools")
                    .unwrap()
                    .execute(json!({"action":action,"file_path":path}))
                    .await
                    .unwrap();
            }
            let learned = tools
                .get("routine_learn")
                .unwrap()
                .execute(json!({"routine":steps,"inputs":inputs,"observations":[1,2]}))
                .await
                .unwrap();
            id = learned["id"].clone();
        }
        let tools = with_routines(registry(), &bus, None, guard());
        let result = tools
            .get("routine_run")
            .unwrap()
            .execute(json!({"id":id,"inputs":inputs}))
            .await
            .unwrap();
        assert_eq!(result["result"]["content"], "one");
        let catalog = tools
            .get("routine_catalog")
            .unwrap()
            .execute(json!({}))
            .await
            .unwrap();
        assert_eq!(catalog["metrics"]["primitive_calls"], 2);
        assert_eq!(catalog["metrics"]["routine_successes"], 1);
    }

    #[tokio::test]
    async fn nested_dispatch_cannot_bypass_grants_or_egress_policy() {
        let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let path = dir.path().join("must-not-exist.txt");
        let guard = guard();
        let key = format!("sk-{}", "x".repeat(48));
        guard.observe_input("document", &format!("API_TOKEN={key}"));
        let executor = HostExecutor {
            registry: registry(),
            granted: None,
            egress: guard,
            bus: bus(),
        };
        let args = json!({"action":"write_file","file_path":path,"content":"copied text","url":"https://external.example/collect"});
        assert!(
            executor
                .execute("filesystem_tools", args.clone())
                .await
                .is_err()
        );
        assert!(!path.exists());
        let executor = HostExecutor {
            granted: Some(HashSet::new()),
            ..executor
        };
        assert!(executor.execute("filesystem_tools", args).await.is_err());
        assert!(!path.exists());
        assert!(!executor.available("routine_run"));
    }

    #[tokio::test]
    async fn failed_tool_results_do_not_become_evidence() {
        let tools = with_routines(registry(), &bus(), None, guard());
        let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        let result = tools
            .get("filesystem_tools")
            .unwrap()
            .execute(json!({"action":"read_file","file_path":dir.path().join("missing.txt")}))
            .await
            .unwrap();
        assert_eq!(result["success"], false);
        let catalog = tools
            .get("routine_catalog")
            .unwrap()
            .execute(json!({}))
            .await
            .unwrap();
        assert_eq!(catalog["observations"], json!([]));
    }

    #[test]
    fn malformed_persistence_disables_learning_instead_of_forgetting_retirement() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("learning.db");
        let first = bus();
        init_persistence(&first, &db);
        let file = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|ext| ext == "json"))
            .unwrap();
        std::fs::write(file, "broken").unwrap();
        drop(first);
        let other = bus();
        init_persistence(&other, &db);
        let original = registry();
        assert!(Arc::ptr_eq(
            &original,
            &with_routines(original.clone(), &other, None, guard())
        ));
    }
}
