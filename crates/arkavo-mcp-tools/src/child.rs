//! Commands for the programs built-in tools run on the operator's behalf.
//!
//! These are the operator's own toolchains (`cargo`, `gh`, `semgrep`,
//! `docker`), so they keep the operator's environment — build settings are
//! open-ended — minus every credential-shaped variable: the agent's provider
//! keys, and anything else a hostile repository's build script or test
//! could read and send on. Without this, an auto-approved `echo $NAME` or a
//! test run by `test_runner` reads every provider key from its environment.

// `pub(crate)` is the real, intended visibility here (the module is private,
// so nothing leaks past the crate either way); `redundant_pub_crate` wants
// `pub`, which `unreachable_pub` then rejects.
#![allow(clippy::redundant_pub_crate)]

use arkavo_process_env::ChildEnv;
use std::ffi::OsString;

/// Comma-separated names the operator lets through to tool subprocesses
/// although they look like credentials, e.g. the private-registry token a
/// `cargo test` needs.
const PASSTHROUGH_ENV: &str = "ARKAVO_TOOL_ENV_PASSTHROUGH";

/// What `gh` authenticates with when the operator exported a token instead
/// of running `gh auth login`.
const GH_CREDENTIALS: &[&str] = &[
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_ENTERPRISE_TOKEN",
    "GITHUB_ENTERPRISE_TOKEN",
];

/// Withheld although they hold no credential. `shell_exec` auto-approves
/// `rg` once its command line carries no `--pre`, and ripgrep reads further
/// flags, `--pre` among them, from the file `RIPGREP_CONFIG_PATH` names.
const FLAG_FILES: &[&str] = &["RIPGREP_CONFIG_PATH"];

/// A blocking command for `program` under the toolchain environment.
pub(crate) fn tool_command(program: &str) -> std::process::Command {
    toolchain_env(&[]).command(program)
}

/// [`tool_command`] for tokio callers.
pub(crate) fn async_tool_command(program: &str) -> tokio::process::Command {
    tool_command(program).into()
}

/// `gh`, which additionally keeps the GitHub token the operator exported.
pub(crate) fn gh_command() -> std::process::Command {
    toolchain_env(GH_CREDENTIALS).command("gh")
}

/// The toolchain environment of this process, with the operator's
/// [`PASSTHROUGH_ENV`] names and `extra` readmitted, and the names this
/// process registered as configured credentials withheld.
pub(crate) fn toolchain_env(extra: &[&str]) -> ChildEnv {
    toolchain_env_from(
        std::env::vars_os().collect(),
        extra,
        &arkavo_process_env::withheld_names(),
    )
}

/// [`toolchain_env`] resolved from `parent`, withholding `configured` (the
/// operator-chosen credential names) and [`FLAG_FILES`] as well.
pub(crate) fn toolchain_env_from(
    parent: Vec<(OsString, OsString)>,
    extra: &[&str],
    configured: &[String],
) -> ChildEnv {
    let passthrough = parent
        .iter()
        .find(|(name, _)| name == PASSTHROUGH_ENV)
        .and_then(|(_, value)| value.to_str())
        .unwrap_or_default()
        .to_owned();
    let mut readmit: Vec<&str> = passthrough
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    readmit.extend_from_slice(extra);
    let mut withhold: Vec<&str> = configured.iter().map(String::as_str).collect();
    withhold.extend_from_slice(FLAG_FILES);
    ChildEnv::toolchain_withholding(parent, &readmit, &withhold)
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn parent(extra: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        [
            ("OPENAI_API_KEY", "planted-secret"),
            ("GH_TOKEN", "gh-token"),
            ("CARGO_REGISTRIES_CORP_TOKEN", "registry-token"),
            ("RUSTC_WRAPPER", "sccache"),
        ]
        .iter()
        .chain(extra)
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
    }

    /// `ChildEnv`'s `Debug` lists the names it holds, never values.
    fn holds(env: &ChildEnv, name: &str) -> bool {
        format!("{env:?}").contains(&format!("{name:?}"))
    }

    #[spec("MCP-016")]
    #[test]
    fn tools_keep_build_settings_but_no_credentials() {
        let env = toolchain_env_from(parent(&[]), &[], &[]);
        assert!(holds(&env, "RUSTC_WRAPPER"));
        for withheld in ["OPENAI_API_KEY", "GH_TOKEN", "CARGO_REGISTRIES_CORP_TOKEN"] {
            assert!(!holds(&env, withheld), "{withheld} reached a tool");
        }
    }

    #[spec("MCP-016")]
    #[test]
    fn gh_keeps_only_the_github_token() {
        let env = toolchain_env_from(parent(&[]), GH_CREDENTIALS, &[]);
        assert!(holds(&env, "GH_TOKEN"));
        assert!(!holds(&env, "OPENAI_API_KEY"));
    }

    #[spec("MCP-016")]
    #[test]
    fn operator_can_readmit_a_named_credential() {
        let env = toolchain_env_from(
            parent(&[(PASSTHROUGH_ENV, "CARGO_REGISTRIES_CORP_TOKEN, ")]),
            &[],
            &[],
        );
        assert!(holds(&env, "CARGO_REGISTRIES_CORP_TOKEN"));
        assert!(!holds(&env, "OPENAI_API_KEY"));
    }

    #[spec("MCP-016")]
    #[test]
    fn configured_credential_names_and_rg_flag_file_are_withheld() {
        let env = toolchain_env_from(
            parent(&[
                ("CORP_LLM_LOGIN", "configured-login"),
                ("RIPGREP_CONFIG_PATH", "/tmp/rgrc"),
            ]),
            &[],
            &["CORP_LLM_LOGIN".to_owned()],
        );
        assert!(holds(&env, "RUSTC_WRAPPER"));
        assert!(!holds(&env, "CORP_LLM_LOGIN"));
        assert!(!holds(&env, "RIPGREP_CONFIG_PATH"));
    }

    /// What `test_runner`'s `cargo`/`pytest` children and every other
    /// [`tool_command`] see, observed from inside a real child.
    #[spec("MCP-016")]
    #[test]
    fn tool_child_sees_no_provider_key() {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut vars = parent(&[
            ("ANTHROPIC_API_KEY", "planted-provider-key"),
            ("CORP_LLM_LOGIN", "planted-configured-login"),
        ]);
        vars.push((OsString::from("PATH"), path));
        let env = toolchain_env_from(vars, &[], &["CORP_LLM_LOGIN".to_owned()]);
        let dir = tempfile::TempDir::new().expect("temp dir");
        #[cfg(unix)]
        let mut dump = env.command("sh");
        #[cfg(unix)]
        dump.args(["-c", "env"]);
        #[cfg(windows)]
        let mut dump = env.command("cmd");
        #[cfg(windows)]
        dump.args(["/C", "set"]);
        let output = dump.current_dir(dir.path()).output();
        let output = output.expect("spawn environment dump");
        let seen = String::from_utf8_lossy(&output.stdout);

        assert!(
            seen.lines()
                .any(|l| l.trim_end() == "RUSTC_WRAPPER=sccache"),
            "{seen}"
        );
        for planted in [
            "planted-secret",
            "planted-provider-key",
            "planted-configured-login",
        ] {
            assert!(!seen.contains(planted), "{planted} reached a tool: {seen}");
        }
    }
}
