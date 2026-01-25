use crate::error::{Result, WalletError};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha3::{Digest, Keccak256};
use std::fmt;
use std::str::FromStr;

/// EVM address (20 bytes) with EIP-55 checksum support
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct EvmAddress([u8; 20]);

impl Serialize for EvmAddress {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_checksum())
    }
}

impl<'de> Deserialize<'de> for EvmAddress {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

impl EvmAddress {
    /// Create address from raw bytes
    pub fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    /// Create address from a slice (must be exactly 20 bytes)
    pub fn from_slice(slice: &[u8]) -> Result<Self> {
        if slice.len() != 20 {
            return Err(WalletError::InvalidAddress(format!(
                "Expected 20 bytes, got {}",
                slice.len()
            )));
        }
        let mut bytes = [0u8; 20];
        bytes.copy_from_slice(slice);
        Ok(Self(bytes))
    }

    /// Create address from public key bytes (uncompressed, 64 bytes without prefix)
    pub fn from_public_key(pubkey_bytes: &[u8]) -> Result<Self> {
        if pubkey_bytes.len() != 64 {
            return Err(WalletError::InvalidAddress(format!(
                "Expected 64 byte public key, got {}",
                pubkey_bytes.len()
            )));
        }
        let hash = Keccak256::digest(pubkey_bytes);
        let mut addr = [0u8; 20];
        addr.copy_from_slice(&hash[12..32]);
        Ok(Self(addr))
    }

    /// Get raw bytes
    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    /// Convert to hex string without 0x prefix
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Convert to EIP-55 checksummed address string (with 0x prefix)
    pub fn to_checksum(&self) -> String {
        let hex_addr = self.to_hex();
        let hash = Keccak256::digest(hex_addr.as_bytes());

        let mut checksummed = String::with_capacity(42);
        checksummed.push_str("0x");

        for (i, c) in hex_addr.chars().enumerate() {
            let nibble = (hash[i / 2] >> (if i % 2 == 0 { 4 } else { 0 })) & 0xf;
            if c.is_ascii_alphabetic() && nibble >= 8 {
                checksummed.push(c.to_ascii_uppercase());
            } else {
                checksummed.push(c);
            }
        }

        checksummed
    }

    /// Parse from hex string (with or without 0x prefix)
    pub fn from_hex(s: &str) -> Result<Self> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        if s.len() != 40 {
            return Err(WalletError::InvalidAddress(format!(
                "Expected 40 hex characters, got {}",
                s.len()
            )));
        }
        let bytes =
            hex::decode(s).map_err(|e| WalletError::InvalidAddress(format!("Invalid hex: {e}")))?;
        Self::from_slice(&bytes)
    }

    /// Verify EIP-55 checksum
    pub fn verify_checksum(s: &str) -> bool {
        match Self::from_hex(s) {
            Ok(addr) => addr.to_checksum() == s,
            Err(_) => false,
        }
    }
}

impl fmt::Display for EvmAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_checksum())
    }
}

impl fmt::Debug for EvmAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EvmAddress({})", self.to_checksum())
    }
}

impl FromStr for EvmAddress {
    type Err = WalletError;

    fn from_str(s: &str) -> Result<Self> {
        Self::from_hex(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_from_bytes() {
        let bytes = [0u8; 20];
        let addr = EvmAddress::from_bytes(bytes);
        assert_eq!(addr.as_bytes(), &bytes);
    }

    #[test]
    fn test_address_from_slice_valid() {
        let bytes = [1u8; 20];
        let addr = EvmAddress::from_slice(&bytes).unwrap();
        assert_eq!(addr.as_bytes(), &bytes);
    }

    #[test]
    fn test_address_from_slice_invalid() {
        let bytes = [1u8; 19];
        assert!(EvmAddress::from_slice(&bytes).is_err());
    }

    #[test]
    fn test_address_checksum() {
        let addr = EvmAddress::from_hex("5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed").unwrap();
        assert_eq!(
            addr.to_checksum(),
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"
        );
    }

    #[test]
    fn test_address_parse_with_prefix() {
        let addr = EvmAddress::from_hex("0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed").unwrap();
        assert!(addr.to_checksum().starts_with("0x"));
    }

    #[test]
    fn test_verify_checksum() {
        assert!(EvmAddress::verify_checksum(
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed"
        ));
        assert!(!EvmAddress::verify_checksum(
            "0x5aaeb6053f3e94c9b9a09f33669435e7ef1beaed"
        ));
    }

    #[test]
    fn test_address_display() {
        let addr = EvmAddress::from_bytes([0xab; 20]);
        let display = format!("{addr}");
        assert!(display.starts_with("0x"));
        assert_eq!(display.len(), 42);
    }
}
