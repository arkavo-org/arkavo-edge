use arkavo_crypto::{AgentKeypair, AgentPublicKey, CryptoError};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Scoped capabilities that can be granted to agents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapScope {
    /// Propose code edits (create OpBundles)
    Edit,
    /// Run tests in isolated workspace
    Test,
    /// Verify OpBundles via Critic pipeline
    Verify,
    /// Merge verified OpBundles into canonical AST
    Merge,
}

/// A signed capability token granting specific permissions to an agent.
///
/// Tokens are issued by agents who already hold the granted capabilities
/// (no self-escalation). Ed25519 signatures ensure authenticity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CapabilityToken {
    pub holder: String,
    pub scopes: BTreeSet<CapScope>,
    pub issued_by: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[serde(with = "hex_bytes")]
    pub signature: Vec<u8>,
}

impl CapabilityToken {
    /// Create a new unsigned token.
    pub fn new(
        holder: String,
        scopes: BTreeSet<CapScope>,
        issued_by: String,
        expires_at: DateTime<Utc>,
    ) -> Self {
        Self {
            holder,
            scopes,
            issued_by,
            issued_at: Utc::now(),
            expires_at,
            signature: Vec::new(),
        }
    }

    /// Bytes to sign/verify — deterministic serialization of all fields except signature.
    pub fn content_to_sign(&self) -> Vec<u8> {
        let mut content = Vec::new();
        content.extend(self.holder.as_bytes());
        for scope in &self.scopes {
            content.extend(scope_tag(*scope));
        }
        content.extend(self.issued_by.as_bytes());
        content.extend(self.issued_at.timestamp().to_le_bytes());
        content.extend(self.expires_at.timestamp().to_le_bytes());
        content
    }

    /// Sign this token with the issuer's keypair.
    pub fn sign(&mut self, keypair: &AgentKeypair) {
        let content = self.content_to_sign();
        self.signature = keypair.sign(&content);
    }

    /// Verify the signature against the issuer's public key.
    pub fn verify(&self, pubkey: &AgentPublicKey) -> Result<(), CryptoError> {
        let content = self.content_to_sign();
        pubkey.verify(&content, &self.signature)
    }

    /// Check if this token grants the given capability.
    pub fn has_scope(&self, scope: CapScope) -> bool {
        self.scopes.contains(&scope)
    }

    /// Check if this token has expired.
    pub fn is_expired(&self) -> bool {
        Utc::now() > self.expires_at
    }

    /// Validate that the issuer token has the authority to issue the granted scopes.
    ///
    /// Enforces no-self-escalation: issuer must hold all scopes being granted.
    pub fn validate_issuance(
        issuer_scopes: &BTreeSet<CapScope>,
        granted_scopes: &BTreeSet<CapScope>,
    ) -> Result<(), String> {
        for scope in granted_scopes {
            if !issuer_scopes.contains(scope) {
                return Err(format!(
                    "issuer lacks {scope:?} capability — cannot grant it"
                ));
            }
        }
        Ok(())
    }
}

fn scope_tag(scope: CapScope) -> &'static [u8] {
    match scope {
        CapScope::Edit => b"edit",
        CapScope::Test => b"test",
        CapScope::Verify => b"verify",
        CapScope::Merge => b"merge",
    }
}

mod hex_bytes {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(bytes: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(bytes))
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        let s = String::deserialize(d)?;
        hex::decode(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn make_token(scopes: &[CapScope]) -> CapabilityToken {
        CapabilityToken::new(
            "agent-1".into(),
            scopes.iter().copied().collect(),
            "root".into(),
            Utc::now() + chrono::Duration::hours(1),
        )
    }

    #[spec("EVO-005")]
    #[test]
    fn sign_and_verify() {
        let keypair = AgentKeypair::generate();
        let mut token = make_token(&[CapScope::Edit, CapScope::Test]);
        token.sign(&keypair);
        assert!(token.verify(&keypair.public_key()).is_ok());
    }

    #[spec("EVO-005")]
    #[test]
    fn verify_fails_wrong_key() {
        let keypair = AgentKeypair::generate();
        let other = AgentKeypair::generate();
        let mut token = make_token(&[CapScope::Edit]);
        token.sign(&keypair);
        assert!(token.verify(&other.public_key()).is_err());
    }

    #[spec("EVO-005")]
    #[test]
    fn has_scope_checks() {
        let token = make_token(&[CapScope::Edit, CapScope::Verify]);
        assert!(token.has_scope(CapScope::Edit));
        assert!(token.has_scope(CapScope::Verify));
        assert!(!token.has_scope(CapScope::Merge));
        assert!(!token.has_scope(CapScope::Test));
    }

    #[spec("EVO-005")]
    #[test]
    fn expired_token() {
        let mut token = make_token(&[CapScope::Edit]);
        token.expires_at = Utc::now() - chrono::Duration::hours(1);
        assert!(token.is_expired());
    }

    #[spec("EVO-005")]
    #[test]
    fn not_expired_token() {
        let token = make_token(&[CapScope::Edit]);
        assert!(!token.is_expired());
    }

    #[spec("EVO-005")]
    #[test]
    fn validate_issuance_ok() {
        let issuer: BTreeSet<_> = [CapScope::Edit, CapScope::Test, CapScope::Merge]
            .into_iter()
            .collect();
        let granted: BTreeSet<_> = [CapScope::Edit, CapScope::Test].into_iter().collect();
        assert!(CapabilityToken::validate_issuance(&issuer, &granted).is_ok());
    }

    #[spec("EVO-005")]
    #[test]
    fn validate_issuance_escalation_denied() {
        let issuer: BTreeSet<_> = [CapScope::Edit].into_iter().collect();
        let granted: BTreeSet<_> = [CapScope::Edit, CapScope::Merge].into_iter().collect();
        let err = CapabilityToken::validate_issuance(&issuer, &granted).unwrap_err();
        assert!(err.contains("Merge"));
    }

    #[spec("EVO-005")]
    #[test]
    fn content_to_sign_deterministic() {
        let token = make_token(&[CapScope::Edit, CapScope::Test]);
        assert_eq!(token.content_to_sign(), token.content_to_sign());
    }

    #[spec("EVO-005")]
    #[test]
    fn content_to_sign_differs_by_scope() {
        let a = make_token(&[CapScope::Edit]);
        let b = make_token(&[CapScope::Test]);
        assert_ne!(a.content_to_sign(), b.content_to_sign());
    }

    #[spec("EVO-005")]
    #[test]
    fn serde_roundtrip() {
        let keypair = AgentKeypair::generate();
        let mut token = make_token(&[CapScope::Edit, CapScope::Verify]);
        token.sign(&keypair);
        let json = serde_json::to_string(&token).unwrap();
        let restored: CapabilityToken = serde_json::from_str(&json).unwrap();
        assert!(restored.verify(&keypair.public_key()).is_ok());
    }
}
