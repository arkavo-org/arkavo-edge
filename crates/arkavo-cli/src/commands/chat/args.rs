//! Argument parsing for `arkavo chat`.
//!
//! Every argument is either recognized or refused. Skipping what is not
//! understood turned a typo into a different command: `--promt "x"` dropped
//! the prompt and opened an interactive session, and scripts passed flags
//! that did nothing while appearing to be honored.

use std::path::Path;

/// Flags read by the pack parser, which runs over the same arguments. Each
/// takes a value. They are recognized here only so they are not refused.
pub(super) const PACK_FLAGS: [&str; 5] = [
    "--pack",
    "--anchor",
    "--index-key",
    "--index-id",
    "--payload-key",
];

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct ChatCliArgs {
    pub(super) prompt: Option<String>,
    pub(super) agent_id: Option<String>,
    pub(super) model: Option<String>,
}

pub(super) fn parse_cli_args(args: &[String]) -> Result<ChatCliArgs, Box<dyn std::error::Error>> {
    let mut flags = ChatCliArgs::default();
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = args.get(i + 1);
        match flag {
            "--prompt" | "--print" => {
                let text = value.ok_or_else(|| format!("{flag} requires the prompt text"))?;
                flags.prompt = Some(text.clone());
                i += 1;
            }
            "--agent-id" => {
                let id = value.ok_or("--agent-id requires an argument")?;
                flags.agent_id = Some(id.clone());
                i += 1;
            }
            "--model" | "--gguf" => {
                flags.model = Some(model_value(flag, value)?);
                i += 1;
            }
            // The value, when present, is skipped here; the pack parser
            // reports a missing one in its own words.
            flag if PACK_FLAGS.contains(&flag) => i += 1,
            // `--verbose`/`-v` are read for every command before dispatch.
            "--debug" | "--verbose" | "-v" | "-h" | "--help" => {}
            other => return Err(unrecognized(other).into()),
        }
        i += 1;
    }
    Ok(flags)
}

fn model_value(flag: &str, value: Option<&String>) -> Result<String, Box<dyn std::error::Error>> {
    let Some(value) = value else {
        return Err(if flag == "--gguf" {
            "--gguf requires a path to a .gguf or .gguf.tdf file".into()
        } else {
            "--model requires a model name or a .gguf path (e.g., ministral-3b, ./adapter.gguf)"
                .into()
        });
    };
    if flag == "--gguf" && !arkavo_router::model_spec::is_gguf_spec(value) {
        return Err("--gguf requires a path ending in .gguf or .gguf.tdf".into());
    }
    if arkavo_router::model_spec::is_gguf_spec(value) {
        let resolved = arkavo_router::model_discovery::resolve_gguf_path(Path::new(value));
        if !resolved.exists() {
            return Err(format!("GGUF not found: {value}").into());
        }
    }
    Ok(value.clone())
}

