//! Signed revocation lists for compromised agents.

use crate::{Result, verify_payload};
use arkavo_crypto::AgentPublicKey;
use serde::{Deserialize, Serialize};

/// A revoked agent and the circumstances of its revocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevokedEntry {
    /// Subject identifier of the revoked agent.
    pub sub: String,
    /// When the revocation took effect (RFC 3339).
    pub revoked_at: String,
    /// Optional human-readable reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The signed claims of a revocation list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationClaims {
    /// Issuing authority identifier.
    pub authority_id: String,
    /// When the list was published (RFC 3339).
    pub issued_at: String,
    /// Every agent the authority has revoked.
    pub revoked: Vec<RevokedEntry>,
}

/// A revocation list signed by the issuing AIA.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevocationList {
    pub claims: RevocationClaims,
    /// Base64 Ed25519 signature over the canonical claims bytes.
    pub signature: String,
}

impl RevocationList {
    /// Whether `subject` appears on this list.
    pub fn is_revoked(&self, subject: &str) -> bool {
        self.claims.revoked.iter().any(|e| e.sub == subject)
    }

    /// Verify the list was signed by the given authority key.
    pub fn verify(&self, authority_key: &AgentPublicKey) -> Result<()> {
        verify_payload(authority_key, &self.claims, &self.signature)
    }
}

#[cfg(test)]
mod tests {
    use crate::{IdentityAuthority, IdentityError};
    use arkavo_crypto::AgentKeypair;

    #[test]
    fn revocation_list_records_and_verifies() {
        let mut aia = IdentityAuthority::new_root("agent:id-authority");
        aia.revoke("agent:worker-17", Some("key compromise".into()));
        aia.revoke("agent:worker-31", None);
        let list = aia.revocation_list().unwrap();

        assert!(list.is_revoked("agent:worker-17"));
        assert!(list.is_revoked("agent:worker-31"));
        assert!(!list.is_revoked("agent:worker-99"));
        assert!(list.verify(&aia.verifying_key()).is_ok());
    }

    #[test]
    fn revoke_is_idempotent() {
        let mut aia = IdentityAuthority::new_root("agent:id-authority");
        aia.revoke("agent:worker-17", Some("first".into()));
        aia.revoke("agent:worker-17", Some("second".into()));
        let list = aia.revocation_list().unwrap();
        assert_eq!(list.claims.revoked.len(), 1);
        assert_eq!(list.claims.revoked[0].reason.as_deref(), Some("first"));
    }

    #[test]
    fn tampered_list_fails_verification() {
        let mut aia = IdentityAuthority::new_root("agent:id-authority");
        aia.revoke("agent:worker-17", None);
        let mut list = aia.revocation_list().unwrap();
        list.claims.revoked.clear();
        let err = list.verify(&aia.verifying_key()).unwrap_err();
        assert!(matches!(err, IdentityError::InvalidSignature));
    }

    #[test]
    fn list_signed_by_another_authority_is_rejected() {
        let mut aia = IdentityAuthority::new_root("agent:id-authority");
        aia.revoke("agent:worker-17", None);
        let list = aia.revocation_list().unwrap();
        let other = AgentKeypair::generate();
        assert!(list.verify(&other.public_key()).is_err());
    }
}
