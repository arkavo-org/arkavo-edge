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
}
