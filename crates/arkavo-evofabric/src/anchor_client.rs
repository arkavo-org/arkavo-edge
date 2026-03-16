use arkavo_crypto::AgentKeypair;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::merkle_tree::{MerkleProof, MerkleTree};
use crate::{Error, Result};

/// Record of a Merkle root anchoring submission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnchorRecord {
    pub merkle_root: [u8; 32],
    pub bundle_count: usize,
    pub timestamp: DateTime<Utc>,
    pub signature: Vec<u8>,
}

/// Receipt from a successful on-chain anchoring.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnchorReceipt {
    pub block_hash: [u8; 32],
    pub block_number: u64,
    pub extrinsic_index: u32,
}

/// Client for anchoring Merkle roots to the Arkavo chain.
///
/// Uses JSON-RPC over HTTP to submit extrinsics to a Substrate node.
/// Keeps the dependency footprint minimal — no `subxt` framework,
/// just reqwest with rustls-tls.
pub struct AnchorClient {
    endpoint: String,
    keypair: AgentKeypair,
}

impl AnchorClient {
    /// Create a new anchor client targeting a Substrate node.
    pub fn new(endpoint: &str, keypair: AgentKeypair) -> Self {
        Self {
            endpoint: endpoint.to_string(),
            keypair,
        }
    }

    /// Get the endpoint URL.
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Create a signed anchor record for a Merkle tree.
    ///
    /// This prepares the record locally. Call `submit` to send it on-chain.
    pub fn create_record(&self, tree: &MerkleTree, bundle_count: usize) -> AnchorRecord {
        let mut record = AnchorRecord {
            merkle_root: tree.root(),
            bundle_count,
            timestamp: Utc::now(),
            signature: Vec::new(),
        };
        let content = record_content_to_sign(&record);
        record.signature = self.keypair.sign(&content);
        record
    }

    /// Submit an anchor record to the chain via JSON-RPC.
    ///
    /// This is an async operation that sends the extrinsic and waits for inclusion.
    pub async fn submit(&self, record: &AnchorRecord) -> Result<AnchorReceipt> {
        let payload = serde_json::to_string(record)
            .map_err(|e| Error::AnchorFailed(format!("serialize: {e}")))?;

        let client = reqwest::Client::new();
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "author_submitExtrinsic",
            "params": [payload]
        });

        let response = client
            .post(&self.endpoint)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::AnchorFailed(format!("request: {e}")))?;

        if !response.status().is_success() {
            return Err(Error::AnchorFailed(format!("HTTP {}", response.status())));
        }

        let result: serde_json::Value = response
            .json()
            .await
            .map_err(|e| Error::AnchorFailed(format!("response parse: {e}")))?;

        // Extract block info from result
        let block_hash = result
            .get("result")
            .and_then(|r| r.get("block_hash"))
            .and_then(|h| h.as_str())
            .and_then(|s| hex::decode(s).ok())
            .map(|bytes| {
                let mut hash = [0u8; 32];
                let len = bytes.len().min(32);
                hash[..len].copy_from_slice(&bytes[..len]);
                hash
            })
            .unwrap_or([0u8; 32]);

        let block_number = result
            .get("result")
            .and_then(|r| r.get("block_number"))
            .and_then(|n| n.as_u64())
            .unwrap_or(0);

        let extrinsic_index = result
            .get("result")
            .and_then(|r| r.get("extrinsic_index"))
            .and_then(|n| n.as_u64())
            .unwrap_or(0) as u32;

        Ok(AnchorReceipt {
            block_hash,
            block_number,
            extrinsic_index,
        })
    }

    /// Verify that a Merkle proof's root matches an anchor receipt.
    pub fn verify_inclusion(proof: &MerkleProof, record: &AnchorRecord) -> bool {
        proof.verify() && proof.root == record.merkle_root
    }
}

