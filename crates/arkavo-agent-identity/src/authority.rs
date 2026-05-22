//! The Agent Identity Authority — the role that issues and attests identity.

use crate::credential::IdentityCredential;
use crate::document::{IdentityDocument, Runtime, TrustLevel, format_rfc3339};
use crate::revocation::{RevocationClaims, RevocationList, RevokedEntry};
use crate::trust_chain::{AuthorityEndorsement, EndorsementClaims, TrustChain};
use crate::{IdentityError, Result, sign_payload};
use arkavo_attestation::{ModelEvidence, PlatformEvidence, SecurityState};
use arkavo_crypto::{AgentKeypair, AgentPublicKey};
use chrono::{Duration, Utc};

/// Runtime and model attestation evidence supplied with an issue request.
///
/// Evidence is the *only* way an issued credential reaches
/// [`TrustLevel::Attested`]: the AIA never accepts a self-asserted trust level.
#[derive(Debug, Clone, Default)]
pub struct AttestationEvidence {
    /// Evidence binding the agent to specific model weights.
    pub model: Option<ModelEvidence>,
    /// Evidence describing the platform the agent runs on.
    pub platform: Option<PlatformEvidence>,
}

/// A request to issue an identity credential for an agent.
#[derive(Debug)]
pub struct IssueRequest {
    /// Subject agent identifier, e.g. `agent:worker-17`.
    pub subject: String,
    /// Free-form agent role/type, e.g. `coding`.
    pub agent_type: String,
    /// The agent's own public key.
    pub agent_public_key: AgentPublicKey,
    /// Where the agent runs.
    pub runtime: Runtime,
    /// Protocols the agent speaks.
    pub capabilities: Vec<String>,
    /// Owning principal.
    pub owner: Option<String>,
    /// Owning organization.
    pub organization: Option<String>,
    /// Credential lifetime.
    pub validity: Duration,
    /// Optional runtime/model attestation evidence.
    pub attestation: Option<AttestationEvidence>,
}

/// The Agent Identity Authority.
///
/// Its sole responsibilities are creating identities, issuing credentials,
/// rotating keys, attesting runtime identity, publishing a [`TrustChain`], and
/// revoking agents. It assigns no work, grants no resource access, makes no
/// ABAC decisions, delegates no authority, and releases no TDF keys.
pub struct IdentityAuthority {
    authority_id: String,
    keypair: AgentKeypair,
    endorsement: Option<AuthorityEndorsement>,
    revoked: Vec<RevokedEntry>,
}

impl IdentityAuthority {
    /// Create a root AIA — its own trust anchor — with a freshly generated key.
    pub fn new_root(authority_id: impl Into<String>) -> Self {
        Self::from_keypair(authority_id, AgentKeypair::generate())
    }

    /// Create a root AIA from an existing keypair.
    pub fn from_keypair(authority_id: impl Into<String>, keypair: AgentKeypair) -> Self {
        Self {
            authority_id: authority_id.into(),
            keypair,
            endorsement: None,
            revoked: Vec::new(),
        }
    }

    /// Create a subordinate AIA that carries a parent's endorsement.
    pub fn new_subordinate(
        authority_id: impl Into<String>,
        keypair: AgentKeypair,
        endorsement: AuthorityEndorsement,
    ) -> Self {
        Self {
            authority_id: authority_id.into(),
            keypair,
            endorsement: Some(endorsement),
            revoked: Vec::new(),
        }
    }

    /// This authority's identifier (the `iss` claim it issues under).
    pub fn authority_id(&self) -> &str {
        &self.authority_id
    }

    /// This authority's public key — the key credentials must be verified with.
    pub fn verifying_key(&self) -> AgentPublicKey {
        self.keypair.public_key()
    }

