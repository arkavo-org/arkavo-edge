use std::process::Stdio;

use anyhow::{Result, ensure};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    process::Command,
    sync::watch,
};

use crate::{CodexConfig, RunOutcome, RunStatus, events::EventReader};

pub(crate) fn command(config: &CodexConfig, thread_id: Option<&str>) -> Command {
    let mut cmd = Command::new(&config.executable);
    cmd.arg("exec");
    // `exec resume` is a clap subcommand with its own option set and no
    // `--sandbox` of its own, so the permission travels as `-c sandbox_mode`
    // and every option follows the subcommand, where both forms accept it.
    if let Some(id) = thread_id {
        cmd.args(["resume", id]);
    }
    cmd.args(["--json", "--ignore-user-config", "--ignore-rules"])
        // `--ignore-user-config` also drops the user's trusted-directory list,
        // reducing Codex's own guard to "is this a git repository". The host,
        // not that heuristic, authorizes the workspace, and the sandbox below
        // still bounds the run; without this an ordinary directory fails with a
        // message that only reaches the deliberately suppressed stderr.
        .arg("--skip-git-repo-check")
        .args(["--model", &config.model])
        .args([
            "-c",
            &format!("sandbox_mode=\"{}\"", config.sandbox.as_str()),
        ])
        .args(["-c", "approval_policy=\"never\""])
        .args(["-c", "sandbox_workspace_write.network_access=false"])
        .args(["-c", "web_search=\"disabled\""])
        .args(["-c", "shell_environment_policy.inherit=\"none\""])
        .arg("-")
        .current_dir(&config.workspace)
        .env_remove("CODEX_THREAD_ID")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Diagnostic stderr can contain credentials or file contents. The
        // structured outcome reports failures without forwarding raw logs.
        .stderr(Stdio::null())
        .kill_on_drop(true);
    crate::containment::prepare(&mut cmd);
    cmd
}

pub(crate) async fn run(
    config: &CodexConfig,
    thread_id: Option<&str>,
    prompt: &str,
    mut cancel: watch::Receiver<bool>,
    mut on_thread: impl FnMut(&str) -> Result<()>,
) -> Result<RunOutcome> {
    let mut child = command(config, thread_id).spawn()?;
    // Attached before the prompt is written, so Codex cannot have created any
    // descendant that is outside the tree this owns.
    let mut tree = crate::containment::ProcessTree::attach(&child)?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Missing stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Missing stdout"))?;
    let mut reader = BufReader::new(stdout);
    let mut events = EventReader::default();
    events.outcome.thread_id = thread_id.map(str::to_owned);
    let operation = async {
        // Drain stdout while writing to avoid pipe deadlock for large prompts.
        let write = async move {
            stdin.write_all(prompt.as_bytes()).await?;
            stdin.shutdown().await?;
            drop(stdin);
            Ok::<_, std::io::Error>(())
        };
        let read = async {
            let mut total = 0usize;
            loop {
                let mut line = Vec::new();
                let remaining = config.max_output_bytes.saturating_sub(total);
                let count = (&mut reader)
                    .take(remaining as u64 + 1)
                    .read_until(b'\n', &mut line)
                    .await?;
                if count == 0 {
                    break;
                }
                total += count;
                ensure!(
                    total <= config.max_output_bytes,
                    "Codex output limit exceeded"
                );
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                events.accept(&line)?;
                if let Some(id) = &events.outcome.thread_id {
                    on_thread(id)?;
                }
            }
            Ok::<_, anyhow::Error>(())
        };
        tokio::try_join!(async { write.await.map_err(anyhow::Error::from) }, read)?;
        Ok::<_, anyhow::Error>(child.wait().await?.success())
    };
    let result = tokio::select! {
        result = tokio::time::timeout(config.timeout, operation) => {
            match result {
                Ok(Ok(success)) => {
                    // The leader has been reaped inside `operation`, so its group
                    // id is free to be reused and must never be signalled again.
                    tree.disarm();
                    return Ok(events.finish(success));
                }
                Ok(Err(_)) => (RunStatus::Failed, "Codex process or event protocol failed"),
                Err(_) => (RunStatus::TimedOut, "Codex worker timed out"),
            }
        }
        _ = cancel.wait_for(|value| *value) => (RunStatus::Cancelled, "Codex worker cancelled"),
    };
    // Signal the tree before reaping: an unreaped leader holds its PID, so the
    // group id still names this worker and nothing else.
    tree.kill();
    child.wait().await.ok();
    tree.disarm();
    let mut outcome = events.finish(false);
    outcome.status = result.0;
    outcome.error = Some(result.1.into());
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Sandbox;

    fn fixture(sandbox: Sandbox) -> CodexConfig {
        CodexConfig {
            executable: "codex".into(),
            workspace: "/tmp".into(),
            agent_id: "worker".into(),
            model: "gpt-6-astra".into(),
            sandbox,
            timeout: std::time::Duration::from_secs(1),
            max_output_bytes: 100,
        }
    }

    fn arguments(config: &CodexConfig, thread_id: Option<&str>) -> Vec<String> {
        command(config, thread_id)
            .as_std()
            .get_args()
            .map(|s| s.to_string_lossy().into_owned())
            .collect()
    }

    /// Pinned against `codex exec --help` and `codex exec resume --help` of
    /// codex-cli 0.153.4: `resume` is a subcommand that rejects nothing here
    /// only because the permissions are expressed as configuration overrides.
    #[test]
    fn argument_vector_matches_the_verified_codex_contract() {
        let options = [
            "--json",
            "--ignore-user-config",
            "--ignore-rules",
            "--skip-git-repo-check",
            "--model",
            "gpt-6-astra",
            "-c",
            "sandbox_mode=\"read-only\"",
            "-c",
            "approval_policy=\"never\"",
            "-c",
            "sandbox_workspace_write.network_access=false",
            "-c",
            "web_search=\"disabled\"",
            "-c",
            "shell_environment_policy.inherit=\"none\"",
            "-",
        ];
        let config = fixture(Sandbox::ReadOnly);
        let mut expected = vec!["exec".to_owned()];
        expected.extend(options.iter().map(|s| (*s).to_owned()));
        assert_eq!(arguments(&config, None), expected);

        let thread = "0199a213-81c0-7800-8aa1-bbab2a035a53";
        let mut expected = vec!["exec".to_owned(), "resume".to_owned(), thread.to_owned()];
        expected.extend(options.iter().map(|s| (*s).to_owned()));
        assert_eq!(arguments(&config, Some(thread)), expected);
    }

    #[test]
    fn resume_carries_the_sandbox_and_never_bypasses_it() {
        let thread = Some("0199a213-81c0-7800-8aa1-bbab2a035a53");
        for (sandbox, expected) in [
            (Sandbox::ReadOnly, "sandbox_mode=\"read-only\""),
            (Sandbox::WorkspaceWrite, "sandbox_mode=\"workspace-write\""),
        ] {
            let args = arguments(&fixture(sandbox), thread);
            assert!(args.contains(&expected.to_owned()), "{expected} missing");
            // `codex exec resume` has no --sandbox option; passing one would
            // either be rejected or silently ignored for the resumed thread.
            assert!(!args.iter().any(|a| a == "--sandbox" || a == "-s"));
            assert!(!args.iter().any(|a| a.contains("dangerously")));
            assert_eq!(args.last().unwrap(), "-");
            assert_eq!(args[1], "resume");
        }
    }
}
