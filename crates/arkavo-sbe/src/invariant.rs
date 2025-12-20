//! Invariant layer for hard safety constraints
//!
//! Invariant contracts are the highest protection level in SBE.
//! They require human approval and cryptographic signatures for updates.

use std::collections::HashMap;

use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use torg_core::Graph;
use uuid::Uuid;

use crate::error::{SbeError, SbeResult};

/// An invariant contract with cryptographic signature
///
/// Invariant contracts represent hard safety constraints that must
/// never be violated. They are signed by trusted human operators
/// and verified before policy evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvariantContract {
    /// Unique identifier for this contract
    pub id: Uuid,
    /// Human-readable description of the invariant
    pub description: String,
    /// The boolean predicate as a TØRG subgraph
    #[serde(with = "crate::graph_serde")]
    pub predicate: Graph,
    /// Ed25519 signature over the contract content
    pub signature: Vec<u8>,
    /// Public key of the signer
    pub signer_public_key: Vec<u8>,
    /// Timestamp when the contract was created
    pub created_at: DateTime<Utc>,
}

impl InvariantContract {
    /// Create a new unsigned invariant contract
    #[must_use]
    pub fn new(description: String, predicate: Graph) -> Self {
        Self {
            id: Uuid::new_v4(),
            description,
            predicate,
            signature: Vec::new(),
            signer_public_key: Vec::new(),
            created_at: Utc::now(),
        }
    }

    /// Sign this contract with the given keypair
    pub fn sign(&mut self, keypair: &AgentKeypair) {
        let content = self.content_to_sign();
        self.signature = keypair.sign(&content);
        self.signer_public_key = keypair.public_key().to_bytes();
    }

    /// Verify the signature on this contract
    pub fn verify_signature(&self) -> SbeResult<()> {
        if self.signature.is_empty() {
            return Err(SbeError::SignatureVerification(
                "No signature present".into(),
            ));
        }
        if self.signer_public_key.is_empty() {
            return Err(SbeError::SignatureVerification(
                "No public key present".into(),
            ));
        }

        let public_key = AgentPublicKey::from_bytes(&self.signer_public_key)?;
        let content = self.content_to_sign();
        public_key.verify(&content, &self.signature)?;
        Ok(())
    }

    /// Get the content bytes that are signed
    fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.id.as_bytes());
        content.extend(self.description.as_bytes());
        content.extend(self.created_at.timestamp().to_le_bytes());
        content
    }

    /// Evaluate the invariant predicate with given inputs
    pub fn evaluate(&self, inputs: &HashMap<u16, bool>) -> SbeResult<bool> {
        let outputs = torg_core::evaluate(&self.predicate, inputs)?;
        Ok(outputs.values().next().copied().unwrap_or(true))
    }
}

/// Record of an attempted invariant violation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViolationAttempt {
    /// The contract that was violated
    pub contract_id: Uuid,
    /// The inputs that caused the violation
    pub inputs: HashMap<u16, bool>,
    /// When the violation was detected
    pub detected_at: DateTime<Utc>,
    /// Source of the violation (patchlet, policy_update, runtime)
    pub source: ViolationSource,
}

/// Source of an invariant violation attempt
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ViolationSource {
    /// Violation from a patchlet application
    Patchlet,
    /// Violation from a policy update
    PolicyUpdate,
    /// Violation detected at runtime evaluation
    Runtime,
}

/// The invariant layer containing all safety constraints
#[derive(Debug, Clone, Default)]
pub struct InvariantLayer {
    contracts: Vec<InvariantContract>,
    hash: [u8; 32],
}

impl InvariantLayer {
    /// Create a new empty invariant layer
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a signed contract to the layer
    pub fn add_contract(&mut self, contract: InvariantContract) -> SbeResult<()> {
        contract.verify_signature()?;
        self.contracts.push(contract);
        self.update_hash();
        Ok(())
    }

    /// Add an unsigned contract (for testing only)
    #[cfg(test)]
    pub fn add_contract_unchecked(&mut self, contract: InvariantContract) {
        self.contracts.push(contract);
        self.update_hash();
    }

