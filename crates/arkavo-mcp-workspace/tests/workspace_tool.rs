//! Spec-traceable integration tests for `WorkspaceTool`, run against a fake
//! container runtime on `PATH` so they work offline and leave no container
//! behind. Each `#[spec]` test re-runs this binary with a controlled `PATH`
//! and `ARKAVO_WS_LOGDIR`; the matching `probe_*` test is the half that
//! executes in the re-run process and drives the tool. A probe run directly
//! (no `ARKAVO_WS_LOGDIR`) returns immediately.
#![cfg(unix)]

use std::path::{Path, PathBuf};

use arkavo_mcp::Tool;
use arkavo_mcp_workspace::WorkspaceTool;
use arkavo_test_macros::spec;
use serde_json::{Value, json};

const LOGDIR: &str = "ARKAVO_WS_LOGDIR";

fn logdir() -> Option<PathBuf> {
    std::env::var_os(LOGDIR).map(PathBuf::from)
}

/// A runtime that logs every invocation and answers `--version` and `run`.
const LOGGING_RUNTIME_RUN: &str = "#!/bin/sh
echo \"$0 $*\" >> \"$ARKAVO_WS_LOGDIR/argv.log\"
case \"$1\" in
--version) echo \"Docker version 26.0.0\";;
run) echo \"cid-001\";;
esac
exit 0
";

/// A runtime that only logs, so `ps` and `rm` succeed with empty output.
const LOGGING_RUNTIME: &str = "#!/bin/sh
echo \"$0 $*\" >> \"$ARKAVO_WS_LOGDIR/argv.log\"
exit 0
";

const EXEC_CAPTURE_RUNTIME: &str = "#!/bin/sh
echo \"$0 $*\" >> \"$ARKAVO_WS_LOGDIR/argv.log\"
case \"$1\" in
--version) echo \"Docker version 26.0.0\";;
exec) echo \"hello stdout\"; echo \"hello stderr\" >&2; exit 42;;
esac
exit 0
";

/// `exec` replaces the script with the sleeper, so the tool's kill on
/// timeout ends it instead of orphaning it.
const EXEC_SLOW_RUNTIME: &str = "#!/bin/sh
echo \"$0 $*\" >> \"$ARKAVO_WS_LOGDIR/argv.log\"
case \"$1\" in
--version) echo \"Docker version 26.0.0\";;
exec) exec /bin/sleep 10;;
esac
exit 0
";

const RM_FAILING_RUNTIME: &str = "#!/bin/sh
echo \"$0 $*\" >> \"$ARKAVO_WS_LOGDIR/argv.log\"
case \"$1\" in
--version) echo \"Docker version 26.0.0\";;
rm) echo \"Error response from daemon: No such container: $3\" >&2; exit 1;;
esac
exit 0
";

fn write_runtime(dir: &Path, name: &str, script: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    let runtime = bin.join(name);
    std::fs::write(&runtime, script).expect("fake runtime");
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755))
        .expect("make fake runtime executable");
    bin
}

fn run_probe(probe: &str, dir: &tempfile::TempDir, path: &Path) {
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", probe, "--nocapture"])
        .env_remove(arkavo_process_env::TOOL_ENV_PASSTHROUGH)
        .env(LOGDIR, dir.path())
        .env("PATH", path)
        .output()
        .expect("re-run test binary");
    let log = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success() && log.contains("1 passed"), "{log}");
}

fn argv_log(dir: &Path) -> String {
    std::fs::read_to_string(dir.join("argv.log")).unwrap_or_default()
}

#[tokio::test]
async fn probe_create_with_limits() {
    if logdir().is_none() {
        return;
    }
    let result = WorkspaceTool::new()
        .execute(json!({
            "action": "create",
            "workspace_id": "arkavo-workspace-limits",
            "image": "alpine:3.20",
            "cpu_limit": "1.5",
            "memory_limit": "512m",
        }))
        .await
        .expect("create with limits");
    assert_eq!(result["success"], json!(true));
    assert_eq!(
        result["result"],
        json!("Workspace arkavo-workspace-limits created successfully")
    );
}

