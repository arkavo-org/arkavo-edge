//! Where each agent's keys live: one directory per agent name (DEVICE-010).

use crate::{DeviceIdentityError, Result};
use std::path::PathBuf;

pub(crate) const AGENTS_DIR: &str = "agents";

/// The directory holding the keys of the agent named `agent`.
pub(crate) fn agent_dir(agent: &str) -> Result<PathBuf> {
    if agent.is_empty() {
        return Err(DeviceIdentityError::WorkloadKey(
            "an agent needs a name to hold keys".to_string(),
        ));
    }
    Ok(crate::keypair::slot_path(AGENTS_DIR)?.join(dir_name(agent)))
}

/// Names compare without case, as the mDNS instance names agents announce
/// do, and every byte but ASCII letters, digits, `-` and `_` is
/// percent-encoded: no name reaches outside the agents directory, and two
/// names share a directory only when they differ in case alone.
fn dir_name(agent: &str) -> String {
    use std::fmt::Write as _;

    let mut name = String::with_capacity(agent.len());
    for byte in agent.to_lowercase().bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' {
            name.push(char::from(byte));
        } else {
            let _ = write!(name, "%{byte:02x}");
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("DEVICE-010")]
    #[test]
    fn no_name_reaches_outside_the_agents_directory() {
        assert_eq!(dir_name("../escape"), "%2e%2e%2fescape");
        assert_eq!(dir_name("a/b\\c"), "a%2fb%5cc");
        assert_eq!(dir_name("Pauls-Mac_mini"), "pauls-mac_mini");
        assert_eq!(dir_name("caf\u{e9}"), "caf%c3%a9");

        let root = crate::keypair::slot_path(AGENTS_DIR).unwrap();
        assert_eq!(
            agent_dir("../escape").unwrap(),
            root.join("%2e%2e%2fescape")
        );
    }

    #[spec("DEVICE-010")]
    #[test]
    fn an_agent_without_a_name_has_no_directory() {
        let err = agent_dir("").unwrap_err().to_string();
        assert!(err.contains("needs a name"), "{err}");
    }
}
