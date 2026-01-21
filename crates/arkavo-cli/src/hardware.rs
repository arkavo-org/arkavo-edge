//! Hardware detection for system capabilities
//!
//! Detects RAM, unified memory, and calculates optimal model context sizes.

/// Get total system RAM in GB
#[cfg(target_os = "macos")]
pub fn get_total_ram_gb() -> u64 {
    use std::process::Command;
    Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|bytes| bytes / 1_073_741_824) // Convert to GB
        .unwrap_or(8)
}

#[cfg(target_os = "linux")]
pub fn get_total_ram_gb() -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|content| {
            content.lines().find(|l| l.starts_with("MemTotal:")).map(|l| {
                l.split_whitespace()
                    .nth(1)
                    .and_then(|kb| kb.parse::<u64>().ok())
                    .map(|kb| kb / 1_048_576) // kB to GB
                    .unwrap_or(8)
            })
        })
        .unwrap_or(8)
}

#[cfg(windows)]
pub fn get_total_ram_gb() -> u64 {
    // Windows: Use GlobalMemoryStatusEx via winapi
    // For now, return a reasonable default
    32
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
pub fn get_total_ram_gb() -> u64 {
    16
}

/// Detect if system has unified memory (Apple Silicon or dGPU with shared memory)
#[cfg(target_os = "macos")]
pub fn detect_unified_memory() -> bool {
    use std::process::Command;
    Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.contains("Apple"))
        .unwrap_or(false)
}

#[cfg(not(target_os = "macos"))]
pub fn detect_unified_memory() -> bool {
    false
}

/// Calculate max context for GLM-4.7-Flash based on available RAM
///
/// GLM-4.7-Flash is a 30B MoE model requiring ~20GB for Q4_K_M weights.
/// KV cache at 128k context can add 10GB+ in float16.
pub fn calculate_glm_max_context(total_ram_gb: u64, use_kv_quant: bool) -> Option<u32> {
    const MODEL_SIZE_GB: u64 = 20; // Q4_K_M
    const OVERHEAD_GB: u64 = 4;

    let remaining = total_ram_gb.saturating_sub(MODEL_SIZE_GB + OVERHEAD_GB);
    let effective = if use_kv_quant { remaining * 2 } else { remaining };

    match effective {
        0..=3 => None,         // Cannot run GLM
        4..=7 => Some(8_192),  // Minimal (24GB RAM with KV quant)
        8..=15 => Some(16_384),
        16..=30 => Some(32_768),  // 32GB RAM default
        31..=50 => Some(65_536),  // 48GB RAM
        _ => Some(131_072),       // 64GB+ RAM - full context
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ram_detection() {
        let ram = get_total_ram_gb();
        assert!(ram > 0);
    }

    #[test]
    fn test_glm_context_calculation() {
        // 24GB RAM without KV quant - cannot run
        assert_eq!(calculate_glm_max_context(24, false), None);
        // 24GB RAM with KV quant - minimal context
        assert_eq!(calculate_glm_max_context(24, true), None);
        // 28GB RAM with KV quant - small context
        assert_eq!(calculate_glm_max_context(28, true), Some(16_384));
        // 32GB RAM with KV quant - medium context
        assert_eq!(calculate_glm_max_context(32, true), Some(32_768));
        // 64GB RAM without KV quant - large context
        assert_eq!(calculate_glm_max_context(64, false), Some(65_536));
        // 128GB RAM - max context
        assert_eq!(calculate_glm_max_context(128, false), Some(131_072));
    }
}
