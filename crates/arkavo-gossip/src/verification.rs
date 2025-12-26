//! Zero-trust patch verification
//!
//! Provides signature verification for all gossip messages using Ed25519.

use std::collections::HashMap;

use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use sha2::{Digest, Sha256};

use crate::error::{GossipError, GossipResult};
use crate::learning_message::{LessonAnnouncement, LessonVote};
use crate::message::{PatchAnnouncement, PatchVote};

/// Registry of known public keys for verification
#[derive(Debug, Clone, Default)]
pub struct KeyRegistry {
    /// Map of agent ID to public key
    keys: HashMap<String, AgentPublicKey>,
}

impl KeyRegistry {
    /// Create a new empty key registry
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a public key for an agent
    pub fn register(&mut self, agent_id: String, pubkey: AgentPublicKey) {
        self.keys.insert(agent_id, pubkey);
    }

    /// Get the public key for an agent
    #[must_use]
    pub fn get(&self, agent_id: &str) -> Option<&AgentPublicKey> {
        self.keys.get(agent_id)
    }

    /// Check if an agent is known
    #[must_use]
    pub fn contains(&self, agent_id: &str) -> bool {
        self.keys.contains_key(agent_id)
    }

    /// Number of registered keys
    #[must_use]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Check if the registry is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

/// Verifier for gossip messages
pub struct PatchVerifier {
    /// Known public keys
    registry: KeyRegistry,
}

impl PatchVerifier {
    /// Create a new verifier with the given key registry
    #[must_use]
    pub fn new(registry: KeyRegistry) -> Self {
        Self { registry }
    }

    /// Verify a patch announcement signature
    pub fn verify_announcement(&self, announcement: &PatchAnnouncement) -> GossipResult<()> {
        let pubkey = self
            .registry
            .get(&announcement.originator)
            .ok_or_else(|| GossipError::UnknownOriginator(announcement.originator.clone()))?;

        let content = announcement.content_to_sign();
        pubkey
            .verify(&content, &announcement.signature)
            .map_err(GossipError::SignatureVerification)?;

        Ok(())
    }

    /// Verify a patch vote signature
    pub fn verify_vote(&self, vote: &PatchVote) -> GossipResult<()> {
        let pubkey = self
            .registry
            .get(&vote.voter)
            .ok_or_else(|| GossipError::UnknownOriginator(vote.voter.clone()))?;

        let content = vote.content_to_sign();
        pubkey
            .verify(&content, &vote.signature)
            .map_err(GossipError::SignatureVerification)?;

        Ok(())
    }

    /// Verify content hash matches expected
    pub fn verify_content_hash(
        &self,
        content: &[u8],
        expected_hash: &[u8; 32],
    ) -> GossipResult<()> {
        let mut hasher = Sha256::new();
        hasher.update(content);
        let actual_hash: [u8; 32] = hasher.finalize().into();

        if actual_hash != *expected_hash {
            return Err(GossipError::Verification(format!(
                "Content hash mismatch: expected {:?}, got {:?}",
                hex::encode(expected_hash),
                hex::encode(actual_hash)
            )));
        }

        Ok(())
    }

    /// Verify a lesson announcement signature
    pub fn verify_lesson_announcement(&self, announcement: &LessonAnnouncement) -> GossipResult<()> {
        let pubkey = self
            .registry
            .get(&announcement.originator)
            .ok_or_else(|| GossipError::UnknownOriginator(announcement.originator.clone()))?;

        let content = announcement.content_to_sign();
        pubkey
            .verify(&content, &announcement.signature)
            .map_err(GossipError::SignatureVerification)?;

        Ok(())
    }

    /// Verify a lesson vote signature
    pub fn verify_lesson_vote(&self, vote: &LessonVote) -> GossipResult<()> {
        let pubkey = self
            .registry
            .get(&vote.voter)
            .ok_or_else(|| GossipError::UnknownOriginator(vote.voter.clone()))?;

        let content = vote.content_to_sign();
        pubkey
            .verify(&content, &vote.signature)
            .map_err(GossipError::SignatureVerification)?;

        Ok(())
    }

    /// Get the underlying key registry
    #[must_use]
    pub fn registry(&self) -> &KeyRegistry {
        &self.registry
    }

