use arkavo_router::ModelSpec;

/// Utility commands remain usable while local inference is being provisioned.
pub fn requires_local_runtime(args: &[String]) -> bool {
    if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "-h" | "--help" | "--version"))
    {
        return false;
    }
    match args.first().map(String::as_str) {
        None => true,
        Some("agent") => args.get(1).is_none_or(|arg| arg != "init"),
        Some("chat" | "task" | "ui" | "terminal") => true,
        Some(arg) => arg.starts_with('-') && arg != "-v",
    }
}

pub fn validate_local_backend(args: &[String], available: bool) -> Result<(), &'static str> {
    if requires_local_runtime(args) && !available {
        Err(
            "The agent harness requires a local inference backend. Use a build with local model support; cloud providers only augment local inference.",
        )
    } else {
        Ok(())
    }
}

/// A cloud model selection never removes the harness's local model requirement.
/// An explicit GGUF supplies local weights without a catalog download.
pub fn needs_local_setup(args: &[String]) -> bool {
    requires_local_runtime(args)
        && !args.windows(2).any(|pair| {
            matches!(pair[0].as_str(), "--model" | "--gguf")
                && matches!(ModelSpec::parse(&pair[1]), Some(ModelSpec::GgufPath(path)) if path.is_file())
        })
}

/// How the first-run gate behaves for this invocation.
///
/// This only matters when [`needs_local_setup`] is true and no local weights
/// are cached yet — a harness command with nothing to run on. Utility
/// commands (`model`, `mcp proxy`, `agent init`, ...) never reach this
/// decision at all, in either variant, because `requires_local_runtime`
/// excludes them upstream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FirstRunAction {
    /// Interactive terminal, no skip requested: run the guided setup flow
    /// (detect capabilities, offer to download, verify with a test query).
    Prompt,
    /// `ARKAVO_SKIP_FIRST_RUN` is set, or stdin is not a TTY (container/CI):
    /// never prompt or auto-download. This does not waive the local model
    /// requirement — a harness command still fails with the "requires local
    /// models" error until weights are provisioned some other way (a
    /// pre-provisioned cache, `arkavo model download`, or `--gguf`).
    Skip,
}

/// Decide how the first-run gate behaves for this invocation.
pub fn first_run_action() -> FirstRunAction {
    use std::io::IsTerminal;
    first_run_action_for(
        std::env::var("ARKAVO_SKIP_FIRST_RUN").ok(),
        std::io::stdin().is_terminal(),
    )
}

fn first_run_action_for(skip_env: Option<String>, stdin_is_tty: bool) -> FirstRunAction {
    let skip_requested = skip_env.is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    if skip_requested || !stdin_is_tty {
        FirstRunAction::Skip
    } else {
        FirstRunAction::Prompt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn cloud_selection_still_requires_local_models() {
        for command in [
            args(&["chat"]),
            args(&["chat", "--model", "gpt-6-astra"]),
            args(&["agent", "run"]),
            vec![],
        ] {
            assert!(needs_local_setup(&command));
            assert!(validate_local_backend(&command, false).is_err());
            assert!(validate_local_backend(&command, true).is_ok());
        }
    }

    #[test]
    fn local_file_override_supplies_weights_but_still_requires_a_backend() {
        let file = tempfile::Builder::new().suffix(".gguf").tempfile().unwrap();
        let command = args(&["chat", "--gguf", file.path().to_str().unwrap()]);
        assert!(!needs_local_setup(&command));
        assert!(validate_local_backend(&command, false).is_err());
    }

    #[test]
    fn missing_gguf_does_not_waive_local_provisioning() {
        let directory = tempfile::tempdir().unwrap();
        let missing = directory.path().join("missing.gguf");
        assert!(needs_local_setup(&args(&[
            "chat",
            "--model",
            "gpt-6-astra",
            "--gguf",
            missing.to_str().unwrap()
        ])));
    }

    #[test]
    fn prompt_and_agent_name_are_not_utility_subcommands() {
        for command in [
            args(&["chat", "--prompt", "help"]),
            args(&["agent", "--name", "init"]),
        ] {
            assert!(needs_local_setup(&command));
            assert!(validate_local_backend(&command, false).is_err());
        }
    }

    #[test]
    fn setup_utilities_do_not_require_inference() {
        for command in [
            args(&["model", "download"]),
            args(&["mcp", "proxy"]),
            args(&["agent", "init", "example"]),
            args(&["chat", "--help"]),
        ] {
            assert!(!needs_local_setup(&command));
            assert!(validate_local_backend(&command, false).is_ok());
        }
    }

    #[test]
    fn first_run_action_skip_env() {
        // ARKAVO_SKIP_FIRST_RUN=1 (or "true") skips interactive setup,
        // regardless of TTY state.
        for val in ["1", "true", "TRUE"] {
            assert_eq!(
                first_run_action_for(Some(val.to_string()), true),
                FirstRunAction::Skip,
                "skip value {val}"
            );
            assert_eq!(
                first_run_action_for(Some(val.to_string()), false),
                FirstRunAction::Skip,
                "skip value {val}"
            );
        }
    }

    #[test]
    fn first_run_action_non_tty_never_prompts() {
        // Regression: in a container stdin is at EOF, which read_line reports
        // as empty input; the old code treated that as "yes" and started an
        // unsolicited multi-GB download. Non-TTY stdin must never prompt,
        // whether or not the skip env var is also set.
        assert_eq!(first_run_action_for(None, false), FirstRunAction::Skip);
        // Non-skip env values must not suppress this on non-TTY stdin either.
        for val in ["0", "false", ""] {
            assert_eq!(
                first_run_action_for(Some(val.to_string()), false),
                FirstRunAction::Skip,
                "value {val}"
            );
        }
    }

    #[test]
    fn first_run_action_tty_prompts_unless_skipped() {
        assert_eq!(first_run_action_for(None, true), FirstRunAction::Prompt);
        // A falsy/unset skip env value on a real terminal still prompts.
        for val in ["0", "false", ""] {
            assert_eq!(
                first_run_action_for(Some(val.to_string()), true),
                FirstRunAction::Prompt,
                "value {val}"
            );
        }
    }
}