#[spec("WORKSPACE-001", "WORKSPACE-004")]
#[test]
fn create_starts_container_with_resource_limits() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let bin = write_runtime(dir.path(), "docker", LOGGING_RUNTIME_RUN);
    run_probe("probe_create_with_limits", &dir, &bin);
    let log = argv_log(dir.path());
    assert!(log.contains("docker --version"), "{log}");
    assert!(
        log.contains(
            "docker run -d --name arkavo-workspace-limits --rm --cpus 1.5 --memory 512m --network none alpine:3.20 sleep infinity"
        ),
        "{log}"
    );
}

#[tokio::test]
async fn probe_create_defaults() {
    if logdir().is_none() {
        return;
    }
    let tool = WorkspaceTool::new();
    for params in [
        json!({
            "action": "create",
            "workspace_id": "ws-defaults",
            "env": {"FOO": "bar"},
        }),
        json!({
            "action": "create",
            "workspace_id": "ws-network",
            "network": true,
        }),
    ] {
        let result = tool.execute(params).await.expect("create");
        assert_eq!(result["success"], json!(true));
    }
}

#[spec("WORKSPACE-001")]
#[test]
fn create_disables_network_by_default_and_passes_env() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let bin = write_runtime(dir.path(), "docker", LOGGING_RUNTIME_RUN);
    run_probe("probe_create_defaults", &dir, &bin);
    let log = argv_log(dir.path());
    assert!(
        log.contains(
            "docker run -d --name ws-defaults --rm --network none -e FOO=bar ubuntu:22.04 sleep infinity"
        ),
        "{log}"
    );
    assert!(
        log.contains("docker run -d --name ws-network --rm ubuntu:22.04 sleep infinity"),
        "{log}"
    );
}

#[tokio::test]
async fn probe_create_memory_only() {
    if logdir().is_none() {
        return;
    }
    let result = WorkspaceTool::new()
        .execute(json!({
            "action": "create",
            "workspace_id": "ws-mem",
            "memory_limit": "1g",
        }))
        .await
        .expect("create with memory limit");
    assert_eq!(result["success"], json!(true));
}

#[spec("WORKSPACE-004")]
#[test]
fn create_applies_memory_quota_without_cpu_quota() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let bin = write_runtime(dir.path(), "docker", LOGGING_RUNTIME_RUN);
    run_probe("probe_create_memory_only", &dir, &bin);
    let log = argv_log(dir.path());
    assert!(
        log.contains(
            "docker run -d --name ws-mem --rm --memory 1g --network none ubuntu:22.04 sleep infinity"
        ),
        "{log}"
    );
    assert!(!log.contains("--cpus"), "{log}");
}

#[tokio::test]
async fn probe_execute_capture() {
    if logdir().is_none() {
        return;
    }
    let result = WorkspaceTool::new()
        .execute(json!({
            "action": "execute",
            "workspace_id": "ws-exec",
            "command": "echo hi",
        }))
        .await
        .expect("execute");
    let output: Value = serde_json::from_str(result["result"].as_str().expect("result string"))
        .expect("result is json");
    assert_eq!(output["stdout"], json!("hello stdout\n"));
    assert_eq!(output["stderr"], json!("hello stderr\n"));
    assert_eq!(output["exit_code"], json!(42));
}

#[spec("WORKSPACE-002")]
#[test]
fn execute_captures_stdout_stderr_and_exit_code() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let bin = write_runtime(dir.path(), "docker", EXEC_CAPTURE_RUNTIME);
    run_probe("probe_execute_capture", &dir, &bin);
    let log = argv_log(dir.path());
    assert!(log.contains("docker exec ws-exec sh -c echo hi"), "{log}");
}

#[tokio::test]
async fn probe_execute_timeout() {
    if logdir().is_none() {
        return;
    }
    let err = WorkspaceTool::new()
        .execute(json!({
            "action": "execute",
            "workspace_id": "ws-slow",
            "command": "sleep 10",
            "timeout": 1,
        }))
        .await
        .expect_err("must time out");
    assert!(
        err.to_string().contains("timed out after 1 seconds"),
        "{err}"
    );
}

