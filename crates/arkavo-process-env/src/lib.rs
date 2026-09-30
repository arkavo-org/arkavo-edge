//! The environment a child process started by the agent is allowed to see.
//!
//! `Command::new` hands a child every variable of its parent, and the
//! agent's environment is where its provider API keys and service
//! credentials live. A [`ChildEnv`] is resolved instead of inherited: the
//! child sees exactly the variables it holds, because [`ChildEnv::command`]
//! clears the inherited environment before setting them.
//!
//! Two profiles, for two kinds of child:
//! - [`ChildEnv::isolated`] for code the operator did not write (MCP
//!   servers): a small platform baseline plus what the operator declared in
//!   an [`EnvSpec`]. Anything not named is withheld.
//! - [`ChildEnv::toolchain`] for the operator's own toolchains run by
//!   built-in tools (`cargo`, `gh`, `semgrep`): the parent's environment
//!   minus credential-shaped names. Build tooling reads an open-ended set of
//!   variables (`RUSTC_WRAPPER`, `CARGO_HOME`, `SDKROOT`, `DATABASE_URL`), so
//!   an allowlist there would break the operator's builds; what must not
//!   reach them is the agent's own credentials.
//!
//! Both take the parent environment as an argument so a caller other than
//! the agent process (a credential broker) can resolve from its own.

mod baseline;
mod secret;

pub use secret::is_secret_name;

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::process::Command;

/// What an operator declared for one child's environment.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct EnvSpec {
    /// Values set on the child exactly as written.
    pub set: BTreeMap<String, String>,
    /// Names copied from the parent's environment when present there, so a
    /// credential reaches the child without being written into
    /// configuration.
    pub passthrough: Vec<String>,
}

// Values may be credentials; names are enough to debug a configuration.
impl fmt::Debug for EnvSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EnvSpec")
            .field("set", &self.set.keys().collect::<Vec<_>>())
            .field("passthrough", &self.passthrough)
            .finish()
    }
}

/// Exactly the environment one child process will see.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ChildEnv {
    vars: Vec<(OsString, OsString)>,
}

impl ChildEnv {
    /// Platform baseline and `spec.passthrough` names taken from `parent`,
    /// then `spec.set` on top. Nothing else from `parent` is kept.
    pub fn isolated<I, K, V>(parent: I, spec: &EnvSpec) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let mut env = Self::default();
        for (name, value) in parent {
            let name = name.into();
            let declared = spec
                .passthrough
                .iter()
                .any(|wanted| same_name(&name, OsStr::new(wanted)));
            if declared || baseline::is_baseline(&name) {
                env.insert(name, value.into());
            }
        }
        for (name, value) in &spec.set {
            env.insert(name.into(), value.into());
        }
        env
    }

    /// [`ChildEnv::isolated`] resolved from this process's environment.
    pub fn isolated_from_current(spec: &EnvSpec) -> Self {
        Self::isolated(std::env::vars_os(), spec)
    }

    /// Everything in `parent` except credential-shaped names, unless the
    /// name is in `readmit`.
    ///
    /// A name that is not valid Unicode cannot be classified and is
    /// withheld.
    pub fn toolchain<I, K, V>(parent: I, readmit: &[&str]) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let mut env = Self::default();
        for (name, value) in parent {
            let name = name.into();
            let readmitted = readmit
                .iter()
                .any(|wanted| same_name(&name, OsStr::new(wanted)));
            if readmitted || !name.to_str().is_none_or(is_secret_name) {
                env.insert(name, value.into());
            }
        }
        env
    }

    /// [`ChildEnv::toolchain`] resolved from this process's environment.
    pub fn toolchain_from_current(readmit: &[&str]) -> Self {
        Self::toolchain(std::env::vars_os(), readmit)
    }

    /// A command for `program` whose environment is exactly this one.
    ///
    /// A tokio caller converts it with `tokio::process::Command::from`.
    pub fn command(&self, program: impl AsRef<OsStr>) -> Command {
        let mut command = Command::new(program);
        command.env_clear();
        command.envs(self.vars.iter().map(|(name, value)| (name, value)));
        command
    }

    fn insert(&mut self, name: OsString, value: OsString) {
        self.vars
            .retain(|(existing, _)| !same_name(existing, &name));
        self.vars.push((name, value));
    }
}

// Values are withheld for the same reason as in `EnvSpec`.
impl fmt::Debug for ChildEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.vars.iter().map(|(name, _)| name))
            .finish()
    }
}

/// Whether `name` can be set on a child process: non-empty, and free of the
/// `=` and NUL that would corrupt the environment block.
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty() && !name.contains(['=', '\0'])
}

