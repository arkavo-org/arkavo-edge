// `#[tokio::test]` expands to `Runtime::block_on`, which the workspace lint
// set disallows in library code.
#![allow(clippy::disallowed_methods)]

use super::*;
use crate::testing::{Behavior, fake_cli, png};
use std::time::Duration;

fn spec<'a>(cli: &'a Path, out: &'a Path, args: &[&str], within: Duration) -> RunSpec<'a> {
    RunSpec {
        cli,
        args: args.iter().map(OsString::from).collect(),
        out_dir: out,
        steps: 4,
        deadline: Instant::now() + within,
        sandbox: Sandbox::Off,
    }
}

/// IMG-005: images and steps are reported as sd-cli prints them, and model
/// loading bars are not counted as steps.
#[arkavo_test_macros::spec("IMG-005")]
#[tokio::test]
async fn events_follow_the_output() {
    let tools = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let fixture = tools.path().join("f.png");
    std::fs::write(&fixture, png(8, 8, false)).unwrap();
    let cli = fake_cli(tools.path(), &fixture, Behavior::Generate);
    let pattern = out.path().join("a_%03d.png");
    let events = Mutex::new(Vec::new());
    let output = run(
        spec(
            &cli,
            out.path(),
            &[
                "-o",
                pattern.to_str().unwrap(),
                "-b",
                "2",
                "--steps",
                "4",
                "-s",
                "7",
            ],
            Duration::from_secs(30),
        ),
        &|e| events.lock().unwrap().push(e),
    )
    .await
    .unwrap();
    let events = events.into_inner().unwrap();
    assert_eq!(
        events.first(),
        Some(&Event::ImageStarted {
            image: 1,
            images: 2,
            seed: 7
        })
    );
    let steps = events
        .iter()
        .filter(|e| matches!(e, Event::Step { .. }))
        .count();
    assert_eq!(steps, 8);
    assert_eq!(output.seeds, [7, 8]);
    assert!(output.seconds_per_step.is_some());
    assert!(output.tail.contains("generate_image completed"));
    assert!(out.path().join("a_000.png").exists() && out.path().join("a_001.png").exists());
}

/// IMG-006: a failing run reports its exit status and the end of its
/// output.
#[arkavo_test_macros::spec("IMG-006", "IMG-010")]
#[tokio::test]
async fn failures_carry_status_and_output() {
    let tools = tempfile::tempdir().unwrap();
    let cli = fake_cli(tools.path(), Path::new("/none"), Behavior::Fail);
    let err = run(
        spec(&cli, tools.path(), &[], Duration::from_secs(30)),
        &|_| {},
    )
    .await
    .unwrap_err();
    match err {
        ImageGenError::Runtime { status, stderr } => {
            assert!(status.contains('3'), "{status}");
            assert!(stderr.contains("unsupported op PAD"), "{stderr}");
        }
        other => panic!("expected Runtime, got {other:?}"),
    }
}

/// IMG-012: the deadline kills the process.
#[arkavo_test_macros::spec("IMG-006", "IMG-012")]
#[tokio::test]
async fn deadlines_kill_the_run() {
    let tools = tempfile::tempdir().unwrap();
    let cli = fake_cli(tools.path(), Path::new("/none"), Behavior::Hang);
    let started = StdInstant::now();
    let err = run(
        spec(&cli, tools.path(), &[], Duration::from_millis(300)),
        &|_| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(err, ImageGenError::TimedOut), "{err}");
    assert!(started.elapsed() < Duration::from_secs(10));
}

/// IMG-006: the runtime gets the isolated environment, not the agent's.
#[arkavo_test_macros::spec("IMG-006")]
#[tokio::test]
async fn the_environment_is_isolated() {
    let tools = tempfile::tempdir().unwrap();
    let cli = fake_cli(tools.path(), Path::new("/none"), Behavior::DumpEnv);
    run(
        spec(&cli, tools.path(), &[], Duration::from_secs(30)),
        &|_| {},
    )
    .await
    .unwrap();
    let env = std::fs::read_to_string(tools.path().join("env.txt")).unwrap();
    // cargo sets these for the test process; none may reach the child.
    assert!(std::env::var_os("CARGO_MANIFEST_DIR").is_some());
    assert!(!env.contains("CARGO_MANIFEST_DIR"), "{env}");
    assert!(!env.contains("CARGO_PKG_NAME"), "{env}");
}

#[arkavo_test_macros::spec("IMG-006")]
#[test]
fn the_sandbox_profile_denies_network_and_confines_writes() {
    assert!(MACOS_PROFILE.contains("(deny network*)"));
    assert!(MACOS_PROFILE.contains("(deny file-write*)"));
    assert!(MACOS_PROFILE.contains(r#"(subpath (param "OUT"))"#));
    let param = param("OUT", Path::new("/tmp/out"));
    assert_eq!(param, OsString::from("OUT=/tmp/out"));
    assert!(user_dir().is_absolute());
}
