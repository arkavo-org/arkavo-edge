//! Resolving the environment the upstream server is started with.
//!
//! The upstream is third-party code, so what it may see of the proxy's
//! environment is decided here, before the process exists, and a
//! configuration that could hand it a credential or a way to run other code
//! is refused rather than trimmed: a launcher that silently dropped a
//! variable would start a server the operator did not configure.

// `pub(crate)` is the real, intended visibility here (the module is private,
// so nothing leaks past the crate either way); `redundant_pub_crate` wants
// `pub`, which `unreachable_pub` then rejects.
#![allow(clippy::redundant_pub_crate)]

use crate::upstream::UpstreamError;
use arkavo_process_env::{ChildEnv, EnvSpec};
use std::ffi::OsString;
use tracing::warn;

/// The environment for `spec`, resolved from this process's environment.
pub(crate) fn resolve(spec: &EnvSpec) -> Result<ChildEnv, UpstreamError> {
    resolve_from(std::env::vars_os().collect(), spec)
}

/// [`resolve`] against an explicit parent environment.
///
/// A name in both `passthrough` and `set` takes the `set` value: the literal
/// is the more specific instruction, and `ChildEnv::isolated` applies it
/// last.
pub(crate) fn resolve_from(
    parent: Vec<(OsString, OsString)>,
    spec: &EnvSpec,
) -> Result<ChildEnv, UpstreamError> {
    screen(spec)?;
    for name in &spec.passthrough {
        let present = parent.iter().any(|(held, _)| held == name.as_str());
        if !present && !spec.set.contains_key(name) {
            // Values are never logged; the name is what an operator needs to
            // find a credential that did not arrive.
            warn!(
                name = name.as_str(),
                "upstream environment passthrough names a variable the proxy does not hold"
            );
        }
    }
    Ok(ChildEnv::isolated(parent, spec))
}

fn screen(spec: &EnvSpec) -> Result<(), UpstreamError> {
    let refuse = |name: &str, why: &str| {
        Err(UpstreamError::Environment(format!(
            "variable '{name}' {why}"
        )))
    };
    for name in spec.set.keys().chain(&spec.passthrough) {
        if !arkavo_process_env::is_valid_name(name) {
            return Err(UpstreamError::Environment(
                "a variable name is empty or contains '=' or NUL".into(),
            ));
        }
        if arkavo_process_env::is_loader_or_hijack_name(name) {
            return refuse(name, "would let the upstream load or run other code");
        }
    }
    if let Some(name) = spec
        .set
        .keys()
        .find(|name| arkavo_process_env::is_secret_name(name))
    {
        return refuse(
            name,
            "holds a credential and is set as a literal; pass it through from the proxy's own environment instead",
        );
    }
    Ok(())
}

#[cfg(test)]
// The `#[tokio::test]` macro expands to `Runtime::block_on`, which
// `.clippy.toml` disallows outside test code.
#[allow(clippy::disallowed_methods)]
mod tests {
    use super::*;
    use crate::policy::AllowAllPolicy;
    use crate::proxy::{McpProxy, ProxyConfig};
    use arkavo_test_macros::spec;
    use std::sync::Arc;

    fn spec_of(set: &[(&str, &str)], passthrough: &[&str]) -> EnvSpec {
        EnvSpec {
            set: set
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            passthrough: passthrough.iter().map(|n| n.to_string()).collect(),
        }
    }

    fn refused(spec: EnvSpec) -> String {
        match McpProxy::spawn(
            ProxyConfig {
                env: spec,
                ..ProxyConfig::new("true", Vec::new())
            },
            Arc::new(AllowAllPolicy),
        ) {
            Err(UpstreamError::Environment(message)) => message,
            Err(other) => panic!("wrong refusal: {other}"),
            Ok(_) => panic!("spawn accepted an unsafe environment"),
        }
    }

    #[spec("PDG-012")]
    #[test]
    fn spawn_refuses_loader_names_in_set_and_passthrough() {
        let set = refused(spec_of(&[("LD_PRELOAD", "/tmp/evil.so")], &[]));
        assert!(set.contains("LD_PRELOAD") && !set.contains("evil"), "{set}");
        let passed = refused(spec_of(&[], &["NODE_OPTIONS"]));
        assert!(passed.contains("NODE_OPTIONS"), "{passed}");
    }

    #[spec("PDG-012")]
    #[test]
    fn spawn_refuses_malformed_names_without_echoing_them() {
        for spec in [
            spec_of(&[("", "v")], &[]),
            spec_of(&[("A=B", "hunter2")], &[]),
            spec_of(&[], &["A=B"]),
        ] {
            let message = refused(spec);
            assert!(!message.contains("hunter2"), "{message}");
        }
    }