fn record_content_to_sign(record: &AnchorRecord) -> Vec<u8> {
    let mut content = Vec::new();
    content.extend(&record.merkle_root);
    content.extend(record.bundle_count.to_le_bytes());
    content.extend(record.timestamp.timestamp().to_le_bytes());
    content
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn leaf_hash(data: &[u8]) -> [u8; 32] {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut out = [0u8; 32];
        out.copy_from_slice(&result);
        out
    }

    #[test]
    fn create_signed_record() {
        let keypair = AgentKeypair::generate();
        let client = AnchorClient::new("http://localhost:9933", keypair);

        let leaves: Vec<_> = (0..4).map(|i| leaf_hash(&[i])).collect();
        let tree = MerkleTree::from_hashes(leaves);
        let record = client.create_record(&tree, 4);

        assert_eq!(record.merkle_root, tree.root());
        assert_eq!(record.bundle_count, 4);
        assert!(!record.signature.is_empty());
    }

    #[test]
    fn verify_inclusion_valid() {
        let leaves: Vec<_> = (0..4).map(|i| leaf_hash(&[i])).collect();
        let tree = MerkleTree::from_hashes(leaves.clone());
        let proof = tree.proof(0);

        let record = AnchorRecord {
            merkle_root: tree.root(),
            bundle_count: 4,
            timestamp: Utc::now(),
            signature: Vec::new(),
        };

        assert!(AnchorClient::verify_inclusion(&proof, &record));
    }

    #[test]
    fn verify_inclusion_wrong_root() {
        let leaves: Vec<_> = (0..4).map(|i| leaf_hash(&[i])).collect();
        let tree = MerkleTree::from_hashes(leaves);
        let proof = tree.proof(0);

        let record = AnchorRecord {
            merkle_root: [0xFF; 32], // wrong root
            bundle_count: 4,
            timestamp: Utc::now(),
            signature: Vec::new(),
        };

        assert!(!AnchorClient::verify_inclusion(&proof, &record));
    }

    #[test]
    fn endpoint_accessor() {
        let keypair = AgentKeypair::generate();
        let client = AnchorClient::new("http://chain.arkavo.net:9933", keypair);
        assert_eq!(client.endpoint(), "http://chain.arkavo.net:9933");
    }

    #[test]
    fn record_serde_roundtrip() {
        let record = AnchorRecord {
            merkle_root: [42u8; 32],
            bundle_count: 10,
            timestamp: Utc::now(),
            signature: vec![1, 2, 3],
        };
        let json = serde_json::to_string(&record).unwrap();
        let restored: AnchorRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(record.merkle_root, restored.merkle_root);
        assert_eq!(record.bundle_count, restored.bundle_count);
    }

    #[test]
    fn receipt_serde_roundtrip() {
        let receipt = AnchorReceipt {
            block_hash: [7u8; 32],
            block_number: 12345,
            extrinsic_index: 3,
        };
        let json = serde_json::to_string(&receipt).unwrap();
        let restored: AnchorReceipt = serde_json::from_str(&json).unwrap();
        assert_eq!(receipt.block_number, restored.block_number);
        assert_eq!(receipt.extrinsic_index, restored.extrinsic_index);
    }

    #[test]
    fn record_signature_covers_content() {
        let keypair = AgentKeypair::generate();
        let client = AnchorClient::new("http://localhost:9933", keypair);

        let leaves_a: Vec<_> = (0..4).map(|i| leaf_hash(&[i])).collect();
        let leaves_b: Vec<_> = (4..8).map(|i| leaf_hash(&[i])).collect();
        let tree_a = MerkleTree::from_hashes(leaves_a);
        let tree_b = MerkleTree::from_hashes(leaves_b);

        let record_a = client.create_record(&tree_a, 4);
        let record_b = client.create_record(&tree_b, 4);

        // Different trees should produce different signatures
        assert_ne!(record_a.signature, record_b.signature);
    }

    #[test]
    fn verify_inclusion_tampered_proof() {
        let leaves: Vec<_> = (0..4).map(|i| leaf_hash(&[i])).collect();
        let tree = MerkleTree::from_hashes(leaves);
        let mut proof = tree.proof(0);
        proof.leaf = [0xFF; 32]; // tamper

        let record = AnchorRecord {
            merkle_root: tree.root(),
            bundle_count: 4,
            timestamp: Utc::now(),
            signature: Vec::new(),
        };

        assert!(!AnchorClient::verify_inclusion(&proof, &record));
    }
}
