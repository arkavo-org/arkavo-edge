//! How `browser_cdp` starts Chrome: inside Chrome's own sandbox, and
//! without the agent's credentials in its environment.
//!
//! The sandbox is what confines a renderer compromised by a hostile page;
//! without it that page runs code as the agent's user, with the agent's
//! files and network. It stays on unless Chrome cannot start with it — on
//! Linux Chrome refuses to run sandboxed as root, and a rootless host
//! without user namespaces has no sandbox to give it — and the decision is
//! never taken from tool parameters, which the model writes.

use crate::{BrowserError, Result};
use arkavo_process_env::credential_names;
use chromiumoxide::browser::{BrowserConfig, BrowserConfigBuilder};
use std::ffi::{OsStr, OsString};
use std::sync::Once;

/// Operator opt-out for a Linux host where Chrome has no usable sandbox
/// (user namespaces disabled and no setuid helper). INSECURE: a compromised
/// renderer then runs unconfined. Only the value `1` is honoured.
const ALLOW_UNSANDBOXED_ENV: &str = "ARKAVO_ALLOW_UNSANDBOXED_BROWSER";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sandbox {
    Enabled,
    /// Chrome exits at start-up when asked to sandbox itself as root on
    /// Linux, so the choice there is no sandbox or no browser.
    DisabledRunningAsRoot,
    /// The operator set [`ALLOW_UNSANDBOXED_ENV`].
    DisabledByOperator,
}

/// macOS and Windows always sandbox Chrome; only Linux has the two
/// conditions under which it cannot start sandboxed.
pub(crate) fn decide(os: &str, effective_uid: Option<u32>, opt_out: Option<&OsStr>) -> Sandbox {
    if os != "linux" {
        return Sandbox::Enabled;
    }
    if effective_uid == Some(0) {
        return Sandbox::DisabledRunningAsRoot;
    }
    if opt_out == Some(OsStr::new("1")) {
        return Sandbox::DisabledByOperator;
    }
    Sandbox::Enabled
}

/// The decision for this process, logged whenever the sandbox is off.
pub(crate) fn current_sandbox() -> Sandbox {
    let opt_out = std::env::var_os(ALLOW_UNSANDBOXED_ENV);
    if opt_out_is_ignored(std::env::consts::OS, opt_out.as_deref()) {
        static WARNED: Once = Once::new();
        // Once per process: the decision is taken for every launch.
        WARNED.call_once(|| {
            tracing::warn!(
                "{ALLOW_UNSANDBOXED_ENV} is ignored: Chrome is always sandboxed outside Linux"
            );
        });
    }
    let sandbox = decide(std::env::consts::OS, effective_uid(), opt_out.as_deref());
    if sandbox != Sandbox::Enabled {
        tracing::warn!(?sandbox, "launching Chrome without its sandbox");
    }
    sandbox
}

/// An operator who sets the opt-out on macOS or Windows expects an
/// unsandboxed browser and would otherwise never learn they did not get one.
fn opt_out_is_ignored(os: &str, opt_out: Option<&OsStr>) -> bool {
    os != "linux" && opt_out.is_some()
}

/// Effective uid from `/proc/self/status`; std has no `geteuid`. `None`
/// when it cannot be read, which keeps the sandbox on.
fn effective_uid() -> Option<u32> {
    parse_effective_uid(&std::fs::read_to_string("/proc/self/status").ok()?)
}

