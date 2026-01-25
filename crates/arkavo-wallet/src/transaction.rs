use crate::address::EvmAddress;
use crate::error::Result;
use crate::keypair::EvmKeypair;
use crate::rlp;
use crate::signature::Signature;
use alloy_primitives::U256;
use sha3::{Digest, Keccak256};

/// EVM transaction (legacy format for maximum compatibility)
#[derive(Debug, Clone)]
pub struct Transaction {
    pub nonce: u64,
    pub gas_price: U256,
    pub gas_limit: u64,
    pub to: Option<EvmAddress>,
    pub value: U256,
    pub data: Vec<u8>,
    pub chain_id: u64,
}

impl Default for Transaction {
    fn default() -> Self {
        Self {
            nonce: 0,
            gas_price: U256::ZERO,
            gas_limit: 21000,
            to: None,
            value: U256::ZERO,
            data: Vec::new(),
            chain_id: 1,
        }
    }
}

impl Transaction {
    /// Create a new transaction builder
    pub fn builder() -> TransactionBuilder {
        TransactionBuilder::default()
    }

    /// Create a simple ETH transfer transaction
    pub fn transfer(to: EvmAddress, value: U256, chain_id: u64) -> Self {
        Self {
            to: Some(to),
            value,
            chain_id,
            ..Default::default()
        }
    }

    /// RLP encode for signing (includes chain_id per EIP-155)
    fn rlp_for_signing(&self) -> Vec<u8> {
        let mut list_bytes = Vec::new();
        rlp::encode_u64(self.nonce, &mut list_bytes);
        rlp::encode_u256(&self.gas_price, &mut list_bytes);
        rlp::encode_u64(self.gas_limit, &mut list_bytes);

        if let Some(to) = &self.to {
            rlp::encode_bytes(to.as_bytes(), &mut list_bytes);
        } else {
            rlp::encode_bytes(&[], &mut list_bytes);
        }

        rlp::encode_u256(&self.value, &mut list_bytes);
        rlp::encode_bytes(&self.data, &mut list_bytes);
        rlp::encode_u64(self.chain_id, &mut list_bytes);
        rlp::encode_u64(0, &mut list_bytes);
        rlp::encode_u64(0, &mut list_bytes);

        let mut stream = Vec::new();
        rlp::encode_list(&list_bytes, &mut stream);
        stream
    }

    /// Compute transaction hash for signing
    pub fn signing_hash(&self) -> [u8; 32] {
        let encoded = self.rlp_for_signing();
        Keccak256::digest(&encoded).into()
    }

    /// Sign the transaction
    pub fn sign(&self, keypair: &EvmKeypair) -> SignedTransaction {
        let hash = self.signing_hash();
        let sig = keypair.sign_prehash(&hash);
        SignedTransaction {
            tx: self.clone(),
            signature: sig,
        }
    }
}

/// Transaction builder for ergonomic construction
#[derive(Debug, Default)]
pub struct TransactionBuilder {
    tx: Transaction,
}

impl TransactionBuilder {
    pub fn nonce(mut self, nonce: u64) -> Self {
        self.tx.nonce = nonce;
        self
    }

    pub fn gas_price(mut self, gas_price: U256) -> Self {
        self.tx.gas_price = gas_price;
        self
    }

    pub fn gas_price_gwei(mut self, gwei: u64) -> Self {
        self.tx.gas_price = U256::from(gwei) * U256::from(1_000_000_000u64);
        self
    }

    pub fn gas_limit(mut self, gas_limit: u64) -> Self {
        self.tx.gas_limit = gas_limit;
        self
    }

    pub fn to(mut self, to: EvmAddress) -> Self {
        self.tx.to = Some(to);
        self
    }

    pub fn value(mut self, value: U256) -> Self {
        self.tx.value = value;
        self
    }

    pub fn value_wei(mut self, wei: u64) -> Self {
        self.tx.value = U256::from(wei);
        self
    }

    pub fn value_ether(mut self, ether: f64) -> Self {
        let wei = (ether * 1e18) as u128;
        self.tx.value = U256::from(wei);
        self
    }

    pub fn data(mut self, data: Vec<u8>) -> Self {
        self.tx.data = data;
        self
    }

    pub fn chain_id(mut self, chain_id: u64) -> Self {
        self.tx.chain_id = chain_id;
        self
    }

    pub fn build(self) -> Transaction {
        self.tx
    }
}

/// Signed EVM transaction ready for broadcast
#[derive(Debug, Clone)]
pub struct SignedTransaction {
    pub tx: Transaction,
    pub signature: Signature,
}

impl SignedTransaction {
    /// Get the transaction hash (used as tx ID)
    pub fn tx_hash(&self) -> [u8; 32] {
        let encoded = self.to_rlp_bytes();
        Keccak256::digest(&encoded).into()
    }

