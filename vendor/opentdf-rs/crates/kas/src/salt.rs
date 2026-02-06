//! NanoTDF salt computation and version detection
//!
//! Per NanoTDF spec section 4: salt = SHA256(MAGIC_NUMBER + VERSION)

use sha2::{Digest, Sha256};

/// NanoTDF magic number prefix
pub const NANOTDF_MAGIC: &[u8] = b"L1";

/// NanoTDF version enumeration
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NanoTdfVersion {
    /// Version 1.2 - "L1L" header
    V12,
    /// Version 1.3 - "L1M" header
    V13,
}

impl NanoTdfVersion {
    /// Version byte for NanoTDF v1.2
    pub const V12_BYTE: u8 = 0x4C; // 'L'
    /// Version byte for NanoTDF v1.3
    pub const V13_BYTE: u8 = 0x4D; // 'M'

    /// Get the version byte
    pub fn byte(self) -> u8 {
        match self {
            NanoTdfVersion::V12 => Self::V12_BYTE,
            NanoTdfVersion::V13 => Self::V13_BYTE,
        }
    }

    /// Create from version byte
    pub fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            Self::V12_BYTE => Some(NanoTdfVersion::V12),
            Self::V13_BYTE => Some(NanoTdfVersion::V13),
            _ => None,
        }
    }
}

/// Computes the HKDF salt for a given NanoTDF version
///
/// Per NanoTDF spec section 4: salt = SHA256(MAGIC_NUMBER + VERSION)
pub fn compute_nanotdf_salt(version: NanoTdfVersion) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(NANOTDF_MAGIC);
    hasher.update([version.byte()]);
    hasher.finalize().into()
}

/// Computes salt from raw version byte
pub fn compute_nanotdf_salt_from_byte(version: u8) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(NANOTDF_MAGIC);
    hasher.update([version]);
    hasher.finalize().into()
}

/// Detects NanoTDF version from header magic bytes
///
/// Returns the version if valid NanoTDF header, None otherwise
pub fn detect_nanotdf_version(header: &[u8]) -> Option<NanoTdfVersion> {
    if header.len() < 3 {
        return None;
    }
    // Check magic number "L1"
    if &header[0..2] != NANOTDF_MAGIC {
        return None;
    }
    NanoTdfVersion::from_byte(header[2])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_nanotdf_salt_v12() {
        let salt = compute_nanotdf_salt(NanoTdfVersion::V12);
        // The salt should be SHA256("L1L")
        let expected = Sha256::digest(b"L1L");
        assert_eq!(salt.as_ref(), expected.as_slice());
    }

    #[test]
    fn test_compute_nanotdf_salt_v13() {
        let salt = compute_nanotdf_salt(NanoTdfVersion::V13);
        // The salt should be SHA256("L1M")
        let expected = Sha256::digest(b"L1M");
        assert_eq!(salt.as_ref(), expected.as_slice());
    }

    #[test]
    fn test_detect_nanotdf_version() {
        assert_eq!(detect_nanotdf_version(b"L1L"), Some(NanoTdfVersion::V12));
        assert_eq!(detect_nanotdf_version(b"L1M"), Some(NanoTdfVersion::V13));
        assert_eq!(detect_nanotdf_version(b"XXL"), None);
        assert_eq!(detect_nanotdf_version(b"L1"), None);
        assert_eq!(detect_nanotdf_version(b"L1Z"), None);
    }

    #[test]
    fn test_version_roundtrip() {
        assert_eq!(
            NanoTdfVersion::from_byte(NanoTdfVersion::V12.byte()),
            Some(NanoTdfVersion::V12)
        );
        assert_eq!(
            NanoTdfVersion::from_byte(NanoTdfVersion::V13.byte()),
            Some(NanoTdfVersion::V13)
        );
    }
}
