use crate::error::{Result, WalletError};
use k256::ecdsa::{RecoveryId, Signature as K256Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::fmt;

/// ECDSA signature with recovery ID for EVM compatibility
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Signature {
    r: [u8; 32],
    s: [u8; 32],
    v: u8,
}

impl Signature {
    /// Create signature from r, s components and recovery ID
    pub fn new(r: [u8; 32], s: [u8; 32], recovery_id: u8) -> Self {
        Self { r, s, v: recovery_id }
    }

    /// Create from k256 signature and recovery ID
    pub fn from_k256(sig: &K256Signature, recovery_id: RecoveryId) -> Self {
        let bytes = sig.to_bytes();
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&bytes[..32]);
        s.copy_from_slice(&bytes[32..]);
        Self {
            r,
            s,
            v: recovery_id.to_byte(),
        }
    }

    /// Get r component
    pub fn r(&self) -> &[u8; 32] {
        &self.r
    }

    /// Get s component
    pub fn s(&self) -> &[u8; 32] {
        &self.s
    }

    /// Get recovery ID (v value, 0 or 1)
    pub fn v(&self) -> u8 {
        self.v
    }

    /// Convert to EIP-155 v value for a given chain ID
    pub fn eip155_v(&self, chain_id: u64) -> u64 {
        u64::from(self.v) + 35 + chain_id * 2
    }

    /// Convert to 65-byte format (r || s || v)
    pub fn to_bytes(&self) -> [u8; 65] {
        let mut bytes = [0u8; 65];
        bytes[..32].copy_from_slice(&self.r);
        bytes[32..64].copy_from_slice(&self.s);
        bytes[64] = self.v;
        bytes
    }

    /// Parse from 65-byte format
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() != 65 {
            return Err(WalletError::InvalidSignature);
        }
        let mut r = [0u8; 32];
        let mut s = [0u8; 32];
        r.copy_from_slice(&bytes[..32]);
        s.copy_from_slice(&bytes[32..64]);
        Ok(Self { r, s, v: bytes[64] })
    }

    /// Recover public key from signature and message hash
    pub fn recover_public_key(&self, message_hash: &[u8; 32]) -> Result<VerifyingKey> {
        let sig = K256Signature::from_slice(&[self.r.as_slice(), self.s.as_slice()].concat())
            .map_err(|_| WalletError::InvalidSignature)?;

        let recovery_id = RecoveryId::try_from(self.v)
            .map_err(|_| WalletError::InvalidSignature)?;

        VerifyingKey::recover_from_prehash(message_hash, &sig, recovery_id)
            .map_err(|_| WalletError::VerificationFailed)
    }

    /// Convert to hex string
    pub fn to_hex(&self) -> String {
        hex::encode(self.to_bytes())
    }
}

impl fmt::Debug for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Signature(0x{})", self.to_hex())
    }
}

impl fmt::Display for Signature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "0x{}", self.to_hex())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_signature_roundtrip() {
        let r = [1u8; 32];
        let s = [2u8; 32];
        let sig = Signature::new(r, s, 0);

        let bytes = sig.to_bytes();
        let recovered = Signature::from_bytes(&bytes).unwrap();

        assert_eq!(sig.r(), recovered.r());
        assert_eq!(sig.s(), recovered.s());
        assert_eq!(sig.v(), recovered.v());
    }

    #[test]
    fn test_eip155_v() {
        let sig = Signature::new([0u8; 32], [0u8; 32], 0);
        assert_eq!(sig.eip155_v(1), 37);
        assert_eq!(sig.eip155_v(137), 309);
    }

    #[test]
    fn test_signature_from_invalid_bytes() {
        let bytes = [0u8; 64];
        assert!(Signature::from_bytes(&bytes).is_err());
    }

    #[test]
    fn test_signature_display() {
        let sig = Signature::new([0xab; 32], [0xcd; 32], 1);
        let display = format!("{sig}");
        assert!(display.starts_with("0x"));
        assert_eq!(display.len(), 132);
    }
}
