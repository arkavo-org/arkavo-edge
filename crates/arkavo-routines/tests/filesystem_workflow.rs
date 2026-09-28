#![allow(clippy::disallowed_methods)] // Tokio's test macro owns its runtime outside async code.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use arkavo_routines::{Check, Executor, Library, Routine, Session, Step};
use async_trait::async_trait;
use serde_json::{Value, json};

// These tools perform actual file IO in an isolated directory. Policy changes
// can therefore prove that a denied later step has no filesystem side effects.
struct Files {
    dir: tempfile::TempDir,
    grants: Mutex<HashSet<String>>,
    calls: Mutex<Vec<String>>,
    revoke_after_read: Mutex<bool>,
    stall_after_read: std::sync::atomic::AtomicBool,
}

impl Files {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            dir: tempfile::tempdir().unwrap(),
            grants: Mutex::new(HashSet::from(["read".into(), "write".into()])),
            calls: Mutex::new(Vec::new()),
            revoke_after_read: Mutex::new(false),
            stall_after_read: std::sync::atomic::AtomicBool::new(false),
        })
    }

    fn seed(&self, path: &str, value: u64) {
        std::fs::write(self.dir.path().join(path), value.to_string()).unwrap();
    }
}

#[async_trait]
impl Executor for Files {
    fn available(&self, tool: &str) -> bool {
        self.grants.lock().unwrap().contains(tool)
    }

    async fn execute(&self, tool: &str, args: Value) -> arkavo_routines::Result<Value> {
        if !self.available(tool) {
            return Err("grant revoked".into());
        }
        let name = args["path"].as_str().ok_or("missing path")?;
        if name.contains('/') || name.contains('\\') {
            return Err("outside scope".into());
        }
        let path = self.dir.path().join(name);
        self.calls.lock().unwrap().push(tool.into());
        match tool {
            "read" => {
                let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
                let value: u64 = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
                if *self.revoke_after_read.lock().unwrap() {
                    self.grants.lock().unwrap().remove("write");
                }
                if self
                    .stall_after_read
                    .load(std::sync::atomic::Ordering::Relaxed)
                {
                    // Inject a lost response after real file IO, so dropping
                    // the replay future exercises cancellation of in-flight work.
                    std::future::pending::<()>().await;
                }
                Ok(json!({"value":value}))
            }
            "write" => {
                std::fs::write(path, args["value"].to_string()).map_err(|e| e.to_string())?;
                Ok(json!({"written":true}))
            }
            _ => Err("unknown tool".into()),
        }
    }
}

fn routine() -> Routine {
    Routine {
        steps: vec![
            Step {
                tool: "read".into(),
                arguments: json!({"path":{"$input":"path"}}),
                check: Check {
                    pointer: "/value".into(),
                    equals: json!({"$input":"before"}),
                },
            },
            Step {
                tool: "write".into(),
                arguments: json!({"path":{"$input":"path"},"value":{"$input":"after"}}),
                check: Check {
                    pointer: "/written".into(),
                    equals: json!(true),
                },
            },
            Step {
                tool: "read".into(),
                arguments: json!({"path":{"$input":"path"}}),
                check: Check {
                    pointer: "/value".into(),
                    equals: json!({"$input":"after"}),
                },
            },
        ],
    }
}

async fn demonstrate(session: &Session, files: &Files, path: &str) {
    for (tool, args) in [
        ("read", json!({"path":path})),
        ("write", json!({"path":path,"value":1})),
        ("read", json!({"path":path})),
    ] {
        let start = Instant::now();
        let result = files.execute(tool, args.clone()).await.unwrap();
        session.observe(tool, args, result, start).unwrap();
    }
}

async fn trained() -> (Arc<Mutex<Library>>, Arc<Files>, String) {
    let library = Arc::new(Mutex::new(Library::default()));
    let files = Files::new();
    let mut id = String::new();
    for name in ["first.json", "second.json"] {
        files.seed(name, 0);
        let session = Session::new(library.clone(), files.clone());
        demonstrate(&session, &files, name).await;
        id = session
            .learn(
                routine(),
                &json!({"path":name,"before":0,"after":1}),
                &[1, 2, 3],
            )
            .unwrap();
    }
    (library, files, id)
}

