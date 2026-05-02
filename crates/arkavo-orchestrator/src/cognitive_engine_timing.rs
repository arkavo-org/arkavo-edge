//! R4: per-step and per-command execution timeouts.
//!
//! A single `do_step` call dispatches N command-level LLM invocations.
//! Wrapping both the step and each command in a timeout bounds wall-clock
//! work and prevents zombie work on a hung provider. Defaults are sane;
//! both are overridable via env var so operators can tune for slow
//! local-only setups or aggressive cloud SLAs without a rebuild.

use std::time::Duration;
use tracing::warn;

/// Default per-step wall-clock budget. Overridable via `ARKAVO_STEP_TIMEOUT_SECS`.
pub const DEFAULT_STEP_EXECUTION_TIMEOUT_SECS: u64 = 300;

/// Default per-command wall-clock budget. Overridable via `ARKAVO_COMMAND_TIMEOUT_SECS`.
pub const DEFAULT_COMMAND_EXECUTION_TIMEOUT_SECS: u64 = 120;

/// Read a Duration from an env var, clamped to a sane range. On parse
/// failure or out-of-range, the default is used and a warning is logged.
fn env_duration_secs(var: &str, default_secs: u64, min: u64, max: u64) -> Duration {
    match std::env::var(var) {
        Ok(val) => match val.parse::<u64>() {
            Ok(n) if n >= min && n <= max => Duration::from_secs(n),
            _ => {
                warn!(
                    var = %var,
                    value = %val,
                    default = default_secs,
                    "invalid env duration; using default"
                );
                Duration::from_secs(default_secs)
            }
        },
        Err(_) => Duration::from_secs(default_secs),
    }
}

pub fn step_execution_timeout() -> Duration {
    env_duration_secs(
        "ARKAVO_STEP_TIMEOUT_SECS",
        DEFAULT_STEP_EXECUTION_TIMEOUT_SECS,
        5,
        3600,
    )
}

pub fn command_execution_timeout() -> Duration {
    env_duration_secs(
        "ARKAVO_COMMAND_TIMEOUT_SECS",
        DEFAULT_COMMAND_EXECUTION_TIMEOUT_SECS,
        5,
        1800,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_env_unset() {
        // Use unique var names to avoid collision with global env in CI.
        let d = env_duration_secs("ARKAVO_TEST_NEVER_SET_VAR_QXZ", 42, 1, 100);
        assert_eq!(d, Duration::from_secs(42));
    }

    #[test]
    fn clamps_out_of_range() {
        // SAFETY: tests are single-threaded for this module by default.
        unsafe { std::env::set_var("ARKAVO_TEST_RANGE_VAR", "999999") };
        let d = env_duration_secs("ARKAVO_TEST_RANGE_VAR", 7, 1, 100);
        assert_eq!(d, Duration::from_secs(7));
        unsafe { std::env::remove_var("ARKAVO_TEST_RANGE_VAR") };
    }

    #[test]
    fn rejects_garbage() {
        unsafe { std::env::set_var("ARKAVO_TEST_GARBAGE_VAR", "not-a-number") };
        let d = env_duration_secs("ARKAVO_TEST_GARBAGE_VAR", 9, 1, 100);
        assert_eq!(d, Duration::from_secs(9));
        unsafe { std::env::remove_var("ARKAVO_TEST_GARBAGE_VAR") };
    }

    #[test]
    fn accepts_in_range() {
        unsafe { std::env::set_var("ARKAVO_TEST_OK_VAR", "55") };
        let d = env_duration_secs("ARKAVO_TEST_OK_VAR", 1, 1, 100);
        assert_eq!(d, Duration::from_secs(55));
        unsafe { std::env::remove_var("ARKAVO_TEST_OK_VAR") };
    }
}