    /// Issue a signed identity credential for an agent.
    ///
    /// The trust level is derived from attestation evidence, never asserted by
    /// the caller. Compromised-platform evidence is refused outright.
    pub fn issue(&self, request: IssueRequest) -> Result<IdentityCredential> {
        let trust_level = evaluate_trust(request.attestation.as_ref())?;
        let model = request
            .attestation
            .as_ref()
            .and_then(|evidence| evidence.model.as_ref())
            .map(|model| model.model_id.clone());
        let (iat, exp) = lifetime(request.validity)?;
        let document = IdentityDocument {
            iss: self.authority_id.clone(),
            sub: request.subject,
            agent_type: request.agent_type,
            public_key: request.agent_public_key.to_did_key(),
            trust_level,
            runtime: request.runtime,
            model,
            owner: request.owner,
            organization: request.organization,
            capabilities: request.capabilities,
            iat,
            exp,
        };
        let signature = sign_payload(&self.keypair, &document)?;
        Ok(IdentityCredential {
            document,
            signature,
        })
    }

    /// Re-issue a credential for the same agent under a new key (key rotation).
    ///
    /// Every identity claim is preserved except the public key and lifetime.
    pub fn rotate(
        &self,
        current: &IdentityCredential,
        new_agent_key: &AgentPublicKey,
        validity: Duration,
    ) -> Result<IdentityCredential> {
        let prior = &current.document;
        let (iat, exp) = lifetime(validity)?;
        let document = IdentityDocument {
            iss: self.authority_id.clone(),
            sub: prior.sub.clone(),
            agent_type: prior.agent_type.clone(),
            public_key: new_agent_key.to_did_key(),
            trust_level: prior.trust_level,
            runtime: prior.runtime,
            model: prior.model.clone(),
            owner: prior.owner.clone(),
            organization: prior.organization.clone(),
            capabilities: prior.capabilities.clone(),
            iat,
            exp,
        };
        let signature = sign_payload(&self.keypair, &document)?;
        Ok(IdentityCredential {
            document,
            signature,
        })
    }

    /// Revoke an agent. Idempotent: a re-revoked agent keeps its first entry.
    pub fn revoke(&mut self, subject: impl Into<String>, reason: Option<String>) {
        let subject = subject.into();
        if self.revoked.iter().any(|entry| entry.sub == subject) {
            return;
        }
        self.revoked.push(RevokedEntry {
            sub: subject,
            revoked_at: format_rfc3339(Utc::now()),
            reason,
        });
    }

    /// Publish the current revocation list, signed by this authority.
    pub fn revocation_list(&self) -> Result<RevocationList> {
        let claims = RevocationClaims {
            authority_id: self.authority_id.clone(),
            issued_at: format_rfc3339(Utc::now()),
            revoked: self.revoked.clone(),
        };
        let signature = sign_payload(&self.keypair, &claims)?;
        Ok(RevocationList { claims, signature })
    }

    /// Endorse a subordinate AIA, binding its key into this authority's chain.
    pub fn endorse_subordinate(
        &self,
        child_id: impl Into<String>,
        child_key: &AgentPublicKey,
        validity: Duration,
    ) -> Result<AuthorityEndorsement> {
        let (issued_at, exp) = lifetime(validity)?;
        let claims = EndorsementClaims {
            parent_id: self.authority_id.clone(),
            child_id: child_id.into(),
            child_key: child_key.to_did_key(),
            issued_at,
            exp,
        };
        let signature = sign_payload(&self.keypair, &claims)?;
        Ok(AuthorityEndorsement { claims, signature })
    }

    /// Publish this authority's trust chain.
    pub fn trust_chain(&self) -> TrustChain {
        TrustChain {
            authority_id: self.authority_id.clone(),
            authority_key: self.verifying_key().to_did_key(),
            endorsement: self.endorsement.clone(),
        }
    }
}

/// Compute `(issued_at, expiry)` RFC 3339 strings for a credential lifetime.
fn lifetime(validity: Duration) -> Result<(String, String)> {
    let now = Utc::now();
    let exp = now.checked_add_signed(validity).ok_or_else(|| {
        IdentityError::Malformed("validity overflows the credential clock".into())
    })?;
    Ok((format_rfc3339(now), format_rfc3339(exp)))
}

