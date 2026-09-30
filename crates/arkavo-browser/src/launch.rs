//! How `browser_cdp` starts Chrome: inside Chrome's own sandbox.
//!
//! The sandbox is what confines a renderer compromised by a hostile page;
//! without it that page runs code as the agent's user, with the agent's
//! files and network. It stays on unless Chrome cannot start with it — on
//! Linux Chrome refuses to run sandboxed as root, and a rootless host
//! without user namespaces has no sandbox to give it — and the decision is
//! never taken from tool parameters, which the model writes.

use chromiumoxide::browser::{BrowserConfig, BrowserConfigBuilder};
use std::ffi::OsStr;

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
    let sandbox = decide(std::env::consts::OS, effective_uid(), opt_out.as_deref());
    if sandbox != Sandbox::Enabled {
        tracing::warn!(?sandbox, "launching Chrome without its sandbox");
    }
    sandbox
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

/// The launch configuration for one `browser_cdp` call. `headless` keeps
/// its existing effect on chromiumoxide's default arguments and nothing
/// else.
pub(crate) fn launch_config(headless: bool, sandbox: Sandbox) -> BrowserConfigBuilder {
    let mut config = BrowserConfig::builder();
    if headless {
        config = config.disable_default_args();
    }
    if sandbox != Sandbox::Enabled {
        config = config.no_sandbox();
    }
    config
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
        let (args, _) = fake_chrome_launch(launch_config(true, Sandbox::Enabled)).await;
        assert!(args.iter().any(|a| a == "--headless"), "{args:?}");
        assert!(!args.iter().any(|a| a == "--no-sandbox"), "{args:?}");
    }

    #[spec("BROWS-007")]
    #[cfg(unix)]
    #[tokio::test]
    async fn root_launch_disables_the_sandbox() {
        let config = launch_config(true, Sandbox::DisabledRunningAsRoot);
        let (args, _) = fake_chrome_launch(config).await;
        assert!(args.iter().any(|a| a == "--no-sandbox"), "{args:?}");
    }
}