fn unrecognized(argument: &str) -> String {
    let problem = if argument.starts_with('-') {
        format!("unknown option '{argument}' for 'arkavo chat'")
    } else {
        format!("unexpected argument '{argument}' for 'arkavo chat'; pass the prompt with --prompt")
    };
    format!("{problem}\nRun 'arkavo chat --help' for usage")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    fn error_for(values: &[&str]) -> String {
        parse_cli_args(&args(values)).unwrap_err().to_string()
    }

    #[test]
    fn test_parse_cli_model_name() {
        let flags = parse_cli_args(&args(&["--model", "qwen3.5-0.8b", "--prompt", "hi"])).unwrap();
        assert_eq!(flags.model.as_deref(), Some("qwen3.5-0.8b"));
        assert_eq!(flags.prompt.as_deref(), Some("hi"));
    }

    #[test]
    fn test_parse_cli_gguf_alias_requires_suffix() {
        assert!(error_for(&["--gguf", "not-a-model"]).contains(".gguf"));
    }

    #[test]
    fn test_parse_cli_missing_gguf_path_errors() {
        assert!(error_for(&["--model", "models/missing-adapter.gguf"]).contains("GGUF not found"));
    }

    #[test]
    fn test_parse_cli_gguf_flag_missing_arg() {
        assert!(error_for(&["--gguf"]).contains("--gguf requires a path"));
    }

    #[test]
    fn test_parse_cli_existing_gguf_path() {
        // A unique file per run: a fixed name in the shared temp dir let
        // concurrent test processes delete each other's fixture mid-test.
        let file = tempfile::Builder::new()
            .prefix("arkavo-chat-cli-test")
            .suffix(".gguf")
            .tempfile()
            .unwrap();
        std::fs::write(file.path(), b"gguf").unwrap();
        let path = file.path();
        let flags = parse_cli_args(&["--gguf".into(), path.to_string_lossy().into()]).unwrap();
        assert_eq!(flags.model.as_deref(), path.to_str());
        let flags = parse_cli_args(&["--model".into(), path.to_string_lossy().into()]).unwrap();
        assert_eq!(flags.model.as_deref(), path.to_str());
    }

    /// Regression: `--promt "x"` was skipped, so the prompt was lost and the
    /// command opened an interactive session.
    #[test]
    fn misspelled_flag_is_an_error() {
        let err = error_for(&["--promt", "What is 2+2?"]);
        assert!(err.contains("unknown option '--promt'"), "{err}");
        assert!(err.contains("arkavo chat --help"), "{err}");
    }

    /// Regression: example scripts passed `--repo-context off`, which no
    /// code reads. Repository context is set with `/context` in a session.
    #[test]
    fn repo_context_is_not_a_flag() {
        let err = error_for(&["--repo-context", "off", "--prompt", "hi"]);
        assert!(err.contains("unknown option '--repo-context'"), "{err}");
    }

    #[test]
    fn bare_prompt_is_an_error_that_names_the_flag() {
        let err = error_for(&["What is 2+2?"]);
        assert!(err.contains("unexpected argument 'What is 2+2?'"), "{err}");
        assert!(err.contains("--prompt"), "{err}");
    }

    /// Regression: `--prompt` as the last argument fell through to the
    /// ignored arm and opened an interactive session.
    #[test]
    fn prompt_without_text_is_an_error() {
        for flag in ["--prompt", "--print"] {
            let err = error_for(&[flag]);
            assert!(err.contains(&format!("{flag} requires")), "{err}");
        }
    }

    #[test]
    fn agent_id_without_value_is_an_error() {
        assert!(error_for(&["--agent-id"]).contains("--agent-id requires"));
    }

    #[test]
    fn prompt_text_may_look_like_a_flag() {
        let flags = parse_cli_args(&args(&["--prompt", "--explain this flag"])).unwrap();
        assert_eq!(flags.prompt.as_deref(), Some("--explain this flag"));
    }

    #[test]
    fn documented_flags_are_accepted_together() {
        let flags = parse_cli_args(&args(&[
            "--debug",
            "--verbose",
            "-v",
            "--agent-id",
            "code-analyzer-agent",
            "--prompt",
            "hi",
        ]))
        .unwrap();
        assert_eq!(flags.agent_id.as_deref(), Some("code-analyzer-agent"));
        assert_eq!(flags.prompt.as_deref(), Some("hi"));
        assert_eq!(flags.model, None);
    }

    #[test]
    fn pack_flags_and_their_values_are_left_to_the_pack_parser() {
        let flags = parse_cli_args(&args(&[
            "--pack",
            "./pack",
            "--anchor",
            "anchor.pub",
            "--index-key",
            "index.key",
            "--index-id",
            "default",
            "--payload-key",
            "payload.key",
            "--prompt",
            "hi",
        ]))
        .unwrap();
        assert_eq!(flags.prompt.as_deref(), Some("hi"));
        assert!(parse_cli_args(&args(&["--pack"])).is_ok());
    }

    #[test]
    fn nothing_after_a_refused_argument_is_acted_on() {
        let err = error_for(&["--prompt", "hi", "--stream"]);
        assert!(err.contains("unknown option '--stream'"), "{err}");
    }
}