#[spec("WORKSPACE-002")]
#[test]
fn execute_kills_command_after_timeout() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let bin = write_runtime(dir.path(), "docker", EXEC_SLOW_RUNTIME);
    run_probe("probe_execute_timeout", &dir, &bin);
    let log = argv_log(dir.path());
    assert!(log.contains("docker exec ws-slow sh -c sleep 10"), "{log}");
}

#[tokio::test]
async fn probe_cleanup() {
    if logdir().is_none() {
        return;
    }
    let result = WorkspaceTool::new()
        .execute(json!({
            "action": "cleanup",
            "workspace_id": "arkavo-workspace-old",
        }))
        .await
        .expect("cleanup");
    assert_eq!(
        result["result"],
        json!("Workspace arkavo-workspace-old cleaned up")
    );
}

#[spec("WORKSPACE-005")]
#[test]
fn cleanup_removes_container_and_confirms() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let bin = write_runtime(dir.path(), "docker", LOGGING_RUNTIME);
    run_probe("probe_cleanup", &dir, &bin);
    let log = argv_log(dir.path());
    assert!(log.contains("docker rm -f arkavo-workspace-old"), "{log}");
}

#[tokio::test]
async fn probe_cleanup_failure() {
    if logdir().is_none() {
        return;
    }
    let err = WorkspaceTool::new()
        .execute(json!({
            "action": "cleanup",
            "workspace_id": "ws-gone",
        }))
        .await
        .expect_err("cleanup must fail");
    assert!(err.to_string().contains("Cleanup failed"), "{err}");
    assert!(err.to_string().contains("No such container"), "{err}");
}

#[spec("WORKSPACE-005")]
#[test]
fn cleanup_reports_runtime_failure() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let bin = write_runtime(dir.path(), "docker", RM_FAILING_RUNTIME);
    run_probe("probe_cleanup_failure", &dir, &bin);
    let log = argv_log(dir.path());
    assert!(log.contains("docker rm -f ws-gone"), "{log}");
}

#[tokio::test]
async fn probe_list() {
    if logdir().is_none() {
        return;
    }
    let result = WorkspaceTool::new()
        .execute(json!({ "action": "list" }))
        .await
        .expect("list");
    assert_eq!(result["success"], json!(true));
}

#[tokio::test]
async fn probe_list_no_runtime() {
    if logdir().is_none() {
        return;
    }
    let err = WorkspaceTool::new()
        .execute(json!({ "action": "list" }))
        .await
        .expect_err("no runtime available");
    assert!(
        err.to_string().contains("Neither Docker nor Podman found"),
        "{err}"
    );
}

#[spec("WORKSPACE-006")]
#[test]
fn docker_is_preferred_over_podman() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    write_runtime(dir.path(), "podman", LOGGING_RUNTIME);
    let bin = write_runtime(dir.path(), "docker", LOGGING_RUNTIME);
    run_probe("probe_list", &dir, &bin);
    let log = argv_log(dir.path());
    assert!(log.contains("docker --version"), "{log}");
    assert!(
        log.contains("docker ps --filter name=arkavo-workspace- --format {{.Names}}"),
        "{log}"
    );
    assert!(!log.contains("podman"), "{log}");
}

#[spec("WORKSPACE-006")]
#[test]
fn podman_is_the_fallback_when_docker_is_missing() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let bin = write_runtime(dir.path(), "podman", LOGGING_RUNTIME);
    run_probe("probe_list", &dir, &bin);
    let log = argv_log(dir.path());
    assert!(log.contains("podman --version"), "{log}");
    assert!(
        log.contains("podman ps --filter name=arkavo-workspace- --format {{.Names}}"),
        "{log}"
    );
}

#[spec("WORKSPACE-006")]
#[test]
fn missing_runtime_is_a_typed_error() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).expect("empty bin dir");
    run_probe("probe_list_no_runtime", &dir, &bin);
}
