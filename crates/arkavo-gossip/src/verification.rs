//! Zero-trust patch verification
//!
//! Provides signature verification for all gossip messages using Ed25519.

use std::collections::HashMap;

use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use arkavo_key_statement::{Generations, KeyStatement, Purpose};
use sha2::{Digest, Sha256};

use crate::advisor_message::AdvisorAdjustmentAnnouncement;
use crate::error::{GossipError, GossipResult};
use crate::learning_message::{LessonAnnouncement, LessonVote};
use crate::message::{PatchAnnouncement, PatchVote};

/// Registry of known public keys for verification
#[derive(Debug, Clone, Default)]
pub struct KeyRegistry {
    /// Map of agent ID to public key
    keys: HashMap<String, AgentPublicKey>,
    /// The workload `did:key` that vouched for each agent id's key
    /// (INGRESS-006). It outlives the key: mDNS goodbyes are unauthenticated,
    /// so an id a workload key has claimed stays that workload's while this
    /// process runs, and a forged goodbye cannot hand it to someone else.
    identities: HashMap<String, String>,
    generations: Generations,
}

impl KeyRegistry {
    /// Create a new empty key registry
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry holding one binding, typically the agent's own key.
    #[must_use]
    pub fn with_key(agent_id: String, pubkey: AgentPublicKey) -> Self {
        let mut keys = HashMap::new();
        keys.insert(agent_id, pubkey);
        Self {
            keys,
            ..Self::default()
        }
    }

    /// Bind a public key to an agent. Binding the same key again is a no-op;
    /// a different key for an id already bound is refused, because replacing
    /// it would let the new key holder sign gossip as that agent. An id a
    /// workload key has vouched for takes only keys that workload vouches
    /// for, through [`Self::register_vouched`].
    pub fn register(&mut self, agent_id: String, pubkey: AgentPublicKey) -> GossipResult<()> {
        match self.keys.get(&agent_id) {
            Some(bound) if bound.to_bytes() != pubkey.to_bytes() => {
                Err(GossipError::KeyConflict(agent_id))
            }
            Some(_) => Ok(()),
            None => {
                if let Some(did) = self.identities.get(&agent_id) {
                    return Err(GossipError::Unvouched {
                        did: did.clone(),
                        agent_id,
                    });
                }
                self.keys.insert(agent_id, pubkey);
                Ok(())
            }
        }
    }

    /// Bind a key its agent's workload key vouched for. `statement` must
    /// already have been verified. The id then belongs to that workload: no
    /// other workload's statement and no unvouched key takes it, and a newer
    /// generation from the same workload replaces the bound key, so an agent
    /// that replaces its gossip key is not refused until this peer restarts.
    pub fn register_vouched(
        &mut self,
        agent_id: String,
        pubkey: AgentPublicKey,
        statement: &KeyStatement,
    ) -> GossipResult<()> {
        if statement.purpose != Purpose::Gossip
            || statement.ed25519_key()?.to_bytes() != pubkey.to_bytes()
        {
            return Err(GossipError::Verification(
                "the key statement does not vouch for this gossip key".into(),
            ));
        }
        let pinned = self.identities.get(&agent_id);
        if let Some(pinned) = pinned.filter(|did| **did != statement.issuer) {
            return Err(GossipError::IdentityConflict {
                pinned: pinned.clone(),
                presented: statement.issuer.clone(),
                agent_id,
            });
        }
        let replaces_unvouched = pinned.is_none()
            && self
                .keys
                .get(&agent_id)
                .is_some_and(|bound| bound.to_bytes() != pubkey.to_bytes());
        if replaces_unvouched {
            return Err(GossipError::KeyConflict(agent_id));
        }
        // Last, because it records the generation: nothing may fail after it.
        self.generations.admit(statement)?;
        self.identities
            .insert(agent_id.clone(), statement.issuer.clone());
        self.keys.insert(agent_id, pubkey);
        Ok(())
    }

    /// The workload `did:key` that vouched for `agent_id`'s key, if one did.
    #[must_use]
    pub fn identity(&self, agent_id: &str) -> Option<&str> {
        self.identities.get(agent_id).map(String::as_str)
    }

