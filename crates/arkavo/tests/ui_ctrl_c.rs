//! `arkavo ui` stops on Ctrl+C, also when started where SIGINT is ignored
//! (CLI-021).
#![cfg(unix)]
#![allow(clippy::disallowed_methods)]

use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

// The workspace loads a small local model at startup, and that model is
// what turned Ctrl+C into a ggml Metal assert.
#[test]
#[ignore = "requires a small local model in the HuggingFace cache"]
fn ctrl_c_stops_the_workspace_started_with_sigint_ignored() {
    let models = std::env::var_os("HF_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME").map(|h| std::path::Path::new(&h).join(".cache/huggingface"))
        })
        .expect("a HuggingFace cache");
    // The workspace's own files go to a home of its own.
    let home = tempfile::tempdir().expect("temp home");
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .expect("free port")
        .port();
    // A shell running a command in the background hands it SIGINT ignored;
    // `trap '' INT` and exec reproduce that.
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(r#"trap '' INT; exec "$0" ui --no-open --port "$1""#)
        .arg(env!("CARGO_BIN_EXE_arkavo"))
        .arg(port.to_string())
        .env("HOME", home.path())
        .env("HF_HOME", &models)
        .env_remove("RUST_LOG")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start arkavo ui");

    let mut stdout = BufReader::new(child.stdout.take().expect("stdout"));
    let mut printed = String::new();
    while !printed.contains("Press Ctrl+C to stop.") {
        let mut line = String::new();
        let read = stdout.read_line(&mut line).expect("read stdout");
        assert!(read > 0, "arkavo ui ended before serving:\n{printed}");
        printed.push_str(&line);
    }

    let interrupted = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
        .status()
        .expect("send SIGINT");
    assert!(interrupted.success());

    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("arkavo ui was still running 15s after Ctrl+C");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert!(status.success(), "arkavo ui exited with {status}");

    let mut rest = String::new();
    stdout.read_to_string(&mut rest).expect("read stdout");
    assert!(rest.trim().is_empty(), "printed after Ctrl+C: {rest}");
    let mut stderr = String::new();
    child
        .stderr
        .take()
        .expect("stderr")
        .read_to_string(&mut stderr)
        .expect("read stderr");
    assert!(
        !stderr.contains("panicked") && !stderr.contains("GGML_ASSERT"),
        "stderr after Ctrl+C:\n{stderr}"
    );
}
