use arkavo_router::ModelSpec;

/// Whether first-run setup must provision local weights before this command runs.
///
/// A remote selection should fail on its own credentials, not ask to download
/// unrelated weights before the router has inspected the request. An explicit
/// `--model`/`--gguf` answers for itself; otherwise only an install with no
/// cloud credentials needs local weights to do anything at all.
pub fn needs_local_setup(args: &[String], has_cloud: bool) -> bool {
    let selected = args.windows(2).find_map(|pair| {
        matches!(pair[0].as_str(), "--model" | "--gguf")
            .then(|| ModelSpec::parse(&pair[1]))
            .flatten()
    });
    match selected {
        Some(ModelSpec::Named(model)) => model.is_local(),
        Some(ModelSpec::GgufPath(_)) => false,
        None => !has_cloud,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    #[test]
    fn explicit_cloud_does_not_require_local_weights() {
        assert!(!needs_local_setup(
            &args(&["chat", "--model", "gpt-6-astra"]),
            false,
        ));
    }

    #[test]
    fn cloud_credentials_skip_download_but_explicit_local_keeps_setup() {
        assert!(!needs_local_setup(&args(&["chat"]), true));
        assert!(needs_local_setup(
            &args(&["chat", "--model", "ministral-3b"]),
            true,
        ));
        assert!(needs_local_setup(&args(&["chat"]), false));
    }

    #[test]
    fn file_override_does_not_download_catalog_models() {
        assert!(!needs_local_setup(
            &args(&["chat", "--gguf", "./adapter.gguf"]),
            false,
        ));
    }
}
