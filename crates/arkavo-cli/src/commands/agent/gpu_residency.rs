//! Gives an idle agent's GPU memory back so its peers on the machine can run.
//!
//! llama.cpp's Metal backend keeps a model's buffers wired for three minutes
//! after the last inference. Agents that load the same model share one copy
//! of the weights, but the system counts wired GPU memory per process. Three
//! idle agents holding a 7 GB model therefore exhaust the GPU limit of a
//! 32 GB machine, and the next inference in any of them fails with an
//! out-of-memory fault. A swarm runs one process per role, so an agent
//! releases its GPU memory between inferences unless the operator has chosen
//! a residency setting.

use std::ffi::OsStr;

/// llama.cpp turns residency sets off when this is set to any value.
const NO_RESIDENCY: &str = "GGML_METAL_NO_RESIDENCY";

/// llama.cpp keeps buffers wired for this many seconds after the last use.
const KEEP_ALIVE_SECS: &str = "GGML_METAL_RESIDENCY_KEEP_ALIVE_S";

/// Whether the agent turns residency sets off: only where Metal is the
/// backend, and only when the operator has left both settings alone.
fn releases_when_idle(
    metal_backend: bool,
    no_residency: Option<&OsStr>,
    keep_alive: Option<&OsStr>,
) -> bool {
    metal_backend && no_residency.is_none() && keep_alive.is_none()
}

/// Turns residency sets off for this process unless the operator set either
/// variable. Must run before the process starts a thread.
pub(super) fn release_gpu_memory_when_idle() {
    let no_residency = std::env::var_os(NO_RESIDENCY);
    let keep_alive = std::env::var_os(KEEP_ALIVE_SECS);
    if releases_when_idle(
        cfg!(target_os = "macos"),
        no_residency.as_deref(),
        keep_alive.as_deref(),
    ) {
        // SAFETY: called once from the single-threaded CLI startup path
        // (`run_agent_with_options`), before the runtime and the llama.cpp
        // backend, which reads this variable when it opens the device.
        unsafe {
            std::env::set_var(NO_RESIDENCY, "1");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_on_metal_releases_gpu_memory_by_default() {
        assert!(releases_when_idle(true, None, None));
    }

    #[test]
    fn an_operator_keep_alive_is_left_in_force() {
        assert!(!releases_when_idle(true, None, Some(OsStr::new("180"))));
    }

    #[test]
    fn an_operator_who_already_turned_residency_off_is_left_alone() {
        assert!(!releases_when_idle(true, Some(OsStr::new("1")), None));
    }

    #[test]
    fn a_platform_without_metal_is_left_alone() {
        assert!(!releases_when_idle(false, None, None));
    }
}
