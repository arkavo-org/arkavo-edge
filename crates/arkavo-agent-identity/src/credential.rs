//! The signed identity credential and its verification.

use crate::document::IdentityDocument;
use crate::{IdentityError, Result, verify_payload};
use arkavo_crypto::AgentPublicKey;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// An identity document signed by the issuing AIA.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityCredential {
    /// The signed identity claims.
    pub document: IdentityDocument,
    /// Base64 Ed25519 signature over the canonical document bytes.
    pub signature: String,
}

impl IdentityCredential {
    /// Verify the credential against a trusted authority key.
    ///
    /// Checks the AIA signature and that the credential has not expired at
    /// `now`. The caller must obtain `authority_key` from a trusted
    /// [`TrustChain`] — never from the credential or its `iss` claim alone.
    ///
    /// [`TrustChain`]: crate::TrustChain
    pub fn verify(
        &self,
        authority_key: &AgentPublicKey,
        now: DateTime<Utc>,
    ) -> Result<&IdentityDocument> {
        verify_payload(authority_key, &self.document, &self.signature)?;
        if self.document.is_expired(now)? {
            return Err(IdentityError::Expired);
        }
        Ok(&self.document)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Runtime, TrustLevel};
    use crate::{IdentityAuthority, IssueRequest};
    use arkavo_crypto::AgentKeypair;
    use chrono::Duration;

    fn issue() -> (IdentityAuthority, IdentityCredential, AgentKeypair) {
        let aia = IdentityAuthority::new_root("agent:id-authority");
        let agent = AgentKeypair::generate();
        let credential = aia
            .issue(IssueRequest {
                subject: "agent:worker-17".into(),
                agent_type: "coding".into(),
                agent_public_key: agent.public_key(),
                runtime: Runtime::Local,
                capabilities: vec!["a2a".into(), "mcp".into()],
                owner: None,
                organization: None,
                validity: Duration::hours(1),
                attestation: None,
            })
            .unwrap();
        (aia, credential, agent)
    }

    #[test]
    fn valid_credential_verifies() {
        let (aia, credential, _) = issue();
        let document = credential
            .verify(&aia.verifying_key(), Utc::now())
            .expect("verify");
        assert_eq!(document.sub, "agent:worker-17");
        assert_eq!(document.trust_level, TrustLevel::Unattested);
    }

    #[test]
    fn tampered_document_fails_verification() {
        let (aia, mut credential, _) = issue();
        credential.document.agent_type = "exfiltration".into();
        let err = credential
            .verify(&aia.verifying_key(), Utc::now())
            .unwrap_err();
        assert!(matches!(err, IdentityError::InvalidSignature));
    }

    #[test]
    fn wrong_authority_key_fails_verification() {
        let (_, credential, _) = issue();
        let impostor = AgentKeypair::generate();
        let err = credential
            .verify(&impostor.public_key(), Utc::now())
            .unwrap_err();
        assert!(matches!(err, IdentityError::InvalidSignature));
    }

    #[test]
    fn expired_credential_is_rejected() {
        let aia = IdentityAuthority::new_root("agent:id-authority");
        let agent = AgentKeypair::generate();
        let credential = aia
            .issue(IssueRequest {
                subject: "agent:worker-17".into(),
                agent_type: "coding".into(),
                agent_public_key: agent.public_key(),
                runtime: Runtime::Local,
                capabilities: vec![],
                owner: None,
                organization: None,
                validity: Duration::seconds(-1),
                attestation: None,
            })
            .unwrap();
        let err = credential
            .verify(&aia.verifying_key(), Utc::now())
            .unwrap_err();
        assert!(matches!(err, IdentityError::Expired));
    }

    #[test]
    fn credential_survives_json_round_trip() {
        let (aia, credential, _) = issue();
        let json = serde_json::to_string(&credential).unwrap();
        let restored: IdentityCredential = serde_json::from_str(&json).unwrap();
        assert!(restored.verify(&aia.verifying_key(), Utc::now()).is_ok());
    }
}