/// Environment names compare case-insensitively on Windows, where the
/// parent may spell `PATH` as `Path`.
fn same_name(a: &OsStr, b: &OsStr) -> bool {
    if cfg!(windows) {
        match (a.to_str(), b.to_str()) {
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b),
            _ => a == b,
        }
    } else {
        a == b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    const PLANTED: &str = "planted-secret-value";

    fn names(env: &ChildEnv) -> Vec<&str> {
        env.vars
            .iter()
            .filter_map(|(name, _)| name.to_str())
            .collect()
    }

    fn value<'a>(env: &'a ChildEnv, wanted: &str) -> Option<&'a str> {
        env.vars
            .iter()
            .find(|(name, _)| same_name(name, OsStr::new(wanted)))
            .and_then(|(_, value)| value.to_str())
    }

    fn parent() -> Vec<(OsString, OsString)> {
        let path = std::env::var_os("PATH").unwrap_or_default();
        [
            ("PATH", path),
            ("HOME", OsString::from("/home/agent")),
            ("OPENAI_API_KEY", OsString::from(PLANTED)),
            ("ARKAVO_MASTER_KEY", OsString::from(PLANTED)),
            ("GITHUB_TOKEN", OsString::from(PLANTED)),
            ("RUSTC_WRAPPER", OsString::from("sccache")),
            ("SERVER_REGION", OsString::from("eu")),
        ]
        .into_iter()
        .map(|(name, value)| (OsString::from(name), value))
        .collect()
    }

    fn spec_with(set: &[(&str, &str)], passthrough: &[&str]) -> EnvSpec {
        EnvSpec {
            set: set
                .iter()
                .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
                .collect(),
            passthrough: passthrough.iter().map(|name| (*name).to_string()).collect(),
        }
    }

    /// Runs `env` (Unix) or `set` (Windows) under `env` and returns its
    /// `NAME=value` lines.
    fn child_sees(env: &ChildEnv) -> Vec<String> {
        #[cfg(unix)]
        let output = env.command("sh").args(["-c", "env"]).output();
        #[cfg(windows)]
        let output = env.command("cmd").args(["/C", "set"]).output();
        let output = output.expect("spawn environment dump");
        assert!(output.status.success(), "environment dump failed");
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.trim_end_matches('\r').to_string())
            .collect()
    }

    #[spec("PENV-001")]
    #[test]
    fn isolated_keeps_baseline_and_declared_names_only() {
        let spec = spec_with(&[("LOG_LEVEL", "debug")], &["GITHUB_TOKEN"]);
        let env = ChildEnv::isolated(parent(), &spec);

        assert_eq!(value(&env, "HOME"), Some("/home/agent"));
        assert_eq!(value(&env, "LOG_LEVEL"), Some("debug"));
        assert_eq!(value(&env, "GITHUB_TOKEN"), Some(PLANTED));
        for withheld in ["OPENAI_API_KEY", "ARKAVO_MASTER_KEY", "RUSTC_WRAPPER"] {
            assert_eq!(value(&env, withheld), None, "{withheld} reached the child");
        }
    }

    #[spec("PENV-001")]
    #[test]
    fn declared_value_replaces_the_inherited_one() {
        let spec = spec_with(&[("HOME", "/srv/mcp")], &[]);
        let env = ChildEnv::isolated(parent(), &spec);
        assert_eq!(value(&env, "HOME"), Some("/srv/mcp"));
        assert_eq!(names(&env).iter().filter(|n| **n == "HOME").count(), 1);
    }

    #[spec("PENV-001")]
    #[test]
    fn isolated_child_sees_nothing_else_of_the_parent() {
        let spec = spec_with(&[("MCP_PROBE_CONFIGURED", "yes")], &["SERVER_REGION"]);
        let seen = child_sees(&ChildEnv::isolated(parent(), &spec));

        assert!(seen.iter().any(|l| l == "MCP_PROBE_CONFIGURED=yes"));
        assert!(seen.iter().any(|l| l == "SERVER_REGION=eu"));
        assert!(
            !seen.iter().any(|l| l.contains(PLANTED)),
            "a planted secret reached the child: {seen:?}"
        );
        // Set by cargo and nextest in this test process's real environment:
        // only a cleared environment keeps it from the child.
        assert!(
            !seen.iter().any(|l| l.starts_with("CARGO_MANIFEST_DIR=")),
            "the test process's own environment leaked into the child"
        );
    }

    #[spec("PENV-002")]
    #[test]
    fn toolchain_withholds_credentials_and_keeps_build_settings() {
        let env = ChildEnv::toolchain(parent(), &[]);
        assert_eq!(value(&env, "RUSTC_WRAPPER"), Some("sccache"));
        assert_eq!(value(&env, "SERVER_REGION"), Some("eu"));
        for withheld in ["OPENAI_API_KEY", "ARKAVO_MASTER_KEY", "GITHUB_TOKEN"] {
            assert_eq!(value(&env, withheld), None, "{withheld} reached the child");
        }
    }

    #[spec("PENV-002")]
    #[test]
    fn toolchain_readmits_only_the_named_credential() {
        let env = ChildEnv::toolchain(parent(), &["GITHUB_TOKEN"]);
        assert_eq!(value(&env, "GITHUB_TOKEN"), Some(PLANTED));
        assert_eq!(value(&env, "OPENAI_API_KEY"), None);
    }

    #[spec("PENV-002")]
    #[test]
    fn toolchain_child_sees_no_planted_credential() {
        let seen = child_sees(&ChildEnv::toolchain(parent(), &[]));
        assert!(seen.iter().any(|l| l == "RUSTC_WRAPPER=sccache"));
        assert!(
            !seen.iter().any(|l| l.contains(PLANTED)),
            "a planted secret reached the child: {seen:?}"
        );
    }

    #[spec("PENV-001")]
    #[test]
    fn debug_output_never_contains_values() {
        let spec = spec_with(&[("API_TOKEN", PLANTED)], &[]);
        let env = ChildEnv::isolated(parent(), &spec);
        assert!(!format!("{spec:?}").contains(PLANTED));
        assert!(!format!("{env:?}").contains(PLANTED));
        assert!(format!("{env:?}").contains("API_TOKEN"));
    }

    #[test]
    fn names_that_would_corrupt_the_environment_are_invalid() {
        assert!(is_valid_name("GITHUB_TOKEN"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("A=B"));
        assert!(!is_valid_name("A\0B"));
    }
}