#[tokio::test]
async fn learns_only_from_two_tasks_then_replays_fresh_inputs() {
    let library = Arc::new(Mutex::new(Library::default()));
    let files = Files::new();
    files.seed("first.json", 0);
    let session = Session::new(library.clone(), files.clone());
    let inputs = json!({"path":"first.json","before":0,"after":1});
    assert!(session.learn(routine(), &inputs, &[1, 2, 3]).is_err());
    demonstrate(&session, &files, "first.json").await;
    let id = session.learn(routine(), &inputs, &[1, 2, 3]).unwrap();
    session.learn(routine(), &inputs, &[1, 2, 3]).unwrap();
    assert!(!library.lock().unwrap().catalog()[&id].active());
    assert!(session.run(&id, &inputs).await.is_err());
    let wrong = json!({"path":"different.json","before":0,"after":1});
    assert!(session.learn(routine(), &wrong, &[1, 2, 3]).is_err());
    assert!(session.learn(routine(), &inputs, &[1, 3, 2]).is_err());

    let other = Session::new(library.clone(), files.clone());
    files.seed("second.json", 0);
    demonstrate(&other, &files, "second.json").await;
    assert_eq!(
        id,
        other
            .learn(
                routine(),
                &json!({"path":"second.json","before":0,"after":1}),
                &[1, 2, 3]
            )
            .unwrap()
    );
    files.seed("new.json", 41);
    let run = Session::new(library.clone(), files.clone());
    let result = run
        .run(&id, &json!({"path":"new.json","before":41,"after":42}))
        .await
        .unwrap();
    assert_eq!(result["result"]["value"], 42);
    assert_eq!(
        std::fs::read_to_string(files.dir.path().join("new.json")).unwrap(),
        "42"
    );
    let catalog = run.catalog().unwrap();
    assert_eq!(catalog["metrics"]["primitive_calls"], 3);
    assert_eq!(catalog["metrics"]["routine_successes"], 1);
    let snapshot = library.lock().unwrap().snapshot().unwrap();
    assert!(!String::from_utf8_lossy(&snapshot).contains("first.json"));
    assert!(!String::from_utf8_lossy(&snapshot).contains("new.json"));
    assert!(Library::restore(&snapshot).unwrap().catalog()[&id].active());
}

#[tokio::test]
async fn stale_precondition_stops_before_write_and_retirement_survives_restart() {
    let (library, files, id) = trained().await;
    files.seed("drift.json", 99);
    let run = Session::new(library.clone(), files.clone());
    for _ in 0..2 {
        assert!(
            run.run(&id, &json!({"path":"drift.json","before":0,"after":1}))
                .await
                .is_err()
        );
    }
    assert_eq!(
        std::fs::read_to_string(files.dir.path().join("drift.json")).unwrap(),
        "99"
    );
    assert!(library.lock().unwrap().catalog()[&id].retired);
    let restored = Library::restore(&library.lock().unwrap().snapshot().unwrap()).unwrap();
    let next = Session::new(Arc::new(Mutex::new(restored)), files.clone());
    files.seed("again.json", 0);
    demonstrate(&next, &files, "again.json").await;
    assert!(
        next.learn(
            routine(),
            &json!({"path":"again.json","before":0,"after":1}),
            &[1, 2, 3]
        )
        .is_err()
    );
    let renamed: Routine = serde_json::from_value(
        serde_json::from_str(
            &serde_json::to_string(&routine())
                .unwrap()
                .replace("\"path\"}", "\"location\"}"),
        )
        .unwrap(),
    )
    .unwrap();
    assert!(
        next.learn(
            renamed,
            &json!({"location":"again.json","before":0,"after":1}),
            &[1, 2, 3]
        )
        .is_err()
    );
}

#[tokio::test]
async fn revoked_grant_is_checked_before_each_step() {
    let (library, files, id) = trained().await;
    files.seed("protected.json", 0);
    *files.revoke_after_read.lock().unwrap() = true;
    let run = Session::new(library, files.clone());
    let inputs = json!({"path":"protected.json","before":0,"after":1});
    assert!(run.run(&id, &inputs).await.is_err());
    assert_eq!(
        std::fs::read_to_string(files.dir.path().join("protected.json")).unwrap(),
        "0"
    );
    let count = files.calls.lock().unwrap().len();
    assert!(run.run(&id, &inputs).await.is_err());
    assert_eq!(files.calls.lock().unwrap().len(), count);
    assert_eq!(run.catalog().unwrap()["routines"], json!([]));
}