    /// Get mutable access to the key registry
    pub fn registry_mut(&mut self) -> &mut KeyRegistry {
        &mut self.registry
    }
}

/// Sign a patch announcement
pub fn sign_announcement(
    announcement: &mut PatchAnnouncement,
    keypair: &AgentKeypair,
) -> GossipResult<()> {
    let content = announcement.content_to_sign();
    let signature = keypair.sign(&content);
    announcement.signature = signature;
    Ok(())
}

/// Sign a patch vote
pub fn sign_vote(vote: &mut PatchVote, keypair: &AgentKeypair) -> GossipResult<()> {
    let content = vote.content_to_sign();
    let signature = keypair.sign(&content);
    vote.signature = signature;
    Ok(())
}

/// Sign a lesson announcement
pub fn sign_lesson_announcement(
    announcement: &mut LessonAnnouncement,
    keypair: &AgentKeypair,
) -> GossipResult<()> {
    let content = announcement.content_to_sign();
    let signature = keypair.sign(&content);
    announcement.signature = signature;
    Ok(())
}

/// Sign a lesson vote
pub fn sign_lesson_vote(vote: &mut LessonVote, keypair: &AgentKeypair) -> GossipResult<()> {
    let content = vote.content_to_sign();
    let signature = keypair.sign(&content);
    vote.signature = signature;
    Ok(())
}

/// Compute hash of patch content
pub fn compute_content_hash(content: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(content);
    hasher.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn create_keypair() -> AgentKeypair {
        AgentKeypair::generate()
    }

    #[test]
    fn test_key_registry() {
        let mut registry = KeyRegistry::new();
        assert!(registry.is_empty());

        let keypair = create_keypair();
        registry.register("agent-1".to_string(), keypair.public_key().clone());

        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
        assert!(registry.contains("agent-1"));
        assert!(!registry.contains("agent-2"));
    }

    #[test]
    fn test_sign_and_verify_announcement() {
        let keypair = create_keypair();
        let mut registry = KeyRegistry::new();
        registry.register("agent-1".to_string(), keypair.public_key().clone());

        let mut announcement =
            PatchAnnouncement::new(Uuid::new_v4(), [0u8; 32], "agent-1".to_string(), vec![]);

        sign_announcement(&mut announcement, &keypair).unwrap();
        assert!(!announcement.signature.is_empty());

        let verifier = PatchVerifier::new(registry);
        verifier.verify_announcement(&announcement).unwrap();
    }

    #[test]
    fn test_sign_and_verify_vote() {
        let keypair = create_keypair();
        let mut registry = KeyRegistry::new();
        registry.register("voter-1".to_string(), keypair.public_key().clone());

        let mut vote = PatchVote::new(Uuid::new_v4(), "voter-1".to_string(), true);

        sign_vote(&mut vote, &keypair).unwrap();
        assert!(!vote.signature.is_empty());

        let verifier = PatchVerifier::new(registry);
        verifier.verify_vote(&vote).unwrap();
    }

    #[test]
    fn test_verify_unknown_originator() {
        let registry = KeyRegistry::new();
        let verifier = PatchVerifier::new(registry);

        let announcement = PatchAnnouncement::new(
            Uuid::new_v4(),
            [0u8; 32],
            "unknown-agent".to_string(),
            vec![],
        );

        let result = verifier.verify_announcement(&announcement);
        assert!(matches!(result, Err(GossipError::UnknownOriginator(_))));
    }

    #[test]
    fn test_verify_invalid_signature() {
        let keypair1 = create_keypair();
        let keypair2 = create_keypair();

        let mut registry = KeyRegistry::new();
        registry.register("agent-1".to_string(), keypair1.public_key().clone());

        // Sign with different key
        let mut announcement =
            PatchAnnouncement::new(Uuid::new_v4(), [0u8; 32], "agent-1".to_string(), vec![]);
        sign_announcement(&mut announcement, &keypair2).unwrap();

        let verifier = PatchVerifier::new(registry);
        let result = verifier.verify_announcement(&announcement);
        assert!(matches!(result, Err(GossipError::SignatureVerification(_))));
    }

    #[test]
    fn test_content_hash() {
        let content = b"test patch content";
        let hash = compute_content_hash(content);

        let registry = KeyRegistry::new();
        let verifier = PatchVerifier::new(registry);

        // Correct hash should verify
        verifier.verify_content_hash(content, &hash).unwrap();

        // Wrong hash should fail
        let wrong_hash = [0u8; 32];
        let result = verifier.verify_content_hash(content, &wrong_hash);
        assert!(matches!(result, Err(GossipError::Verification(_))));
    }
}
