//! Spec traceability for the interactive chat REPL commands
//! (specs/arkavo-edge/cli.spec.yaml, CLI-008..CLI-017).
//!
//! The REPL in `commands::chat` parses input with `parse_protocol_command`
//! and runs it through `execute_command` on a `ChatSession`; these tests
//! drive that same path. A bare `A2aClient` keeps everything offline.

// `#[tokio::test]` expands to a `block_on`, which the workspace lint flags.
#![allow(clippy::disallowed_methods)]

use arkavo_cli::commands::chat::{
    ChatSession, CommandResult, ContextMode, execute_command, parse_protocol_command,
};
use arkavo_protocol::A2aClient;
use arkavo_test_macros::spec;

fn bare_session() -> ChatSession {
    ChatSession::from_client(A2aClient::new())
}

async fn run_command(session: &mut ChatSession, input: &str) -> CommandResult {
    let (cmd, arg) = parse_protocol_command(input).expect("a slash command parses");
    execute_command(session, cmd, arg).await
}

#[spec("CLI-008")]
#[tokio::test]
async fn new_clears_context_and_starts_a_fresh_session() {
    let Ok(router) = arkavo_router::Router::new_offline().await else {
        eprintln!("Skipping: Router::new_offline requires a local inference backend");
        return;
    };
    let client = A2aClient::with_router(std::sync::Arc::new(router), None);
    let mut session = ChatSession::from_client(client);

    // State in the current session that /new must throw away.
    let result = session.cmd_read("Cargo.toml");
    assert!(matches!(result, CommandResult::Output(_)));
    assert_eq!(session.pending_context().len(), 1);

    let result = run_command(&mut session, "/new").await;
    match result {
        CommandResult::Success(Some(message)) => {
            assert!(message.contains("Started new session"), "{message}");
        }
        other => panic!("expected a new session confirmation, got {other:?}"),
    }
    assert!(session.pending_context().is_empty());
    assert!(session.is_active());
}

#[spec("CLI-008")]
#[tokio::test]
async fn new_without_a_session_manager_reports_the_failure() {
    let mut session = bare_session();
    let result = run_command(&mut session, "/new").await;
    match result {
        CommandResult::Error(message) => {
            assert!(message.contains("Failed to start new session"), "{message}");
        }
        other => panic!("expected an error, got {other:?}"),
    }
}

#[spec("CLI-009")]
#[tokio::test]
async fn clear_empties_pending_context_and_history() {
    let mut session = bare_session();
    let result = session.cmd_read("Cargo.toml");
    assert!(matches!(result, CommandResult::Output(_)));
    // A failed send still records the user message in history.
    let _ = session.send_message("hello").await;
    assert_ne!(session.history(), []);

    let result = run_command(&mut session, "/clear").await;
    assert!(matches!(result, CommandResult::Success(Some(ref m)) if m == "Context cleared"));
    assert!(session.pending_context().is_empty());
    assert_eq!(session.history(), []);
}

#[spec("CLI-009")]
#[tokio::test]
async fn clear_on_an_empty_session_still_confirms() {
    let mut session = bare_session();
    let result = run_command(&mut session, "/clear").await;
    assert!(matches!(result, CommandResult::Success(Some(ref m)) if m == "Context cleared"));
}

#[spec("CLI-010")]
#[tokio::test]
async fn context_sets_mode_and_reports_the_current_one() {
    let mut session = bare_session();
    assert_eq!(session.context_mode(), ContextMode::Auto);

    let result = run_command(&mut session, "/context off").await;
    assert!(matches!(result, CommandResult::Success(_)));
    assert_eq!(session.context_mode(), ContextMode::Off);

    // No argument shows the current mode instead of changing it.
    let result = run_command(&mut session, "/context").await;
    match result {
        CommandResult::Output(text) => assert!(text.contains("off"), "{text}"),
        other => panic!("expected current mode output, got {other:?}"),
    }

    let result = run_command(&mut session, "/context bogus").await;
    assert!(matches!(result, CommandResult::Error(_)));
}