/// Derive a trust level from attestation evidence.
///
/// Compromised-platform evidence is a hard failure: the AIA will not attest a
/// runtime it has been told is compromised.
fn evaluate_trust(attestation: Option<&AttestationEvidence>) -> Result<TrustLevel> {
    let Some(evidence) = attestation else {
        return Ok(TrustLevel::Unattested);
    };
    if let Some(platform) = &evidence.platform
        && platform.platform_state == SecurityState::Compromised
    {
        return Err(IdentityError::CompromisedRuntime);
    }
    let platform_trusted = evidence
        .platform
        .as_ref()
        .is_some_and(|platform| platform.platform_state == SecurityState::Trusted);
    if evidence.model.is_some() || platform_trusted {
        Ok(TrustLevel::Attested)
    } else {
        Ok(TrustLevel::Unattested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_attestation::PlatformEvidence;
    use arkavo_device_identity::AgentIdentity;

    fn request(agent: &AgentKeypair) -> IssueRequest {
        IssueRequest {
            subject: "agent:worker-17".into(),
            agent_type: "coding".into(),
            agent_public_key: agent.public_key(),
            runtime: Runtime::Local,
            capabilities: vec!["a2a".into(), "mcp".into()],
            owner: Some("user:paul".into()),
            organization: Some("arkavo".into()),
            validity: Duration::days(10),
            attestation: None,
        }
    }

    fn platform_evidence(state: SecurityState) -> PlatformEvidence {
        let identity = AgentIdentity::new("0.77.0".to_string());
        let mut evidence = PlatformEvidence::from_agent_identity(identity, "linux-x86_64".into());
        evidence.platform_state = state;
        evidence
    }

    #[test]
    fn issued_credential_carries_the_requested_identity() {
        let aia = IdentityAuthority::new_root("agent:id-authority");
        let agent = AgentKeypair::generate();
        let credential = aia.issue(request(&agent)).unwrap();
        let document = &credential.document;
        assert_eq!(document.iss, "agent:id-authority");
        assert_eq!(document.sub, "agent:worker-17");
        assert_eq!(document.public_key, agent.public_key().to_did_key());
        assert_eq!(document.trust_level, TrustLevel::Unattested);
        assert_eq!(document.owner.as_deref(), Some("user:paul"));
    }

    #[test]
    fn attestation_evidence_drives_the_trust_level() {
        let aia = IdentityAuthority::new_root("agent:id-authority");
        let agent = AgentKeypair::generate();
        let mut req = request(&agent);
        req.attestation = Some(AttestationEvidence {
            model: None,
            platform: Some(platform_evidence(SecurityState::Trusted)),
        });
        let credential = aia.issue(req).unwrap();
        assert_eq!(credential.document.trust_level, TrustLevel::Attested);
    }

    #[test]
    fn suspicious_platform_yields_an_unattested_credential() {
        let aia = IdentityAuthority::new_root("agent:id-authority");
        let agent = AgentKeypair::generate();
        let mut req = request(&agent);
        req.attestation = Some(AttestationEvidence {
            model: None,
            platform: Some(platform_evidence(SecurityState::Suspicious)),
        });
        let credential = aia.issue(req).unwrap();
        assert_eq!(credential.document.trust_level, TrustLevel::Unattested);
    }

    #[test]
    fn compromised_platform_is_refused() {
        let aia = IdentityAuthority::new_root("agent:id-authority");
        let agent = AgentKeypair::generate();
        let mut req = request(&agent);
        req.attestation = Some(AttestationEvidence {
            model: None,
            platform: Some(platform_evidence(SecurityState::Compromised)),
        });
        let err = aia.issue(req).unwrap_err();
        assert!(matches!(err, IdentityError::CompromisedRuntime));
    }

    #[test]
    fn rotation_preserves_identity_but_swaps_the_key() {
        let aia = IdentityAuthority::new_root("agent:id-authority");
        let agent = AgentKeypair::generate();
        let original = aia.issue(request(&agent)).unwrap();
        let rotated_key = AgentKeypair::generate();
        let rotated = aia
            .rotate(&original, &rotated_key.public_key(), Duration::days(10))
            .unwrap();
        assert_eq!(rotated.document.sub, original.document.sub);
        assert_eq!(rotated.document.agent_type, original.document.agent_type);
        assert_eq!(
            rotated.document.capabilities,
            original.document.capabilities
        );
        assert_ne!(rotated.document.public_key, original.document.public_key);
        assert_eq!(
            rotated.document.public_key,
            rotated_key.public_key().to_did_key()
        );
        assert!(rotated.verify(&aia.verifying_key(), Utc::now()).is_ok());
    }
}