    /// Get the transaction hash as hex string
    pub fn tx_hash_hex(&self) -> String {
        format!("0x{}", hex::encode(self.tx_hash()))
    }

    /// Encode as RLP bytes for broadcast
    pub fn to_rlp_bytes(&self) -> Vec<u8> {
        let v = self.signature.eip155_v(self.tx.chain_id);

        let mut list_bytes = Vec::new();
        rlp::encode_u64(self.tx.nonce, &mut list_bytes);
        rlp::encode_u256(&self.tx.gas_price, &mut list_bytes);
        rlp::encode_u64(self.tx.gas_limit, &mut list_bytes);

        if let Some(to) = &self.tx.to {
            rlp::encode_bytes(to.as_bytes(), &mut list_bytes);
        } else {
            rlp::encode_bytes(&[], &mut list_bytes);
        }

        rlp::encode_u256(&self.tx.value, &mut list_bytes);
        rlp::encode_bytes(&self.tx.data, &mut list_bytes);
        rlp::encode_u64(v, &mut list_bytes);
        rlp::encode_bytes(self.signature.r(), &mut list_bytes);
        rlp::encode_bytes(self.signature.s(), &mut list_bytes);

        let mut stream = Vec::new();
        rlp::encode_list(&list_bytes, &mut stream);
        stream
    }

    /// Get hex-encoded transaction for broadcast
    pub fn to_hex(&self) -> String {
        format!("0x{}", hex::encode(self.to_rlp_bytes()))
    }

    /// Recover the signer's address
    pub fn recover_signer(&self) -> Result<EvmAddress> {
        let hash = self.tx.signing_hash();
        let pubkey = self.signature.recover_public_key(&hash)?;
        let encoded = pubkey.to_encoded_point(false);
        EvmAddress::from_public_key(&encoded.as_bytes()[1..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transaction_default() {
        let tx = Transaction::default();
        assert_eq!(tx.nonce, 0);
        assert_eq!(tx.gas_limit, 21000);
        assert_eq!(tx.chain_id, 1);
    }

    #[test]
    fn test_transaction_builder() {
        let to = EvmAddress::from_bytes([0xab; 20]);
        let tx = Transaction::builder()
            .nonce(5)
            .gas_price_gwei(20)
            .gas_limit(21000)
            .to(to)
            .value_ether(1.5)
            .chain_id(1)
            .build();

        assert_eq!(tx.nonce, 5);
        assert_eq!(tx.gas_limit, 21000);
        assert_eq!(tx.to, Some(to));
        assert!(tx.value > U256::ZERO);
    }

    #[test]
    fn test_sign_and_recover() {
        let kp = EvmKeypair::generate();
        let to = EvmAddress::from_bytes([0xab; 20]);

        let tx = Transaction::builder()
            .nonce(0)
            .gas_price_gwei(20)
            .gas_limit(21000)
            .to(to)
            .value_wei(1000)
            .chain_id(1)
            .build();

        let signed = tx.sign(&kp);
        let recovered = signed.recover_signer().unwrap();
        assert_eq!(recovered, kp.address());
    }

    #[test]
    fn test_tx_hash_deterministic() {
        let kp = EvmKeypair::from_bytes(&[1u8; 32]).unwrap();
        let to = EvmAddress::from_bytes([0xab; 20]);

        let tx = Transaction::builder()
            .nonce(0)
            .gas_price_gwei(20)
            .gas_limit(21000)
            .to(to)
            .value_wei(1000)
            .chain_id(1)
            .build();

        let signed1 = tx.sign(&kp);
        let signed2 = tx.sign(&kp);
        assert_eq!(signed1.tx_hash(), signed2.tx_hash());
    }

    #[test]
    fn test_rlp_encoding() {
        let kp = EvmKeypair::generate();
        let to = EvmAddress::from_bytes([0xab; 20]);

        let tx = Transaction::transfer(to, U256::from(1000u64), 1);
        let signed = tx.sign(&kp);

        let rlp_bytes = signed.to_rlp_bytes();
        assert!(!rlp_bytes.is_empty());

        let hex_str = signed.to_hex();
        assert!(hex_str.starts_with("0x"));
    }

    #[test]
    fn test_transfer_helper() {
        let to = EvmAddress::from_bytes([0xab; 20]);
        let tx = Transaction::transfer(to, U256::from(1_000_000_000_000_000_000u128), 1);

        assert_eq!(tx.to, Some(to));
        assert_eq!(tx.value, U256::from(1_000_000_000_000_000_000u128));
        assert!(tx.data.is_empty());
    }

    #[test]
    fn test_signing_hash_changes_with_chain_id() {
        let to = EvmAddress::from_bytes([0xab; 20]);

        let tx1 = Transaction::builder().to(to).chain_id(1).build();
        let tx2 = Transaction::builder().to(to).chain_id(137).build();

        assert_ne!(tx1.signing_hash(), tx2.signing_hash());
    }
}
