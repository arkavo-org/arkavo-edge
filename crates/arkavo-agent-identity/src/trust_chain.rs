//! The published trust chain linking a subordinate AIA to a root anchor.

use crate::document::parse_rfc3339;
use crate::{IdentityError, Result, verify_payload};
use arkavo_crypto::AgentPublicKey;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The signed claims of a parent AIA endorsing a subordinate AIA.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndorsementClaims {
    /// Endorsing (parent) authority identifier.
    pub parent_id: String,
    /// Endorsed (subordinate) authority identifier.
    pub child_id: String,
    /// Subordinate AIA's public key as a `did:key` string.
    pub child_key: String,
    /// When the endorsement was issued (RFC 3339).
    pub issued_at: String,
    /// When the endorsement expires (RFC 3339).
    pub exp: String,
}

/// A parent AIA's signed endorsement of a subordinate AIA.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityEndorsement {
    pub claims: EndorsementClaims,
    /// Base64 Ed25519 signature over the canonical claims bytes.
    pub signature: String,
}

impl AuthorityEndorsement {
    /// Verify the endorsement was signed by `parent_key` and is valid at `now`.
    pub fn verify(&self, parent_key: &AgentPublicKey, now: DateTime<Utc>) -> Result<()> {
        verify_payload(parent_key, &self.claims, &self.signature)?;
        if now >= parse_rfc3339(&self.claims.exp)? {
            return Err(IdentityError::Expired);
        }
        Ok(())
    }
}

/// The published trust chain for an AIA.
///
/// A root AIA is its own trust anchor (`endorsement` is `None`). A subordinate
/// AIA carries the endorsement that binds its key to the root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustChain {
    /// This AIA's identifier.
    pub authority_id: String,
    /// This AIA's public key as a `did:key` string.
    pub authority_key: String,
    /// Endorsement from the root, absent when this AIA is itself the root.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endorsement: Option<AuthorityEndorsement>,
}

impl TrustChain {
    /// Resolve and verify this AIA's public key against a trusted root.
    ///
    /// For a root AIA the chain is trusted only if its key equals
    /// `trusted_root`. For a subordinate AIA the endorsement must be signed by
    /// `trusted_root`, unexpired at `now`, and bound to this AIA's identity.
    pub fn resolve(
        &self,
        trusted_root: &AgentPublicKey,
        now: DateTime<Utc>,
    ) -> Result<AgentPublicKey> {
        let authority_key = AgentPublicKey::from_did_key(&self.authority_key)?;
        match &self.endorsement {
            None => {
                if authority_key.to_bytes() != trusted_root.to_bytes() {
                    return Err(IdentityError::UntrustedIssuer(self.authority_id.clone()));
                }
            }
            Some(endorsement) => {
                endorsement.verify(trusted_root, now)?;
                if endorsement.claims.child_key != self.authority_key {
                    return Err(IdentityError::Malformed(
                        "endorsement child_key does not match authority_key".into(),
                    ));
                }
                if endorsement.claims.child_id != self.authority_id {
                    return Err(IdentityError::Malformed(
                        "endorsement child_id does not match authority_id".into(),
                    ));
                }
            }
        }
        Ok(authority_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::IdentityAuthority;
    use arkavo_crypto::AgentKeypair;
    use chrono::Duration;

    #[test]
    fn root_chain_resolves_to_its_own_key() {
        let root = IdentityAuthority::new_root("agent:root-authority");
        let chain = root.trust_chain();
        assert!(chain.endorsement.is_none());
        let resolved = chain.resolve(&root.verifying_key(), Utc::now()).unwrap();
        assert_eq!(resolved.to_bytes(), root.verifying_key().to_bytes());
    }

    #[test]
    fn root_chain_rejects_a_different_anchor() {
        let root = IdentityAuthority::new_root("agent:root-authority");
        let chain = root.trust_chain();
        let other = AgentKeypair::generate();
        let err = chain.resolve(&other.public_key(), Utc::now()).unwrap_err();
        assert!(matches!(err, IdentityError::UntrustedIssuer(_)));
    }

    #[test]
    fn subordinate_chain_resolves_through_the_root() {
        let root = IdentityAuthority::new_root("agent:root-authority");
        let swarm_key = AgentKeypair::generate();
        let endorsement = root
            .endorse_subordinate(
                "agent:swarm-authority",
                &swarm_key.public_key(),
                Duration::days(30),
            )
            .unwrap();
        let swarm =
            IdentityAuthority::new_subordinate("agent:swarm-authority", swarm_key, endorsement);
        let chain = swarm.trust_chain();
        let resolved = chain.resolve(&root.verifying_key(), Utc::now()).unwrap();
        assert_eq!(resolved.to_bytes(), swarm.verifying_key().to_bytes());
    }

    #[test]
    fn subordinate_chain_rejects_endorsement_from_an_untrusted_root() {
        let root = IdentityAuthority::new_root("agent:root-authority");
        let impostor = IdentityAuthority::new_root("agent:impostor");
        let swarm_key = AgentKeypair::generate();
        let endorsement = root
            .endorse_subordinate(
                "agent:swarm-authority",
                &swarm_key.public_key(),
                Duration::days(30),
            )
            .unwrap();
        let swarm =
            IdentityAuthority::new_subordinate("agent:swarm-authority", swarm_key, endorsement);
        let err = swarm
            .trust_chain()
            .resolve(&impostor.verifying_key(), Utc::now())
            .unwrap_err();
        assert!(matches!(err, IdentityError::InvalidSignature));
    }

    #[test]
    fn expired_endorsement_breaks_the_chain() {
        let root = IdentityAuthority::new_root("agent:root-authority");
        let swarm_key = AgentKeypair::generate();
        let endorsement = root
            .endorse_subordinate(
                "agent:swarm-authority",
                &swarm_key.public_key(),
                Duration::seconds(-1),
            )
            .unwrap();
        let swarm =
            IdentityAuthority::new_subordinate("agent:swarm-authority", swarm_key, endorsement);
        let err = swarm
            .trust_chain()
            .resolve(&root.verifying_key(), Utc::now())
            .unwrap_err();
        assert!(matches!(err, IdentityError::Expired));
    }

    #[test]
    fn rebound_endorsement_is_detected() {
        let root = IdentityAuthority::new_root("agent:root-authority");
        let swarm_key = AgentKeypair::generate();
        let endorsement = root
            .endorse_subordinate(
                "agent:swarm-authority",
                &swarm_key.public_key(),
                Duration::days(30),
            )
            .unwrap();
        // Attacker keeps a valid endorsement but swaps in their own key.
        let mut chain =
            IdentityAuthority::new_subordinate("agent:swarm-authority", swarm_key, endorsement)
                .trust_chain();
        chain.authority_key = AgentKeypair::generate().public_key().to_did_key();
        let err = chain
            .resolve(&root.verifying_key(), Utc::now())
            .unwrap_err();
        assert!(matches!(err, IdentityError::Malformed(_)));
    }
}
