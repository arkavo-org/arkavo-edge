//! Recognising a variable that makes a program load or run other code.
//!
//! The dynamic loader, language runtimes, `git` and shells read variables
//! that name code to execute (`LD_PRELOAD`, `NODE_OPTIONS`, `PYTHONPATH`,
//! `GIT_SSH_COMMAND`, `BASH_ENV`). Setting one on a child turns a trusted
//! command into an arbitrary-code launcher.
//!
//! This is a deny-list, not an allowlist, because a kit's environment
//! legitimately varies from server to server and the kit author already
//! chooses the command and its arguments. It is defence in depth for a
//! trusted command, not a boundary against an untrusted one.

/// Families of variables that all name code to load or run.
const PREFIXES: &[&str] = &["LD_", "DYLD_", "PYTHON", "RUBY", "PERL", "GIT_", "NODE_"];

/// Single variables that name code to load or run, or that redirect where
/// the child looks for it.
const EXACT: &[&str] = &[
    "JAVA_TOOL_OPTIONS",
    "_JAVA_OPTIONS",
    "BASH_ENV",
    "ENV",
    "PATH",
    "HOME",
    "SHELL",
    "IFS",
    "PS4",
    "RIPGREP_CONFIG_PATH",
    "LESSOPEN",
    "LESSCLOSE",
    "RUSTC_WRAPPER",
];

/// Whether `name` can make a child load or run code its command line does
/// not name. Case-insensitive, because Windows folds case and a loader that
/// ignores `ld_preload` today may not tomorrow.
pub fn is_loader_or_hijack_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    // `NODE_ENV` selects a mode ("production") and loads no code, and
    // build tooling sets it as a matter of course.
    if upper == "NODE_ENV" {
        return false;
    }
    PREFIXES.iter().any(|prefix| upper.starts_with(prefix)) || EXACT.contains(&upper.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("SK-105")]
    #[test]
    fn every_listed_name_and_prefix_is_recognised() {
        let names = [
            "LD_PRELOAD",
            "LD_LIBRARY_PATH",
            "DYLD_INSERT_LIBRARIES",
            "PYTHONPATH",
            "PYTHONSTARTUP",
            "RUBYOPT",
            "PERL5OPT",
            "GIT_SSH_COMMAND",
            "GIT_CONFIG_COUNT",
            "NODE_OPTIONS",
            "JAVA_TOOL_OPTIONS",
            "_JAVA_OPTIONS",
            "BASH_ENV",
            "ENV",
            "PATH",
            "HOME",
            "SHELL",
            "IFS",
            "PS4",
            "RIPGREP_CONFIG_PATH",
            "LESSOPEN",
            "LESSCLOSE",
            "RUSTC_WRAPPER",
        ];
        for name in names {
            assert!(is_loader_or_hijack_name(name), "{name}");
        }
    }

    #[spec("SK-105")]
    #[test]
    fn matching_ignores_case() {
        for name in [
            "ld_preload",
            "Node_Options",
            "pythonpath",
            "Path",
            "git_ssh",
        ] {
            assert!(is_loader_or_hijack_name(name), "{name}");
        }
    }

    #[spec("SK-105")]
    #[test]
    fn ordinary_configuration_passes() {
        for name in [
            "LOG_LEVEL",
            "RUST_LOG",
            "GITHUB_TOKEN",
            "ENVIRONMENT",
            "LDAP_URL",
            "NODEJS_VERSION",
            "HOMEPAGE",
            "NODE_ENV",
            "node_env",
        ] {
            assert!(!is_loader_or_hijack_name(name), "{name}");
        }
    }
}
