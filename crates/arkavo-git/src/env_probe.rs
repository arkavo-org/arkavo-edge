//! Re-running one test of this binary in a fresh process whose real
//! environment holds a planted provider key, so a test observes what a
//! production spawn path hands a child without `set_var`.

// `pub(crate)` is the real, intended visibility here (the module is private,
// so nothing leaks past the crate either way); `redundant_pub_crate` wants
// `pub`, which `unreachable_pub` then rejects.
#![allow(clippy::redundant_pub_crate)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Set only on the re-run process; names the directory the probe works in.
const PROBE_DIR: &str = "ARKAVO_GIT_ENV_PROBE_DIR";
/// The planted provider key's value; it must never reach a tool child.
pub(crate) const PLANTED: &str = "planted-provider-key";
/// A planted setting that is not a credential, proving the program ran.
pub(crate) const KEPT_LINE: &str = "ARKAVO_PROBE_SETTING=kept";

/// The probe directory when this is the re-run process. The probe half of a
/// test returns at once when this is `None`.
pub(crate) fn probe_dir() -> Option<PathBuf> {
    std::env::var_os(PROBE_DIR).map(PathBuf::from)
}

/// Writes an executable `dir/bin/<name>` that records its environment in
/// `<name>-env.txt` in its working directory. Called in the re-run process,
/// where no other test forks while the file is open.
pub(crate) fn fake_program(dir: &Path, name: &str) {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).expect("bin dir");
    let program = bin.join(name);
    std::fs::write(&program, format!("#!/bin/sh\nenv > {name}-env.txt\n")).expect("fake program");
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755))
        .expect("make fake program executable");
}

/// Runs the test named `test` in a fresh copy of this test binary, in
/// `dir`, with `dir/bin` first on `PATH`, `OPENAI_API_KEY` planted and
/// `extra` set, then returns what the fake `program` recorded. Panics
/// unless exactly that test ran and passed.
pub(crate) fn rerun(test: &str, dir: &Path, extra: &[(&str, &str)], program: &str) -> String {
    let path = std::env::join_paths(
        std::iter::once(dir.join("bin")).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )),
    )
    .expect("PATH");
    let output = Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", test, "--nocapture"])
        .current_dir(dir)
        .env_remove(arkavo_process_env::TOOL_ENV_PASSTHROUGH)
        .env(PROBE_DIR, dir)
        .env("PATH", path)
        .env("OPENAI_API_KEY", PLANTED)
        .env("ARKAVO_PROBE_SETTING", "kept")
        .envs(extra.iter().copied())
        .output()
        .expect("re-run test binary");
    let log = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && log.contains("1 passed"),
        "{log}{}",
        String::from_utf8_lossy(&output.stderr)
    );
    std::fs::read_to_string(dir.join(format!("{program}-env.txt"))).expect("the program ran")
}
