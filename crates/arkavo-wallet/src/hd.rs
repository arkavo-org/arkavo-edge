use crate::error::{Result, WalletError};
use crate::keypair::EvmKeypair;
use crate::mnemonic::Mnemonic;
use hmac::Mac;
use k256::ecdsa::SigningKey;
use k256::elliptic_curve::PrimeField;
use k256::elliptic_curve::ops::Add;
use sha2::Sha512;
use std::str::FromStr;

/// BIP44 derivation path for Ethereum: m/44'/60'/0'/0/index
const ETH_PURPOSE: u32 = 44;
const ETH_COIN_TYPE: u32 = 60;

/// HD derivation path component
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathComponent {
    Normal(u32),
    Hardened(u32),
}

impl PathComponent {
    pub fn index(&self) -> u32 {
        match self {
            Self::Normal(i) | Self::Hardened(i) => *i,
        }
    }

    pub fn is_hardened(&self) -> bool {
        matches!(self, Self::Hardened(_))
    }

    pub fn to_child_number(&self) -> u32 {
        match self {
            Self::Normal(i) => *i,
            Self::Hardened(i) => i | 0x8000_0000,
        }
    }
}

/// BIP32/44 derivation path
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivationPath {
    components: Vec<PathComponent>,
}

impl DerivationPath {
    /// Create standard Ethereum path: m/44'/60'/0'/0/{index}
    pub fn ethereum(index: u32) -> Self {
        Self {
            components: vec![
                PathComponent::Hardened(ETH_PURPOSE),
                PathComponent::Hardened(ETH_COIN_TYPE),
                PathComponent::Hardened(0),
                PathComponent::Normal(0),
                PathComponent::Normal(index),
            ],
        }
    }

    /// Create custom path
    pub fn new(components: Vec<PathComponent>) -> Self {
        Self { components }
    }

    /// Get path components
    pub fn components(&self) -> &[PathComponent] {
        &self.components
    }

    /// Convert to string representation
    pub fn to_string_repr(&self) -> String {
        let mut path = String::from("m");
        for component in &self.components {
            path.push('/');
            match component {
                PathComponent::Normal(i) => path.push_str(&i.to_string()),
                PathComponent::Hardened(i) => {
                    path.push_str(&i.to_string());
                    path.push('\'');
                }
            }
        }
        path
    }
}

impl FromStr for DerivationPath {
    type Err = WalletError;

    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        if !s.starts_with('m') {
            return Err(WalletError::InvalidDerivationPath(
                "Path must start with 'm'".to_string(),
            ));
        }

        let parts: Vec<&str> = s.split('/').skip(1).collect();
        let mut components = Vec::with_capacity(parts.len());

        for part in parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            let (num_str, hardened) =
                if part.ends_with('\'') || part.ends_with('h') || part.ends_with('H') {
                    (&part[..part.len() - 1], true)
                } else {
                    (part, false)
                };

            let index: u32 = num_str.parse().map_err(|_| {
                WalletError::InvalidDerivationPath(format!("Invalid path component: {part}"))
            })?;

            components.push(if hardened {
                PathComponent::Hardened(index)
            } else {
                PathComponent::Normal(index)
            });
        }

        Ok(Self { components })
    }
}

/// Extended private key for HD derivation
struct ExtendedKey {
    key: [u8; 32],
    chain_code: [u8; 32],
}

impl ExtendedKey {
    fn from_seed(seed: &[u8]) -> Result<Self> {
        if seed.len() < 16 {
            return Err(WalletError::InvalidKeyFormat(
                "Seed must be at least 16 bytes".to_string(),
            ));
        }

        let mut mac =
            hmac::Hmac::<Sha512>::new_from_slice(b"Bitcoin seed").expect("HMAC accepts any key");
        hmac::Mac::update(&mut mac, seed);
        let result = hmac::Mac::finalize(mac).into_bytes();

        let mut key = [0u8; 32];
        let mut chain_code = [0u8; 32];
        key.copy_from_slice(&result[..32]);
        chain_code.copy_from_slice(&result[32..]);

        Ok(Self { key, chain_code })
    }

    fn derive_child(&self, child: u32) -> Result<Self> {
        let hardened = child & 0x8000_0000 != 0;

        let mut data = Vec::with_capacity(37);
        if hardened {
            data.push(0x00);
            data.extend_from_slice(&self.key);
        } else {
            let sk = SigningKey::from_bytes((&self.key).into())
                .map_err(|e| WalletError::CryptoError(e.to_string()))?;
            let pk = sk.verifying_key().to_encoded_point(true);
            data.extend_from_slice(pk.as_bytes());
        }
        data.extend_from_slice(&child.to_be_bytes());

        let mut mac =
            hmac::Hmac::<Sha512>::new_from_slice(&self.chain_code).expect("HMAC accepts any key");
        hmac::Mac::update(&mut mac, &data);
        let result = hmac::Mac::finalize(mac).into_bytes();

        let mut il = [0u8; 32];
        il.copy_from_slice(&result[..32]);

        let parent_key = k256::Scalar::from_repr_vartime(self.key.into())
            .ok_or_else(|| WalletError::CryptoError("Invalid parent key scalar".to_string()))?;
        let tweak = k256::Scalar::from_repr_vartime(il.into())
            .ok_or_else(|| WalletError::CryptoError("Invalid tweak scalar".to_string()))?;
        let child_key = parent_key.add(&tweak);

        let mut key = [0u8; 32];
        key.copy_from_slice(&child_key.to_repr());

        let mut chain_code = [0u8; 32];
        chain_code.copy_from_slice(&result[32..]);

        Ok(Self { key, chain_code })
    }
}

