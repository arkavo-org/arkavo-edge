//! EVM-compatible wallet with secp256k1 cryptography
//!
//! This crate provides:
//! - secp256k1 keypair generation and management
//! - EIP-55 checksummed addresses
//! - BIP39 mnemonic support
//! - BIP44 HD derivation for Ethereum
//! - Legacy EVM transaction signing (EIP-155)
//!
//! All cryptographic operations are performed offline - no blockchain RPC calls are made.

pub mod address;
pub mod error;
pub mod hd;
pub mod keypair;
pub mod mnemonic;
pub mod rlp;
pub mod signature;
pub mod transaction;

pub use address::EvmAddress;
pub use error::{Result, WalletError};
pub use hd::{DerivationPath, HdWallet};
pub use keypair::{EvmKeypair, EvmPublicKey};
pub use mnemonic::Mnemonic;
pub use signature::Signature;
pub use transaction::{SignedTransaction, Transaction, TransactionBuilder};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_wallet_flow() {
        let mnemonic = Mnemonic::generate();
        let wallet = HdWallet::from_mnemonic(&mnemonic, "").unwrap();
        let keypair = wallet.derive_ethereum(0).unwrap();

        let to = EvmAddress::from_bytes([0xde; 20]);
        let tx = Transaction::builder()
            .nonce(0)
            .gas_price_gwei(20)
            .gas_limit(21000)
            .to(to)
            .value_ether(0.1)
            .chain_id(1)
            .build();

        let signed = tx.sign(&keypair);
        let recovered = signed.recover_signer().unwrap();
        assert_eq!(recovered, keypair.address());

        let tx_hex = signed.to_hex();
        assert!(tx_hex.starts_with("0x"));
    }

    #[test]
    fn test_did_key_format() {
        let keypair = EvmKeypair::generate();
        let did = keypair.to_did_key();
        assert!(did.starts_with("did:key:z"));
    }

    #[test]
    fn test_address_checksum() {
        let keypair = EvmKeypair::generate();
        let addr = keypair.address();
        let checksummed = addr.to_checksum();
        assert!(checksummed.starts_with("0x"));
        assert!(EvmAddress::verify_checksum(&checksummed));
    }
}