/// The `Uid:` line lists real, effective, saved and filesystem uids.
fn parse_effective_uid(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Uid:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

/// chromiumoxide 0.7 can add to Chrome's environment but cannot clear it,
/// so every credential in `parent` is overridden with an empty value
/// instead of being inherited. Which names are credentials is the shared
/// child-environment policy's decision. Its operator grant,
/// `ARKAVO_TOOL_ENV_PASSTHROUGH`, is for tool programs and is not honoured:
/// a renderer that hostile pages can reach is not one.
///
/// A credential-like name that is not valid Unicode cannot be overridden
/// through chromiumoxide's `String` keys, so the launch is refused rather
/// than letting Chrome inherit it. The error names the variable, never its
/// value.
fn blanked_credentials(
    parent: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<Vec<(String, String)>> {
    let names: Vec<OsString> = parent.into_iter().map(|(name, _)| name).collect();
    let found = credential_names(&names);
    if !found.unblankable.is_empty() {
        return Err(BrowserError::Environment(format!(
            "refusing to start Chrome: {} looks like a credential but is not valid Unicode, \
             so it cannot be withheld from Chrome",
            found.unblankable.join(", ")
        )));
    }
    Ok(found
        .blankable
        .into_iter()
        .map(|name| (name, String::new()))
        .collect())
}

/// The launch configuration for one `browser_cdp` call: the sandbox
/// decision and `parent`'s credentials blanked. `headless` keeps its
/// existing effect on chromiumoxide's default arguments and nothing else.
pub(crate) fn launch_config(
    headless: bool,
    sandbox: Sandbox,
    parent: impl IntoIterator<Item = (OsString, OsString)>,
) -> Result<BrowserConfigBuilder> {
    let mut config = BrowserConfig::builder().envs(blanked_credentials(parent)?);
    if headless {
        config = config.disable_default_args();
    }
    if sandbox != Sandbox::Enabled {
        config = config.no_sandbox();
    }
    Ok(config)
}

#[cfg(test)]
#[allow(clippy::disallowed_methods)] // tokio::test uses block_on internally
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("BROWS-007")]
    #[test]
    fn sandbox_stays_on_except_for_linux_root_or_explicit_opt_out() {
        let opt_out = Some(OsStr::new("1"));
        assert_eq!(decide("macos", Some(0), opt_out), Sandbox::Enabled);
        assert_eq!(decide("windows", None, opt_out), Sandbox::Enabled);
        assert_eq!(decide("linux", Some(1000), None), Sandbox::Enabled);
        assert_eq!(decide("linux", None, None), Sandbox::Enabled);
        assert_eq!(
            decide("linux", Some(1000), Some(OsStr::new("true"))),
            Sandbox::Enabled
        );
        assert_eq!(
            decide("linux", Some(0), None),
            Sandbox::DisabledRunningAsRoot
        );
        assert_eq!(
            decide("linux", Some(1000), opt_out),
            Sandbox::DisabledByOperator
        );
    }

    #[spec("BROWS-007")]
    #[test]
    fn effective_uid_is_the_second_uid_field() {
        let status = "Name:\tarkavo\nUid:\t1000\t0\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\n";
        assert_eq!(parse_effective_uid(status), Some(0));
        assert_eq!(parse_effective_uid("Name:\tarkavo\n"), None);
    }

    #[spec("BROWS-007")]
    #[test]
    fn a_malformed_uid_line_keeps_the_sandbox_on() {
        for status in [
            "Uid:\t1000\n",
            "Uid:\n",
            "Uid:\t1000\troot\t1000\t1000\n",
            "Uid:\t1000\t-1\t1000\t1000\n",
            "Uid:\t1000\t4294967296\t1000\t1000\n",
        ] {
            assert_eq!(parse_effective_uid(status), None, "{status:?}");
        }
        assert_eq!(
            parse_effective_uid("Uid:\t1000\t4294967295\t1000\t1000\n"),
            Some(u32::MAX)
        );
    }

    #[test]
    fn the_opt_out_is_ignored_only_outside_linux() {
        let set = Some(OsStr::new("1"));
        assert!(opt_out_is_ignored("macos", set));
        assert!(opt_out_is_ignored("windows", set));
        assert!(!opt_out_is_ignored("linux", set));
        assert!(!opt_out_is_ignored("macos", None));
    }

    /// Launches `/bin/sh` in Chrome's place with the configuration under
    /// test and returns the arguments and environment it was started with.
    /// `sh -c` takes the script as an argument, so no freshly written file
    /// is executed.
    #[cfg(unix)]
    async fn fake_chrome_launch(config: BrowserConfigBuilder) -> (Vec<String>, Vec<String>) {
        let dir = tempfile::tempdir().expect("tempdir");
        let args_out = dir.path().join("args");
        let env_out = dir.path().join("env");
        let script = format!(
            "printf '%s\\n' \"$@\" > '{}'; env > '{}'",
            args_out.display(),
            env_out.display()
        );
        let config = config
            .chrome_executable("/bin/sh")
            .args(["-c", script.as_str(), "fake-chrome"])
            .build()
            .expect("build config");
        let mut child = config.launch().expect("spawn fake chrome");
        child.wait().await.expect("fake chrome exits");
        let lines = |path: &std::path::Path| {
            std::fs::read_to_string(path)
                .expect("fake chrome output")
                .lines()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        (lines(&args_out), lines(&env_out))
    }

    #[spec("BROWS-007")]
    #[cfg(unix)]
    #[tokio::test]
    async fn headless_launch_keeps_the_sandbox() {
        let config = launch_config(true, Sandbox::Enabled, Vec::new()).expect("config");
        let (args, _) = fake_chrome_launch(config).await;
        assert!(args.iter().any(|a| a == "--headless"), "{args:?}");
        assert!(!args.iter().any(|a| a == "--no-sandbox"), "{args:?}");
    }

    #[spec("BROWS-007")]
    #[cfg(unix)]
    #[tokio::test]
    async fn root_launch_disables_the_sandbox() {
        let config =
            launch_config(true, Sandbox::DisabledRunningAsRoot, Vec::new()).expect("config");
        let (args, _) = fake_chrome_launch(config).await;
        assert!(args.iter().any(|a| a == "--no-sandbox"), "{args:?}");
    }

    fn var(name: &str, value: &str) -> (OsString, OsString) {
        (OsString::from(name), OsString::from(value))
    }

    /// Goes through `launch_config`, the only path `execute_browser` takes,
    /// so the credentials it blanks are the ones Chrome is started without.
    #[spec("BROWS-008")]
    #[cfg(unix)]
    #[tokio::test]
    async fn chrome_gets_agent_credentials_blanked() {
        let parent = vec![
            var("OPENAI_API_KEY", "planted-secret"),
            var("RUSTFLAGS", "-Dwarnings"),
        ];
        let config = launch_config(true, Sandbox::Enabled, parent).expect("config");
        let (_, seen) = fake_chrome_launch(config).await;
        assert!(seen.iter().any(|l| l == "OPENAI_API_KEY="), "{seen:?}");
        assert!(!seen.iter().any(|l| l.contains("planted-secret")));
    }

    #[spec("BROWS-008")]
    #[test]
    fn only_credentials_are_blanked_each_under_its_own_spelling() {
        let parent = vec![
            var("OPENAI_API_KEY", "planted-secret"),
            var("Openai_Api_Key", "planted-secret"),
            var("RUSTFLAGS", "-Dwarnings"),
        ];
        assert_eq!(
            blanked_credentials(parent).expect("blanked"),
            vec![
                ("OPENAI_API_KEY".to_string(), String::new()),
                ("Openai_Api_Key".to_string(), String::new()),
            ]
        );
    }

    /// A provider's `auth_ref` may name any variable, so the names the
    /// process registered are blanked as well as the credential-shaped ones.
    #[spec("BROWS-008")]
    #[test]
    fn registered_credential_names_are_blanked_too() {
        // The registry is process-global and tests run in parallel: this
        // name is used by no other test.
        const REGISTERED: &str = "BROWSER_TEST_REGISTERED_LOGIN";
        arkavo_process_env::withhold_name(REGISTERED);
        let parent = vec![var(REGISTERED, "login"), var("HOME", "/home/agent")];
        assert_eq!(
            blanked_credentials(parent).expect("blanked"),
            vec![(REGISTERED.to_string(), String::new())]
        );
    }

    /// The operator's tool-program grant does not readmit a credential into
    /// Chrome.
    #[spec("BROWS-008")]
    #[test]
    fn the_tool_passthrough_grant_does_not_reach_chrome() {
        let parent = vec![
            var("ARKAVO_TOOL_ENV_PASSTHROUGH", "OPENAI_API_KEY"),
            var("OPENAI_API_KEY", "planted-secret"),
        ];
        assert_eq!(
            blanked_credentials(parent).expect("blanked"),
            vec![("OPENAI_API_KEY".to_string(), String::new())]
        );
    }

    #[spec("BROWS-008")]
    #[cfg(unix)]
    #[test]
    fn a_non_unicode_credential_name_refuses_the_launch_without_its_value() {
        use std::os::unix::ffi::OsStringExt;
        let credential = OsString::from_vec(b"MY_\xff_TOKEN".to_vec());
        let parent = vec![(credential, OsString::from("planted-secret"))];
        let Err(refusal) = launch_config(true, Sandbox::Enabled, parent) else {
            panic!("a non-Unicode credential name must refuse the launch");
        };
        let refusal = refusal.to_string();
        assert!(refusal.contains("MY_\u{fffd}_TOKEN"), "{refusal}");
        assert!(!refusal.contains("planted-secret"), "{refusal}");
    }

    #[spec("BROWS-008")]
    #[cfg(unix)]
    #[test]
    fn a_non_unicode_name_that_is_no_credential_does_not_block() {
        use std::os::unix::ffi::OsStringExt;
        let plain = OsString::from_vec(b"PLAIN_\xff_NAME".to_vec());
        let parent = vec![(plain, OsString::from("x")), var("OPENAI_API_KEY", "s")];
        assert_eq!(
            blanked_credentials(parent).expect("blanked"),
            vec![("OPENAI_API_KEY".to_string(), String::new())]
        );
    }
}