#[tokio::test]
async fn missing_input_is_rejected_before_any_io() {
    let (library, files, id) = trained().await;
    let run = Session::new(library, files.clone());
    let count = files.calls.lock().unwrap().len();
    assert!(
        run.run(&id, &json!({"path":"missing.json","before":0}))
            .await
            .is_err()
    );
    assert_eq!(files.calls.lock().unwrap().len(), count);
}

#[tokio::test]
async fn failed_and_overlapping_demonstrations_are_not_admitted() {
    let library = Arc::new(Mutex::new(Library::default()));
    let files = Files::new();
    files.seed("first.json", 0);
    let session = Session::new(library, files.clone());
    demonstrate(&session, &files, "first.json").await;
    session.invalidate_evidence().unwrap();
    assert!(
        session
            .learn(
                routine(),
                &json!({"path":"first.json","before":0,"after":1}),
                &[1, 2, 3]
            )
            .is_err()
    );
    let start = Instant::now();
    session
        .observe(
            "read",
            json!({"path":"first.json"}),
            json!({"value":0}),
            start,
        )
        .unwrap();
    session
        .observe(
            "write",
            json!({"path":"first.json","value":1}),
            json!({"written":true}),
            start,
        )
        .unwrap();
    session
        .observe(
            "read",
            json!({"path":"first.json"}),
            json!({"value":1}),
            start,
        )
        .unwrap();
    assert!(
        session
            .learn(
                routine(),
                &json!({"path":"first.json","before":0,"after":1}),
                &[4, 5, 6]
            )
            .is_err()
    );
}

#[tokio::test]
async fn exported_mcp_tools_validate_learn_list_and_run_requests() {
    let (library, files, id) = trained().await;
    files.seed("tool.json", 0);
    let session = Arc::new(Session::new(library, files.clone()));
    demonstrate(&session, &files, "tool.json").await;
    let mut tools = std::collections::HashMap::new();
    arkavo_routines::register_tools(
        |name, tool| {
            tools.insert(name.to_string(), tool);
        },
        session,
    );
    assert_eq!(tools.len(), 3);
    for (name, tool) in &tools {
        assert_eq!(tool.schema().name, *name);
    }
    let catalog = tools["routine_catalog"].execute(json!({})).await.unwrap();
    assert_eq!(catalog["observations"].as_array().unwrap().len(), 3);
    assert!(
        tools["routine_catalog"]
            .execute(json!({"unexpected":true}))
            .await
            .is_err()
    );
    assert!(tools["routine_learn"].execute(json!({})).await.is_err());
    let inputs = json!({"path":"tool.json","before":0,"after":1});
    let learned = tools["routine_learn"]
        .execute(json!({"routine":routine(),"inputs":inputs,"observations":[1,2,3]}))
        .await
        .unwrap();
    assert_eq!(learned["id"], id);
    files.seed("tool.json", 0);
    assert!(
        tools["routine_run"]
            .execute(json!({"id":id,"inputs":inputs}))
            .await
            .is_ok()
    );
    assert!(
        tools["routine_run"]
            .execute(json!({"id":"unknown","inputs":{}}))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn cancelled_replay_retires_the_version_in_the_current_process() {
    use std::future::Future;
    let (library, files, id) = trained().await;
    files.seed("cancelled.json", 0);
    files
        .stall_after_read
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let session = Session::new(library.clone(), files.clone());
    let inputs = json!({"path":"cancelled.json","before":0,"after":1});
    let mut future = Box::pin(session.run(&id, &inputs));
    let mut context = std::task::Context::from_waker(std::task::Waker::noop());
    assert!(future.as_mut().poll(&mut context).is_pending());
    drop(future);
    assert!(library.lock().unwrap().catalog()[&id].retired);
    assert_eq!(
        std::fs::read_to_string(files.dir.path().join("cancelled.json")).unwrap(),
        "0"
    );
}
