//! Plugin/tool integrity verification using SHA-256 hash manifests
//!
//! Verifies tool content against a trusted hash registry before registration.
//! Supports enforce and warn-only modes for gradual adoption.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Result of an integrity verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrityStatus {
    /// Hash matches the registry entry.
    Verified,
    /// Hash does not match the registry entry.
    HashMismatch { expected: String, actual: String },
    /// Tool is not in the allowlist.
    NotInAllowlist,
    /// No hash registered for this tool.
    MissingHash,
}

/// Enforcement mode for integrity checks.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EnforcementMode {
    /// Block tools that fail verification.
    Enforce,
    /// Log warnings but allow tools to load.
    #[default]
    WarnOnly,
}

/// Registry entry for a single tool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolIntegrityEntry {
    pub tool_name: String,
    pub sha256: String,
    pub version: Option<String>,
    pub source: Option<String>,
}

/// Registry of tool integrity hashes.
///
/// Loaded from `~/.arkavo/tool-integrity.toml` or configured programmatically.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntegrityRegistry {
    pub mode: EnforcementMode,
    #[serde(default)]
    pub entries: HashMap<String, ToolIntegrityEntry>,
}

impl IntegrityRegistry {
    /// Create an empty registry with default mode.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a registry with enforcement mode.
    #[must_use]
    pub fn with_mode(mode: EnforcementMode) -> Self {
        Self {
            mode,
            entries: HashMap::new(),
        }
    }

    /// Register a tool's expected hash.
    pub fn register(&mut self, tool_name: impl Into<String>, sha256: impl Into<String>) {
        let name = tool_name.into();
        self.entries.insert(
            name.clone(),
            ToolIntegrityEntry {
                tool_name: name,
                sha256: sha256.into(),
                version: None,
                source: None,
            },
        );
    }

    /// Register a tool with full metadata.
    pub fn register_entry(&mut self, entry: ToolIntegrityEntry) {
        self.entries.insert(entry.tool_name.clone(), entry);
    }

    /// Verify a tool's integrity by computing its SHA-256 hash.
    pub fn verify(&self, tool_name: &str, data: &[u8]) -> IntegrityStatus {
        let entry = match self.entries.get(tool_name) {
            Some(e) => e,
            None => {
                // If no entries at all, tools are not managed
                if self.entries.is_empty() {
                    return IntegrityStatus::MissingHash;
                }
                return IntegrityStatus::NotInAllowlist;
            }
        };

        let actual_hash = compute_sha256(data);
        if actual_hash == entry.sha256 {
            IntegrityStatus::Verified
        } else {
            IntegrityStatus::HashMismatch {
                expected: entry.sha256.clone(),
                actual: actual_hash,
            }
        }
    }

    /// Check if the verification result should block tool loading.
    pub fn should_block(&self, status: &IntegrityStatus) -> bool {
        match self.mode {
            EnforcementMode::Enforce => !matches!(
                status,
                IntegrityStatus::Verified | IntegrityStatus::MissingHash
            ),
            EnforcementMode::WarnOnly => false,
        }
    }

    /// Load from a TOML string.
    pub fn from_toml(toml_str: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_str)
    }

    /// Serialize to a TOML string.
    pub fn to_toml(&self) -> Result<String, toml::ser::Error> {
        toml::to_string_pretty(self)
    }
}

/// Compute SHA-256 hash of data, returning lowercase hex string.
fn compute_sha256(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    use std::fmt::Write;
    result.iter().fold(String::with_capacity(64), |mut acc, b| {
        let _ = write!(acc, "{b:02x}");
        acc
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    #[spec("MCPD-004")]
    #[test]
    fn test_verified() {
        let mut registry = IntegrityRegistry::new();
        let data = b"tool content";
        let hash = compute_sha256(data);
        registry.register("my_tool", &hash);

        assert_eq!(registry.verify("my_tool", data), IntegrityStatus::Verified);
    }

    #[spec("MCPD-004")]
    #[test]
    fn test_hash_mismatch() {
        let mut registry = IntegrityRegistry::new();
        registry.register("my_tool", "deadbeef");

        let status = registry.verify("my_tool", b"different content");
        assert!(matches!(status, IntegrityStatus::HashMismatch { .. }));
    }

    #[spec("MCPD-004")]
    #[test]
    fn test_not_in_allowlist() {
        let mut registry = IntegrityRegistry::new();
        registry.register("allowed_tool", "abc123");

        let status = registry.verify("unknown_tool", b"data");
        assert_eq!(status, IntegrityStatus::NotInAllowlist);
    }

    #[spec("MCPD-004")]
    #[test]
    fn test_missing_hash_empty_registry() {
        let registry = IntegrityRegistry::new();
        let status = registry.verify("any_tool", b"data");
        assert_eq!(status, IntegrityStatus::MissingHash);
    }

    #[spec("MCPD-004")]
    #[test]
    fn test_enforce_mode_blocks() {
        let registry = IntegrityRegistry::with_mode(EnforcementMode::Enforce);
        assert!(registry.should_block(&IntegrityStatus::NotInAllowlist));
        assert!(registry.should_block(&IntegrityStatus::HashMismatch {
            expected: "a".into(),
            actual: "b".into(),
        }));
        assert!(!registry.should_block(&IntegrityStatus::Verified));
        assert!(!registry.should_block(&IntegrityStatus::MissingHash));
    }

    #[spec("MCPD-004")]
    #[test]
    fn test_warn_mode_never_blocks() {
        let registry = IntegrityRegistry::with_mode(EnforcementMode::WarnOnly);
        assert!(!registry.should_block(&IntegrityStatus::NotInAllowlist));
        assert!(!registry.should_block(&IntegrityStatus::HashMismatch {
            expected: "a".into(),
            actual: "b".into(),
        }));
    }

    #[spec("MCPD-004")]
    #[test]
    fn test_sha256_deterministic() {
        let hash1 = compute_sha256(b"hello");
        let hash2 = compute_sha256(b"hello");
        assert_eq!(hash1, hash2);
        // Known SHA-256 of "hello"
        assert_eq!(
            hash1,
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }

    #[spec("MCPD-004")]
    #[test]
    fn test_toml_roundtrip() {
        let mut registry = IntegrityRegistry::with_mode(EnforcementMode::Enforce);
        registry.register("test_tool", "abc123def456");
        let toml = registry.to_toml().unwrap();
        let parsed = IntegrityRegistry::from_toml(&toml).unwrap();
        assert_eq!(parsed.mode, EnforcementMode::Enforce);
        assert!(parsed.entries.contains_key("test_tool"));
    }
}