#[spec("CLI-011")]
#[tokio::test]
async fn history_on_a_fresh_session_reports_no_messages() {
    let mut session = bare_session();
    let result = run_command(&mut session, "/history").await;
    match result {
        CommandResult::Output(text) => assert!(text.contains("No messages"), "{text}"),
        other => panic!("expected empty-history output, got {other:?}"),
    }
}

#[spec("CLI-011")]
#[tokio::test]
async fn history_lists_recorded_messages_with_role_and_number() {
    let mut session = bare_session();
    // Recorded even though the bare client's send fails.
    let _ = session.send_message("what time is it").await;

    let result = run_command(&mut session, "/history").await;
    match result {
        CommandResult::Output(text) => {
            assert!(text.contains("Conversation history"), "{text}");
            assert!(text.contains("1. [user]: what time is it"), "{text}");
        }
        other => panic!("expected history output, got {other:?}"),
    }
}

#[spec("CLI-013")]
#[tokio::test]
async fn read_loads_a_file_into_pending_context() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "remember this").unwrap();

    let mut session = bare_session();
    let result = run_command(&mut session, &format!("/read {}", file.display())).await;
    match result {
        CommandResult::Output(text) => assert!(text.contains("remember this"), "{text}"),
        other => panic!("expected read confirmation, got {other:?}"),
    }
    assert_eq!(session.pending_context().len(), 1);
    assert_eq!(session.pending_context()[0].content, "remember this");
}

#[spec("CLI-013")]
#[tokio::test]
async fn read_validates_its_argument_and_the_file() {
    let mut session = bare_session();

    let result = run_command(&mut session, "/read").await;
    assert!(matches!(result, CommandResult::Error(ref m) if m.contains("Usage: /read")));

    let result = run_command(&mut session, "/read /nonexistent/file.txt").await;
    assert!(matches!(result, CommandResult::Error(ref m) if m.contains("Failed to read")));
    assert!(session.pending_context().is_empty());
}

#[spec("CLI-014")]
#[tokio::test]
async fn list_shows_files_and_directories_sorted() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("b-file.txt"), "x").unwrap();
    std::fs::create_dir(dir.path().join("a-subdir")).unwrap();

    let mut session = bare_session();
    let result = run_command(&mut session, &format!("/list {}", dir.path().display())).await;
    match result {
        CommandResult::Output(text) => {
            assert!(text.contains("d a-subdir"), "{text}");
            assert!(text.contains("- b-file.txt"), "{text}");
            assert!(
                text.find("a-subdir") < text.find("b-file.txt"),
                "entries sorted by name: {text}"
            );
        }
        other => panic!("expected directory listing, got {other:?}"),
    }
}

#[spec("CLI-014")]
#[tokio::test]
async fn list_defaults_to_the_current_directory_and_reports_errors() {
    let mut session = bare_session();

    let result = run_command(&mut session, "/list").await;
    assert!(matches!(result, CommandResult::Output(ref text) if text.contains("Contents of .")));

    let result = run_command(&mut session, "/list /nonexistent/dir/12345").await;
    assert!(matches!(result, CommandResult::Error(ref m) if m.contains("Failed to list")));
}

#[spec("CLI-017")]
#[tokio::test]
async fn exit_quit_and_q_all_end_the_repl() {
    for input in ["/exit", "/quit", "/q"] {
        let mut session = bare_session();
        let result = run_command(&mut session, input).await;
        assert!(
            matches!(result, CommandResult::Exit),
            "{input} must end the REPL"
        );
    }
}

#[spec("CLI-017")]
#[test]
fn only_slash_commands_reach_the_command_handler() {
    assert_eq!(parse_protocol_command("/exit"), Some(("exit", None)));
    assert_eq!(parse_protocol_command("/quit"), Some(("quit", None)));
    // Without the slash the text goes to the model, not the command handler.
    assert_eq!(parse_protocol_command("exit"), None);
}