    #[spec("PDG-012")]
    #[test]
    fn spawn_refuses_a_credential_literal_but_allows_passing_one_through() {
        let message = refused(spec_of(&[("OPENAI_API_KEY", "sk-live-value")], &[]));
        assert!(
            message.contains("OPENAI_API_KEY") && !message.contains("sk-live-value"),
            "{message}"
        );
        assert!(screen(&spec_of(&[], &["OPENAI_API_KEY"])).is_ok());
    }

    #[test]
    fn passthrough_copies_a_held_name_and_set_wins_a_conflict() {
        let parent = vec![
            (OsString::from("HELD"), OsString::from("from-parent")),
            (OsString::from("BOTH"), OsString::from("from-parent")),
            (OsString::from("UNLISTED"), OsString::from("nope")),
        ];
        let env = resolve_from(
            parent,
            &spec_of(&[("BOTH", "literal")], &["HELD", "BOTH", "ABSENT"]),
        )
        .unwrap();
        let shown = format!("{env:?}");
        assert!(shown.contains("HELD") && shown.contains("BOTH"), "{shown}");
        assert!(!shown.contains("UNLISTED") && !shown.contains("ABSENT"));
    }

    #[derive(Clone, Default)]
    struct Captured(Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for Captured {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Captured {
        type Writer = Self;
        fn make_writer(&'a self) -> Self {
            self.clone()
        }
    }

    #[spec("PDG-012")]
    #[test]
    fn an_absent_passthrough_name_is_warned_about_by_name_only() {
        let captured = Captured::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(captured.clone())
            .with_ansi(false)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let parent = vec![(OsString::from("HELD"), OsString::from("held-value"))];
            resolve_from(
                parent,
                &spec_of(&[("SET", "set-value")], &["HELD", "MISSING"]),
            )
            .unwrap();
        });
        let log = String::from_utf8(captured.0.lock().unwrap().clone()).unwrap();
        assert!(log.contains("MISSING"), "{log}");
        assert!(!log.contains("HELD") && !log.contains("-value"), "{log}");
    }

    #[cfg(unix)]
    async fn env_seen_by_upstream(spec: EnvSpec, tag: &str) -> String {
        let dump =
            std::env::temp_dir().join(format!("arkavo-mcp-proxy-env-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_file(&dump);
        let mut spec = spec;
        spec.set
            .insert("ENV_PROBE_OUT".into(), dump.display().to_string());
        // Renamed into place so a poll never reads a half-written dump.
        let _proxy = McpProxy::spawn(
            ProxyConfig {
                env: spec,
                ..ProxyConfig::new(
                    "sh",
                    vec![
                        "-c".into(),
                        r#"env > "$ENV_PROBE_OUT.tmp" && mv "$ENV_PROBE_OUT.tmp" "$ENV_PROBE_OUT""#
                            .into(),
                    ],
                )
            },
            Arc::new(AllowAllPolicy),
        )
        .expect("spawn");
        for _ in 0..500 {
            if let Ok(seen) = std::fs::read_to_string(&dump) {
                let _ = std::fs::remove_file(&dump);
                return seen;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("the upstream never wrote its environment");
    }

    /// `CARGO_MANIFEST_DIR` is held by the test process and is neither
    /// baseline nor a loader or credential name.
    #[spec("PDG-012")]
    #[tokio::test]
    #[cfg(unix)]
    async fn spawn_delivers_a_held_passthrough_name_and_nothing_else() {
        let seen = env_seen_by_upstream(spec_of(&[], &["CARGO_MANIFEST_DIR"]), "pass").await;
        let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        assert!(
            seen.lines()
                .any(|l| l == format!("CARGO_MANIFEST_DIR={manifest}")),
            "{seen}"
        );
        assert!(!seen.lines().any(|l| l.starts_with("CARGO_PKG_NAME=")));
    }

    #[spec("PDG-012")]
    #[tokio::test]
    #[cfg(unix)]
    async fn spawn_lets_a_literal_override_the_same_name_passed_through() {
        let seen = env_seen_by_upstream(
            spec_of(
                &[("CARGO_MANIFEST_DIR", "literal")],
                &["CARGO_MANIFEST_DIR"],
            ),
            "conflict",
        )
        .await;
        assert!(
            seen.lines().any(|l| l == "CARGO_MANIFEST_DIR=literal"),
            "{seen}"
        );
    }
}