    /// Forget an agent's key, as when the agent leaves the network. Which
    /// workload the id belongs to is kept.
    pub fn unregister(&mut self, agent_id: &str) {
        self.keys.remove(agent_id);
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
    pub fn verify_lesson_announcement(
        &self,
        announcement: &LessonAnnouncement,
    ) -> GossipResult<()> {
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

    /// Verify an advisor adjustment announcement signature
    pub fn verify_advisor_announcement(
        &self,
        announcement: &AdvisorAdjustmentAnnouncement,
    ) -> GossipResult<()> {
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

/// Sign an advisor adjustment announcement
pub fn sign_advisor_announcement(
    announcement: &mut AdvisorAdjustmentAnnouncement,
    keypair: &AgentKeypair,
) -> GossipResult<()> {
    let content = announcement.content_to_sign();
    let signature = keypair.sign(&content);
    announcement.signature = signature;
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
        registry
            .register("agent-1".to_string(), keypair.public_key().clone())
            .unwrap();

        assert!(!registry.is_empty());
        assert_eq!(registry.len(), 1);
        assert!(registry.contains("agent-1"));
        assert!(!registry.contains("agent-2"));
    }

    #[test]
    fn test_sign_and_verify_announcement() {
        let keypair = create_keypair();
        let mut registry = KeyRegistry::new();
        registry
            .register("agent-1".to_string(), keypair.public_key().clone())
            .unwrap();

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
        registry
            .register("voter-1".to_string(), keypair.public_key().clone())
            .unwrap();

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
        registry
            .register("agent-1".to_string(), keypair1.public_key().clone())
            .unwrap();

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

    mod vouched {
        use super::*;
        use crate::test_support::Workload;
        use arkavo_test_macros::spec;

        #[spec("INGRESS-017")]
        #[test]
        fn a_restarted_agent_rebinds_its_kept_key() {
            let workload = Workload::new();
            let gossip = AgentKeypair::generate().public_key();
            let (_, statement) = workload.vouch(&gossip, 0);
            let mut registry = KeyRegistry::new();
            registry
                .register_vouched("planner".into(), gossip.clone(), &statement)
                .unwrap();
            assert_eq!(registry.identity("planner"), Some(workload.did.as_str()));

            registry.unregister("planner");
            registry
                .register_vouched("planner".into(), gossip.clone(), &statement)
                .unwrap();
            assert!(registry.get("planner").is_some());
        }

        /// A forged mDNS goodbye unbinds the key; the name must not then go
        /// to whoever binds next.
        #[spec("INGRESS-017")]
        #[test]
        fn a_vouched_name_takes_no_unvouched_key_even_after_a_goodbye() {
            let workload = Workload::new();
            let gossip = AgentKeypair::generate().public_key();
            let (_, statement) = workload.vouch(&gossip, 0);
            let mut registry = KeyRegistry::new();
            registry
                .register_vouched("planner".into(), gossip, &statement)
                .unwrap();
            registry.unregister("planner");

            let intruder = AgentKeypair::generate().public_key();
            let err = registry.register("planner".into(), intruder).unwrap_err();
            assert!(matches!(err, GossipError::Unvouched { .. }), "{err}");
            assert!(registry.get("planner").is_none());
        }

        #[spec("INGRESS-017")]
        #[test]
        fn a_vouched_name_takes_no_other_agents_statement() {
            let planner = Workload::new();
            let intruder = Workload::new();
            let mut registry = KeyRegistry::new();
            let gossip = AgentKeypair::generate().public_key();
            let (_, statement) = planner.vouch(&gossip, 0);
            registry
                .register_vouched("planner".into(), gossip, &statement)
                .unwrap();
            registry.unregister("planner");

            let theirs = AgentKeypair::generate().public_key();
            let (_, statement) = intruder.vouch(&theirs, 7);
            let err = registry
                .register_vouched("planner".into(), theirs, &statement)
                .unwrap_err();
            assert!(err.to_string().contains("needs a new name"), "{err}");
        }

        #[spec("INGRESS-017")]
        #[test]
        fn a_newer_generation_replaces_the_key_and_an_older_one_is_refused() {
            let workload = Workload::new();
            let first = AgentKeypair::generate().public_key();
            let second = AgentKeypair::generate().public_key();
            let (_, old) = workload.vouch(&first, 1);
            let (_, new) = workload.vouch(&second, 2);
            let mut registry = KeyRegistry::new();
            registry
                .register_vouched("planner".into(), first.clone(), &old)
                .unwrap();
            registry
                .register_vouched("planner".into(), second.clone(), &new)
                .unwrap();
            assert_eq!(
                registry.get("planner").unwrap().to_bytes(),
                second.to_bytes()
            );

            assert!(
                registry
                    .register_vouched("planner".into(), first, &old)
                    .is_err()
            );
            let (_, rival) = workload.vouch(&AgentKeypair::generate().public_key(), 2);
            assert!(
                registry
                    .register_vouched("planner".into(), rival.ed25519_key().unwrap(), &rival)
                    .is_err()
            );
            assert_eq!(
                registry.get("planner").unwrap().to_bytes(),
                second.to_bytes()
            );
        }

        #[spec("INGRESS-017")]
        #[test]
        fn a_statement_does_not_displace_a_bound_unvouched_key() {
            let workload = Workload::new();
            let mut registry = KeyRegistry::new();
            registry
                .register("planner".into(), AgentKeypair::generate().public_key())
                .unwrap();
            let gossip = AgentKeypair::generate().public_key();
            let (_, statement) = workload.vouch(&gossip, 0);
            assert!(matches!(
                registry.register_vouched("planner".into(), gossip, &statement),
                Err(GossipError::KeyConflict(_))
            ));
            assert_eq!(registry.identity("planner"), None);
        }

        #[spec("INGRESS-017")]
        #[test]
        fn a_statement_for_another_key_binds_nothing() {
            let workload = Workload::new();
            let (_, statement) = workload.vouch(&AgentKeypair::generate().public_key(), 0);
            let mut registry = KeyRegistry::new();
            let other = AgentKeypair::generate().public_key();
            assert!(
                registry
                    .register_vouched("planner".into(), other, &statement)
                    .is_err()
            );
            assert!(registry.get("planner").is_none());
            assert_eq!(registry.identity("planner"), None);
        }
    }
}
