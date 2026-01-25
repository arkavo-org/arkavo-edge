use crate::error::{Result, WalletError};
use bip39::Mnemonic as Bip39Mnemonic;
use zeroize::Zeroizing;

/// BIP39 mnemonic phrase wrapper
pub struct Mnemonic {
    inner: Bip39Mnemonic,
}

impl Mnemonic {
    /// Generate a new random 24-word mnemonic
    pub fn generate() -> Self {
        Self::generate_with_word_count(24)
    }

    /// Generate a new random mnemonic with specified word count (12, 15, 18, 21, or 24)
    ///
    /// # Panics
    /// This function should not panic with valid word counts.
    pub fn generate_with_word_count(word_count: usize) -> Self {
        let entropy_bytes = match word_count {
            12 => 16,
            15 => 20,
            18 => 24,
            21 => 28,
            24 => 32,
            _ => 32,
        };
        let entropy: Vec<u8> = (0..entropy_bytes).map(|_| rand::random()).collect();
        let inner = Bip39Mnemonic::from_entropy(&entropy)
            .expect("entropy should be valid");
        Self { inner }
    }

    /// Parse from a mnemonic phrase string
    pub fn from_phrase(phrase: &str) -> Result<Self> {
        let inner = Bip39Mnemonic::parse_normalized(phrase)
            .map_err(|e| WalletError::InvalidMnemonic(e.to_string()))?;
        Ok(Self { inner })
    }

    /// Get the mnemonic phrase as a string
    pub fn phrase(&self) -> &str {
        self.inner.words().collect::<Vec<_>>().join(" ").leak()
    }

    /// Get the words as a vector
    pub fn words(&self) -> Vec<&str> {
        self.inner.words().collect()
    }

    /// Get the word count
    pub fn word_count(&self) -> usize {
        self.inner.word_count()
    }

    /// Derive seed bytes with optional passphrase
    pub fn to_seed(&self, passphrase: &str) -> Zeroizing<[u8; 64]> {
        Zeroizing::new(self.inner.to_seed(passphrase))
    }

    /// Derive seed bytes with empty passphrase
    pub fn to_seed_normalized(&self) -> Zeroizing<[u8; 64]> {
        self.to_seed("")
    }

    /// Validate that a phrase is valid BIP39
    pub fn validate(phrase: &str) -> bool {
        Bip39Mnemonic::parse_normalized(phrase).is_ok()
    }

    /// Get the entropy bytes
    pub fn to_entropy(&self) -> Vec<u8> {
        self.inner.to_entropy()
    }

    /// Create from entropy bytes
    pub fn from_entropy(entropy: &[u8]) -> Result<Self> {
        let inner = Bip39Mnemonic::from_entropy(entropy)
            .map_err(|e| WalletError::InvalidMnemonic(e.to_string()))?;
        Ok(Self { inner })
    }
}

impl std::fmt::Debug for Mnemonic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Mnemonic(<{} words>)", self.word_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_24_words() {
        let m = Mnemonic::generate();
        assert_eq!(m.word_count(), 24);
    }

    #[test]
    fn test_generate_12_words() {
        let m = Mnemonic::generate_with_word_count(12);
        assert_eq!(m.word_count(), 12);
    }

    #[test]
    fn test_parse_valid_phrase() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let m = Mnemonic::from_phrase(phrase).unwrap();
        assert_eq!(m.word_count(), 12);
    }

    #[test]
    fn test_parse_invalid_phrase() {
        let phrase = "invalid mnemonic phrase";
        assert!(Mnemonic::from_phrase(phrase).is_err());
    }

    #[test]
    fn test_to_seed_deterministic() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let m = Mnemonic::from_phrase(phrase).unwrap();
        let seed1 = m.to_seed_normalized();
        let seed2 = m.to_seed_normalized();
        assert_eq!(*seed1, *seed2);
    }

    #[test]
    fn test_passphrase_changes_seed() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let m = Mnemonic::from_phrase(phrase).unwrap();
        let seed1 = m.to_seed("");
        let seed2 = m.to_seed("password");
        assert_ne!(*seed1, *seed2);
    }

    #[test]
    fn test_entropy_roundtrip() {
        let m1 = Mnemonic::generate_with_word_count(12);
        let entropy = m1.to_entropy();
        let m2 = Mnemonic::from_entropy(&entropy).unwrap();
        assert_eq!(m1.words(), m2.words());
    }

    #[test]
    fn test_validate() {
        assert!(Mnemonic::validate("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"));
        assert!(!Mnemonic::validate("invalid phrase"));
    }

    #[test]
    fn test_debug_hides_phrase() {
        let m = Mnemonic::generate();
        let debug = format!("{m:?}");
        assert!(!debug.contains("abandon"));
        assert!(debug.contains("words"));
    }
}
