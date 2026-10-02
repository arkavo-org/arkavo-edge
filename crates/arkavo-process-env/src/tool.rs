//! The environment of a program a built-in tool runs on the operator's
//! behalf (`sh` for `shell_exec`, `cargo`, `gh`, `git`, `rg`, `docker`).
//!
//! It is the toolchain profile, so the operator's build settings apply,
//! with three additions every such program needs alike, which is why they
//! live here once instead of in each crate that starts one:
//! - the credential names this process registered (a provider's `auth_ref`);
//! - variables that hand a program options its command line was checked
//!   not to carry;
//! - the operator's [`TOOL_ENV_PASSTHROUGH`] grant.

use crate::{ChildEnv, same_name, withheld_names};
use std::ffi::{OsStr, OsString};

/// The operator's grant: comma-separated names every tool child keeps.
///
/// The grant is global. It reaches every tool child, `shell_exec`'s shell
/// included, and readmits whatever the tool profile withholds:
/// credential-shaped names, registered credential names and the flag-file
/// variables alike. A private-registry token a `cargo test` needs is the
/// intended use.
pub const TOOL_ENV_PASSTHROUGH: &str = "ARKAVO_TOOL_ENV_PASSTHROUGH";

/// Variables that give an auto-approved command the options `shell_exec`'s
/// classifier and `codegrep` refuse on its command line.
const FLAG_FILES: &[&str] = &[
    // A file of extra ripgrep flags: `--pre` and `--pre-glob` run a program
    // per file searched, `--hostname-bin` runs one outright.
    "RIPGREP_CONFIG_PATH",
    // Default `less` options: `-o`/`-O` write a log file.
    "LESS",
    // `less`'s input preprocessor and its cleanup step, both commands.
    "LESSOPEN",
    "LESSCLOSE",
];

impl ChildEnv {
    /// The toolchain profile of `parent` that also withholds `withhold` and
    /// the flag-file variables. `readmit`, the caller's grant for one
    /// program, and the names in `parent`'s [`TOOL_ENV_PASSTHROUGH`], the
    /// operator's global grant for every tool program (`shell_exec`'s shell
    /// included), win over everything withheld: credential-shaped names,
    /// `withhold`, registered names and flag files alike.
    pub fn tool<I, K, V>(parent: I, readmit: &[&str], withhold: &[&str]) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let parent: Vec<(OsString, OsString)> = parent
            .into_iter()
            .map(|(name, value)| (name.into(), value.into()))
            .collect();
        let granted = parent
            .iter()
            .find(|(name, _)| same_name(name, OsStr::new(TOOL_ENV_PASSTHROUGH)))
            .and_then(|(_, value)| value.to_str())
            .unwrap_or_default()
            .to_owned();
        let mut readmitted: Vec<&str> = granted
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect();
        readmitted.extend_from_slice(readmit);
        let mut withheld = withhold.to_vec();
        withheld.extend_from_slice(FLAG_FILES);
        Self::toolchain_withholding(parent, &readmitted, &withheld)
    }

    /// [`ChildEnv::tool`] of this process's environment, withholding every
    /// name registered with [`crate::withhold_name`].
    pub fn tool_from_current(readmit: &[&str]) -> Self {
        Self::tool_registered(std::env::vars_os(), readmit)
    }

    fn tool_registered<I, K, V>(parent: I, readmit: &[&str]) -> Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let registered = withheld_names();
        let names: Vec<&str> = registered.iter().map(String::as_str).collect();
        Self::tool(parent, readmit, &names)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::withhold_name;
    use arkavo_test_macros::spec;

    fn parent(extra: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        [
            ("OPENAI_API_KEY", "planted-secret"),
            ("GH_TOKEN", "gh-token"),
            ("CARGO_REGISTRIES_CORP_TOKEN", "registry-token"),
            ("RUSTC_WRAPPER", "sccache"),
            ("RIPGREP_CONFIG_PATH", "/tmp/rgrc"),
            ("LESSOPEN", "|lesspipe %s"),
            ("CORP_LLM_LOGIN", "configured-login"),
        ]
        .iter()
        .chain(extra)
        .map(|(name, value)| (OsString::from(name), OsString::from(value)))
        .collect()
    }

    fn holds(env: &ChildEnv, name: &str) -> bool {
        env.vars
            .iter()
            .any(|(held, _)| same_name(held, OsStr::new(name)))
    }

    #[spec("MCP-016")]
    #[test]
    fn tools_keep_build_settings_but_no_credentials_or_flag_files() {
        let env = ChildEnv::tool(parent(&[]), &[], &["CORP_LLM_LOGIN"]);
        assert!(holds(&env, "RUSTC_WRAPPER"));
        for withheld in [
            "OPENAI_API_KEY",
            "GH_TOKEN",
            "CARGO_REGISTRIES_CORP_TOKEN",
            "RIPGREP_CONFIG_PATH",
            "LESSOPEN",
            "CORP_LLM_LOGIN",
        ] {
            assert!(!holds(&env, withheld), "{withheld} reached a tool");
        }
    }

    #[spec("MCP-016")]
    #[test]
    fn caller_readmits_only_the_names_it_lists() {
        let env = ChildEnv::tool(parent(&[]), &["GH_TOKEN"], &[]);
        assert!(holds(&env, "GH_TOKEN"));
        assert!(!holds(&env, "OPENAI_API_KEY"));
    }

    /// The operator's grant is global: it readmits a credential-shaped
    /// name, a configured one and a flag file alike.
    #[spec("MCP-016")]
    #[test]
    fn operator_grant_readmits_anything_withheld() {
        let grant = "CARGO_REGISTRIES_CORP_TOKEN, CORP_LLM_LOGIN,RIPGREP_CONFIG_PATH,";
        let env = ChildEnv::tool(
            parent(&[(TOOL_ENV_PASSTHROUGH, grant)]),
            &[],
            &["CORP_LLM_LOGIN"],
        );
        for granted in [
            "CARGO_REGISTRIES_CORP_TOKEN",
            "CORP_LLM_LOGIN",
            "RIPGREP_CONFIG_PATH",
        ] {
            assert!(holds(&env, granted), "{granted} was not readmitted");
        }
        assert!(!holds(&env, "OPENAI_API_KEY"));
        assert!(!holds(&env, "LESSOPEN"));
    }

    #[spec("MCP-016")]
    #[test]
    fn tool_profile_withholds_registered_names() {
        // The registry is process-global and tests run in parallel: this
        // name is used by no other test.
        const REGISTERED: &str = "PENV_TOOL_TEST_REGISTERED_LOGIN";
        withhold_name(REGISTERED);
        let env = ChildEnv::tool_registered(parent(&[(REGISTERED, "login")]), &[]);
        assert!(!holds(&env, REGISTERED));
        assert!(holds(&env, "RUSTC_WRAPPER"));
    }
}
