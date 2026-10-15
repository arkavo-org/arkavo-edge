//! Spec traceability for the CLI top level: tracing init, first-run gating
//! and command dispatch (specs/arkavo-edge/cli.spec.yaml, CLI-001..CLI-007).

use arkavo_cli::startup_policy;
use arkavo_test_macros::spec;

fn args(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

#[spec("CLI-001")]
#[test]
fn tracing_init_is_idempotent() {
    // A subscriber installed by an earlier init (or by a host embedding the
    // CLI) must be left in place, not replaced or fatal.
    arkavo_cli::logging::init();
    arkavo_cli::logging::init();
}

#[spec("CLI-001")]
#[test]
fn run_initializes_tracing_once_across_invocations() {
    // Three run() calls share the one-time init; a second subscriber install
    // would be an error the command must survive.
    for invocation in [["--help"], ["-h"], ["help"]] {
        arkavo_cli::run(&args(&invocation)).expect("help invocation succeeds");
    }
}

#[spec("CLI-002")]
#[test]
fn help_commands_skip_first_run_setup() {
    // Help must work on a machine with no models: no download offer, no
    // runtime, straight to usage text.
    for invocation in [["--help"], ["-h"], ["help"]] {
        assert!(
            !startup_policy::needs_local_setup(&args(&invocation)),
            "{invocation:?} must not need local setup"
        );
        arkavo_cli::run(&args(&invocation)).expect("help invocation succeeds without models");
    }
}

#[spec("CLI-003")]
#[test]
fn agent_help_is_dispatched_to_the_agent_handler() {
    arkavo_cli::run(&args(&["agent", "--help"])).expect("agent --help is dispatched");
}

#[spec("CLI-003")]
#[test]
fn agent_dispatch_passes_args_through_and_propagates_the_error() {
    // `agent init` with no name never reaches first-run setup (it is a
    // utility command), so this exercises the full dispatch deterministically.
    let err = arkavo_cli::run(&args(&["agent", "init"]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("Missing agent name"), "{err}");
}

#[spec("CLI-004")]
#[test]
fn chat_help_is_dispatched_to_the_chat_command() {
    arkavo_cli::run(&args(&["chat", "--help"])).expect("chat --help is dispatched");
}

#[spec("CLI-005")]
#[test]
fn task_help_is_dispatched_to_the_task_command() {
    arkavo_cli::run(&args(&["task", "--help"])).expect("task --help is dispatched");
}

#[spec("CLI-005")]
#[test]
fn task_dispatch_propagates_argument_errors() {
    // The task handler's own parser rejects a flag missing its value; the
    // error reaches the caller unchanged.
    let err = arkavo_cli::commands::task::execute(&args(&["--message"]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("--message requires an argument"), "{err}");
}

#[spec("CLI-006")]
#[test]
fn empty_command_line_is_treated_as_the_agent_command() {
    // run() with no command defaults to `agent run`; the startup policy
    // treats the empty command line exactly like that harness command.
    let no_args: Vec<String> = Vec::new();
    assert!(startup_policy::requires_local_runtime(&no_args));
    assert!(startup_policy::needs_local_setup(&no_args));
}

#[spec("CLI-006")]
#[test]
fn empty_command_line_without_a_backend_fails_like_agent_run() {
    let no_args: Vec<String> = Vec::new();
    let err = startup_policy::validate_local_backend(&no_args, false).unwrap_err();
    assert!(err.contains("local inference backend"), "{err}");
    assert!(startup_policy::validate_local_backend(&no_args, true).is_ok());
}

// Where the iOS-simulator test command is not compiled in, it must refuse
// before doing any work. The happy path needs a macOS host with the
// mcp-macos feature and a simulator, which is not available offline.
#[cfg(not(all(target_os = "macos", feature = "mcp-macos")))]
#[spec("CLI-007")]
#[test]
fn test_command_is_refused_where_the_platform_lacks_support() {
    let err = arkavo_cli::run(&args(&["test"])).unwrap_err().to_string();
    assert!(err.contains("requires macOS"), "{err}");
}

#[cfg(not(all(target_os = "macos", feature = "mcp-macos")))]
#[spec("CLI-007")]
#[test]
fn test_command_exits_early_regardless_of_extra_arguments() {
    let err = arkavo_cli::run(&args(&["test", "ArkavoReference"]))
        .unwrap_err()
        .to_string();
    assert!(err.contains("requires macOS"), "{err}");
}