    /// Get all contracts in this layer
    #[must_use]
    pub fn contracts(&self) -> &[InvariantContract] {
        &self.contracts
    }

    /// Get a contract by ID
    #[must_use]
    pub fn get_contract(&self, id: &Uuid) -> Option<&InvariantContract> {
        self.contracts.iter().find(|c| c.id == *id)
    }

    /// Check if the given inputs would violate any invariant
    pub fn check_violation(&self, inputs: &HashMap<u16, bool>) -> Option<ViolationAttempt> {
        for contract in &self.contracts {
            match contract.evaluate(inputs) {
                Ok(true) => continue,
                Ok(false) => {
                    return Some(ViolationAttempt {
                        contract_id: contract.id,
                        inputs: inputs.clone(),
                        detected_at: Utc::now(),
                        source: ViolationSource::Runtime,
                    });
                }
                Err(_) => continue,
            }
        }
        None
    }

    /// Get the hash of all contracts for integrity verification
    #[must_use]
    pub fn hash(&self) -> &[u8; 32] {
        &self.hash
    }

    /// Verify the integrity of the layer
    pub fn verify_hash(&self) -> SbeResult<()> {
        let computed = self.compute_hash();
        if computed == self.hash {
            Ok(())
        } else {
            Err(SbeError::InvalidContract("Hash mismatch".into()))
        }
    }

    fn update_hash(&mut self) {
        self.hash = self.compute_hash();
    }

    fn compute_hash(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for contract in &self.contracts {
            hasher.update(contract.id.as_bytes());
            hasher.update(&contract.signature);
        }
        hasher.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use torg_core::{Builder, Token};

    fn create_simple_graph() -> Graph {
        let mut builder = Builder::new();
        builder.push(Token::InputDecl).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeStart).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.push(Token::Or).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::Id(0)).unwrap();
        builder.push(Token::NodeEnd).unwrap();
        builder.push(Token::OutputDecl).unwrap();
        builder.push(Token::Id(1)).unwrap();
        builder.finish().unwrap()
    }

    #[test]
    fn test_contract_creation() {
        let graph = create_simple_graph();
        let contract = InvariantContract::new("Test invariant".into(), graph);
        assert!(!contract.id.is_nil());
        assert!(contract.signature.is_empty());
    }

    #[test]
    fn test_contract_signing() {
        let graph = create_simple_graph();
        let mut contract = InvariantContract::new("Test invariant".into(), graph);
        let keypair = AgentKeypair::generate();

        contract.sign(&keypair);

        assert!(!contract.signature.is_empty());
        assert!(!contract.signer_public_key.is_empty());
        assert!(contract.verify_signature().is_ok());
    }

    #[test]
    fn test_invariant_layer_add_signed() {
        let graph = create_simple_graph();
        let mut contract = InvariantContract::new("Test".into(), graph);
        let keypair = AgentKeypair::generate();
        contract.sign(&keypair);

        let mut layer = InvariantLayer::new();
        assert!(layer.add_contract(contract).is_ok());
        assert_eq!(layer.contracts().len(), 1);
    }

    #[test]
    fn test_invariant_layer_reject_unsigned() {
        let graph = create_simple_graph();
        let contract = InvariantContract::new("Test".into(), graph);

        let mut layer = InvariantLayer::new();
        assert!(layer.add_contract(contract).is_err());
    }

    #[test]
    fn test_violation_detection() {
        let graph = create_simple_graph();
        let contract = InvariantContract::new("Test".into(), graph);

        let mut layer = InvariantLayer::new();
        layer.add_contract_unchecked(contract);

        let mut inputs_true = HashMap::new();
        inputs_true.insert(0, true);
        assert!(layer.check_violation(&inputs_true).is_none());

        let mut inputs_false = HashMap::new();
        inputs_false.insert(0, false);
        assert!(layer.check_violation(&inputs_false).is_some());
    }

    #[test]
    fn test_hash_verification() {
        let graph = create_simple_graph();
        let contract = InvariantContract::new("Test".into(), graph);

        let mut layer = InvariantLayer::new();
        layer.add_contract_unchecked(contract);

        assert!(layer.verify_hash().is_ok());
    }
}