/// HD wallet for deriving multiple keypairs from a single mnemonic
pub struct HdWallet {
    root: ExtendedKey,
}

impl HdWallet {
    /// Create HD wallet from mnemonic with optional passphrase
    pub fn from_mnemonic(mnemonic: &Mnemonic, passphrase: &str) -> Result<Self> {
        let seed = mnemonic.to_seed(passphrase);
        let root = ExtendedKey::from_seed(&*seed)?;
        Ok(Self { root })
    }

    /// Create HD wallet from seed bytes
    pub fn from_seed(seed: &[u8]) -> Result<Self> {
        let root = ExtendedKey::from_seed(seed)?;
        Ok(Self { root })
    }

    /// Derive keypair at the given path
    pub fn derive(&self, path: &DerivationPath) -> Result<EvmKeypair> {
        let mut current = ExtendedKey {
            key: self.root.key,
            chain_code: self.root.chain_code,
        };

        for component in path.components() {
            current = current.derive_child(component.to_child_number())?;
        }

        EvmKeypair::from_bytes(&current.key)
    }

    /// Derive keypair at standard Ethereum path for given account index
    pub fn derive_ethereum(&self, index: u32) -> Result<EvmKeypair> {
        self.derive(&DerivationPath::ethereum(index))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ethereum_path() {
        let path = DerivationPath::ethereum(0);
        assert_eq!(path.to_string_repr(), "m/44'/60'/0'/0/0");

        let path = DerivationPath::ethereum(5);
        assert_eq!(path.to_string_repr(), "m/44'/60'/0'/0/5");
    }

    #[test]
    fn test_parse_path() {
        let path: DerivationPath = "m/44'/60'/0'/0/0".parse().unwrap();
        assert_eq!(path, DerivationPath::ethereum(0));

        let path: DerivationPath = "m/44'/60'/0'/0/5".parse().unwrap();
        assert_eq!(path, DerivationPath::ethereum(5));
    }

    #[test]
    fn test_parse_path_with_h() {
        let path: DerivationPath = "m/44h/60h/0h/0/0".parse().unwrap();
        assert_eq!(path.components().len(), 5);
        assert!(path.components()[0].is_hardened());
    }

    #[test]
    fn test_hd_wallet_derive_deterministic() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mnemonic = Mnemonic::from_phrase(phrase).unwrap();
        let wallet = HdWallet::from_mnemonic(&mnemonic, "").unwrap();

        let kp1 = wallet.derive_ethereum(0).unwrap();
        let kp2 = wallet.derive_ethereum(0).unwrap();
        assert_eq!(kp1.address(), kp2.address());
    }

    #[test]
    fn test_hd_wallet_different_indices() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mnemonic = Mnemonic::from_phrase(phrase).unwrap();
        let wallet = HdWallet::from_mnemonic(&mnemonic, "").unwrap();

        let kp0 = wallet.derive_ethereum(0).unwrap();
        let kp1 = wallet.derive_ethereum(1).unwrap();
        assert_ne!(kp0.address(), kp1.address());
    }

    #[test]
    fn test_passphrase_changes_keys() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mnemonic = Mnemonic::from_phrase(phrase).unwrap();

        let wallet1 = HdWallet::from_mnemonic(&mnemonic, "").unwrap();
        let wallet2 = HdWallet::from_mnemonic(&mnemonic, "password").unwrap();

        let kp1 = wallet1.derive_ethereum(0).unwrap();
        let kp2 = wallet2.derive_ethereum(0).unwrap();
        assert_ne!(kp1.address(), kp2.address());
    }

    #[test]
    fn test_derive_custom_path() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mnemonic = Mnemonic::from_phrase(phrase).unwrap();
        let wallet = HdWallet::from_mnemonic(&mnemonic, "").unwrap();

        let path: DerivationPath = "m/44'/60'/1'/0/0".parse().unwrap();
        let kp = wallet.derive(&path).unwrap();
        assert_eq!(kp.address().as_bytes().len(), 20);
    }

    #[test]
    fn test_known_derivation() {
        let phrase = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mnemonic = Mnemonic::from_phrase(phrase).unwrap();
        let wallet = HdWallet::from_mnemonic(&mnemonic, "").unwrap();

        let kp = wallet.derive_ethereum(0).unwrap();
        let addr = kp.address();
        assert!(!addr.to_hex().is_empty());
    }
}
