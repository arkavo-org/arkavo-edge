//! Commands for the programs built-in tools run on the operator's behalf.
//!
//! These are the operator's own toolchains (`cargo`, `gh`, `semgrep`,
//! `docker`), so they get [`ChildEnv::tool_from_current`]: the operator's
//! environment — build settings are open-ended — minus every credential the
//! agent holds, which a hostile repository's build script or test could
//! otherwise read and send on. Without it, a test run by `test_runner` reads
//! every provider key from its environment.

// `pub(crate)` is the real, intended visibility here (the module is private,
// so nothing leaks past the crate either way); `redundant_pub_crate` wants
// `pub`, which `unreachable_pub` then rejects.
#![allow(clippy::redundant_pub_crate)]

use arkavo_git::remote_fallback::GITHUB_CREDENTIALS;
use arkavo_process_env::ChildEnv;

/// A blocking command for `program` under the tool environment.
pub(crate) fn tool_command(program: &str) -> std::process::Command {
    ChildEnv::tool_from_current(&[]).command(program)
}

/// [`tool_command`] for tokio callers.
pub(crate) fn async_tool_command(program: &str) -> tokio::process::Command {
    tool_command(program).into()
}

/// `gh`, which additionally keeps the GitHub token the operator exported.
pub(crate) fn gh_command() -> std::process::Command {
    ChildEnv::tool_from_current(GITHUB_CREDENTIALS).command("gh")
}

/// Re-running one test of this binary in a fresh process whose real
/// environment holds planted credentials, so a test observes what the
/// production spawn path hands a child without `set_var`.
#[cfg(test)]
pub(crate) mod probe {
    use std::path::Path;
    use std::process::Output;

    /// Set only on the re-run process; names the directory the probe works
    /// in. The probe half of a test returns at once when it is unset.
    pub(crate) const PROBE_DIR: &str = "ARKAVO_TOOL_ENV_PROBE_DIR";
    /// The planted provider keys' value; it must never reach a tool child.
    pub(crate) const PLANTED: &str = "planted-provider-key";
    /// A planted setting that is not a credential, proving the dump ran.
    pub(crate) const KEPT_LINE: &str = "ARKAVO_PROBE_SETTING=kept";

    /// Runs the test named `test` in a fresh copy of this test binary, in
    /// `dir`, with `dir/bin` first on `PATH` and provider keys planted.
    /// Panics unless exactly that test ran and passed.
    pub(crate) fn rerun(test: &str, dir: &Path) -> Output {
        let path = std::env::join_paths(std::iter::once(dir.join("bin")).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .expect("PATH");
        let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
            .args(["--exact", test, "--nocapture"])
            .current_dir(dir)
            .env_remove(arkavo_process_env::TOOL_ENV_PASSTHROUGH)
            .env(PROBE_DIR, dir)
            .env("PATH", path)
            .env("OPENAI_API_KEY", PLANTED)
            .env("ANTHROPIC_API_KEY", PLANTED)
            .env("ARKAVO_PROBE_SETTING", "kept")
            .output()
            .expect("re-run test binary");
        let log = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && log.contains("1 passed"),
            "{log}{}",
            String::from_utf8_lossy(&output.stderr)
        );
        output
    }

    /// Writes an executable `dir/bin/<name>` that records its environment
    /// in `<name>-env.txt` in its working directory. Called in the re-run
    /// process, where no other test forks while the file is open.
    #[cfg(unix)]
    pub(crate) fn fake_program(dir: &Path, name: &str) {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("bin");
        std::fs::create_dir_all(&bin).expect("bin dir");
        let program = bin.join(name);
        std::fs::write(&program, format!("#!/bin/sh\nenv > {name}-env.txt\n"))
            .expect("fake program");
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
            .expect("make fake program executable");
    }
}
