//! Logs must not share a stream with a command's output.
//!
//! The streams of the running test process cannot be inspected from inside
//! it, so the test runs this same binary again as a child that installs the
//! CLI's logging and emits one error, and reads what the child wrote where.

use std::process::Command;

const CHILD_MARKER: &str = "ARKAVO_LOG_STREAM_TEST_CHILD";
const LOG_LINE: &str = "log-stream-regression-marker";

/// The child half. A no-op in a normal run of the suite.
#[test]
fn emit_error_log_when_run_as_child() {
    if std::env::var_os(CHILD_MARKER).is_none() {
        return;
    }
    arkavo_cli::logging::init();
    tracing::error!("{LOG_LINE}");
}

/// Regression: ERROR lines were written to stdout, mixed into the model's
/// answer, because the subscriber was left on its default writer.
#[test]
fn error_logs_go_to_stderr_not_stdout() {
    let output = Command::new(std::env::current_exe().expect("test binary path"))
        .args(["--exact", "emit_error_log_when_run_as_child", "--nocapture"])
        .env(CHILD_MARKER, "1")
        .env_remove("RUST_LOG")
        .output()
        .expect("run the test binary as a child");
    assert!(output.status.success(), "child failed: {output:?}");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(LOG_LINE),
        "the error should be logged to stderr, got:\n{stderr}"
    );
    assert!(
        !stdout.contains(LOG_LINE),
        "the error must not reach stdout, got:\n{stdout}"
    );
}
