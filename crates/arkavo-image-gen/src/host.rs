//! Whether this host can generate images, and how much memory it has for
//! them (IMG-001).

const GIB: u64 = 1 << 30;

/// Memory left to the OS and everything outside the agent.
pub const OS_RESERVE: u64 = 4 * GIB;

/// What kind of machine the agent runs on, as far as generation cares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    /// macOS on Apple Silicon running a native aarch64 binary: Metal.
    AppleSilicon,
    /// An x86_64 binary under Rosetta on Apple Silicon.
    Translated,
    /// Anything else; no GPU backend is specified for it.
    Other,
}

/// The host as the generator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub platform: Platform,
    /// For example "Apple M6".
    pub chip: Option<String>,
    pub memory_bytes: u64,
}

impl Host {
    /// Probe the machine this process runs on.
    pub fn detect() -> Self {
        Self {
            platform: detect_platform(),
            chip: sysctl("machdep.cpu.brand_string"),
            memory_bytes: sysctl("hw.memsize")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0),
        }
    }

    pub fn qualifies(&self) -> bool {
        self.platform == Platform::AppleSilicon
    }

    /// Unified memory minus the chat model the agent keeps resident minus
    /// the OS reserve.
    pub const fn budget(&self, resident_chat_bytes: u64) -> u64 {
        self.memory_bytes
            .saturating_sub(resident_chat_bytes)
            .saturating_sub(OS_RESERVE)
    }
}

fn detect_platform() -> Platform {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Platform::AppleSilicon
    } else if cfg!(target_os = "macos") && sysctl("sysctl.proc_translated").as_deref() == Some("1")
    {
        Platform::Translated
    } else {
        Platform::Other
    }
}

/// Read one sysctl value; `None` off macOS or when the key is missing.
fn sysctl(key: &str) -> Option<String> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let output = std::process::Command::new("/usr/sbin/sysctl")
        .args(["-n", key])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mac(gib: u64) -> Host {
        Host {
            platform: Platform::AppleSilicon,
            chip: Some("Apple M6".to_string()),
            memory_bytes: gib * GIB,
        }
    }

    /// The IMG-001 examples: a 32 GiB M6 with Gemma 4 12B resident.
    #[test]
    fn budget_subtracts_chat_model_and_os() {
        let budget = mac(32).budget(7_200_000_000);
        assert_eq!(budget, 32 * GIB - 7_200_000_000 - 4 * GIB);
        assert!((21.2..21.4).contains(&(budget as f64 / GIB as f64)));
    }

    #[test]
    fn budget_never_underflows() {
        assert_eq!(mac(4).budget(7_200_000_000), 0);
    }

    #[test]
    fn only_native_apple_silicon_qualifies() {
        assert!(mac(32).qualifies());
        for platform in [Platform::Translated, Platform::Other] {
            let host = Host {
                platform,
                ..mac(32)
            };
            assert!(!host.qualifies());
        }
    }

    #[test]
    fn detection_matches_the_build_target() {
        let host = Host::detect();
        if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
            assert!(host.qualifies());
            assert!(host.memory_bytes > 0);
        } else {
            assert!(!host.qualifies());
        }
        if !cfg!(target_os = "macos") {
            assert_eq!(host.chip, None);
            assert_eq!(host.memory_bytes, 0);
        }
    }
}
