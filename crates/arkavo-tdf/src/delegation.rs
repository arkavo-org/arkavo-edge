//! NTDF delegation token verification for A2A KAS.
//!
//! Implements delegation token chain verification for attribute-based
//! access control. Tokens form a chain of trust from a root authority
//! down to the requesting agent.

use base64::{Engine as _, engine::general_purpose};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors that can occur during delegation verification.
#[derive(Error, Debug)]
pub enum DelegationError {
    /// Token has expired
    #[error("Token expired at {0}")]
    Expired(DateTime<Utc>),

    /// Subject DID does not match caller
    #[error("Subject mismatch: expected {expected}, got {actual}")]
    SubjectMismatch { expected: String, actual: String },

    /// Signature verification failed
    #[error("Signature verification failed: {0}")]
    SignatureInvalid(String),

    /// Invalid issuer DID format
    #[error("Invalid issuer DID: {0}")]
    InvalidIssuerDid(String),

    /// Token parsing failed
    #[error("Token parse error: {0}")]
    ParseError(String),

    /// Chain of trust broken
    #[error("Delegation chain broken at depth {depth}: {reason}")]
    ChainBroken { depth: u32, reason: String },

    /// No trusted root found
    #[error("No trusted root in delegation chain")]
    NoTrustedRoot,
}

/// A single delegation token in the NTDF chain.
///
/// Tokens are linked via the `parent` field, forming a chain from the
/// root authority to the final subject (the requesting agent).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegationToken {
    /// DID of the entity that issued this token (DID:key format)
    pub issuer_did: String,

    /// DID of the entity this token grants access to
    pub subject_did: String,

    /// Entitlement FQNs granted by this token
    /// e.g., `["https://arkavo.net/attr/role/value/admin"]`
    pub entitlements: Vec<String>,

    /// Expiration timestamp
    pub expires_at: DateTime<Utc>,

    /// Ed25519 signature of the token payload (base64)
    pub signature: String,

    /// Parent token in the delegation chain (if not root)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<Box<DelegationToken>>,
}

impl DelegationToken {
    /// Compute the canonical payload bytes for signing.
    fn payload_bytes(&self) -> Result<Vec<u8>, DelegationError> {
        let payload = serde_json::json!({
            "issuer_did": self.issuer_did,
            "subject_did": self.subject_did,
            "entitlements": self.entitlements,
            "expires_at": self.expires_at.to_rfc3339(),
        });
        serde_json::to_vec(&payload)
            .map_err(|e| DelegationError::ParseError(format!("payload serialization: {e}")))
    }

    /// Parse a delegation token from a JSON string.
    pub fn from_json(json: &str) -> Result<Self, DelegationError> {
        serde_json::from_str(json).map_err(|e| DelegationError::ParseError(e.to_string()))
    }

    /// Serialize the token to a JSON string.
    pub fn to_json(&self) -> Result<String, DelegationError> {
        serde_json::to_string(self)
            .map_err(|e| DelegationError::ParseError(format!("token serialization: {e}")))
    }
}

/// Verifier for NTDF delegation token chains.
///
/// Maintains a list of trusted root public keys and verifies that
/// token chains terminate at a trusted root.
pub struct DelegationVerifier {
    trusted_roots: Vec<TrustedRoot>,
}

/// A trusted root authority for delegation chains.
#[derive(Debug, Clone)]
pub struct TrustedRoot {
    /// DID:key identifier
    pub did: String,
    /// Ed25519 public key bytes
    pub public_key_bytes: Vec<u8>,
}

impl DelegationVerifier {
    /// Create a new verifier with the given trusted roots.
    pub fn new(trusted_roots: Vec<TrustedRoot>) -> Self {
        Self { trusted_roots }
    }

    /// Create an empty verifier (trusts no roots).
    pub fn empty() -> Self {
        Self {
            trusted_roots: Vec::new(),
        }
    }

    /// Add a trusted root to the verifier.
    pub fn add_trusted_root(&mut self, root: TrustedRoot) {
        self.trusted_roots.push(root);
    }

    /// Verify a delegation token chain and return the granted entitlements.
    ///
    /// Verification checks:
    /// 1. `caller_did` matches the subject of the leaf token
    /// 2. All tokens in the chain are not expired
    /// 3. All signatures are valid (issuer signed the payload)
    /// 4. Parent-child subject/issuer relationships are valid
    /// 5. Chain terminates at a trusted root
    ///
    /// Returns the intersection of entitlements through the chain.
    pub fn verify(
        &self,
        token: &DelegationToken,
        caller_did: &str,
    ) -> Result<Vec<String>, DelegationError> {
        // Check caller matches leaf subject
        if token.subject_did != caller_did {
            return Err(DelegationError::SubjectMismatch {
                expected: token.subject_did.clone(),
                actual: caller_did.to_string(),
            });
        }

        // Verify the chain
        let mut current = token;
        let mut depth = 0u32;
        let mut entitlements = token.entitlements.clone();

        loop {
            // Check expiration
            let now = Utc::now();
            if current.expires_at < now {
                return Err(DelegationError::Expired(current.expires_at));
            }

            // Verify signature using issuer's DID:key
            self.verify_signature(current, depth)?;

            // Check for trusted root
            if self.is_trusted_root(&current.issuer_did) {
                return Ok(entitlements);
            }

            // Move to parent
            match &current.parent {
                Some(parent) => {
                    // Verify parent's subject matches current's issuer
                    if parent.subject_did != current.issuer_did {
                        return Err(DelegationError::ChainBroken {
                            depth,
                            reason: format!(
                                "Parent subject {} != issuer {}",
                                parent.subject_did, current.issuer_did
                            ),
                        });
                    }

                    // Intersect entitlements (child can only narrow, not expand)
                    entitlements = intersect_entitlements(&entitlements, &parent.entitlements);

                    current = parent;
                    depth += 1;
                }
                None => {
                    // Reached end of chain without finding trusted root
                    return Err(DelegationError::NoTrustedRoot);
                }
            }
        }
    }

    /// Check if a DID is in the trusted roots list.
    fn is_trusted_root(&self, did: &str) -> bool {
        self.trusted_roots.iter().any(|r| r.did == did)
    }

    /// Verify the signature on a token using the issuer's DID:key.
    fn verify_signature(&self, token: &DelegationToken, depth: u32) -> Result<(), DelegationError> {
        // Extract public key from DID:key
        let public_key = arkavo_crypto::AgentPublicKey::from_did_key(&token.issuer_did)
            .map_err(|e| DelegationError::InvalidIssuerDid(e.to_string()))?;

        // Decode signature from base64
        let signature_bytes = general_purpose::STANDARD
            .decode(&token.signature)
            .map_err(|e| DelegationError::SignatureInvalid(format!("Base64 decode error: {e}")))?;

        // Verify Ed25519 signature
        let payload = token.payload_bytes()?;
        public_key
            .verify(&payload, &signature_bytes)
            .map_err(|e| DelegationError::ChainBroken {
                depth,
                reason: format!("Signature verification failed: {e}"),
            })
    }
}

/// Compute the intersection of two entitlement sets.
fn intersect_entitlements(a: &[String], b: &[String]) -> Vec<String> {
    a.iter().filter(|ent| b.contains(ent)).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use arkavo_test_macros::spec;

    fn make_test_token(
        issuer: &str,
        subject: &str,
        entitlements: &[&str],
        expires_in_secs: i64,
    ) -> DelegationToken {
        DelegationToken {
            issuer_did: issuer.to_string(),
            subject_did: subject.to_string(),
            entitlements: entitlements.iter().map(|s| (*s).to_string()).collect(),
            expires_at: Utc::now() + chrono::Duration::seconds(expires_in_secs),
            signature: String::new(),
            parent: None,
        }
    }

    #[spec("TDFS-008")]
    #[test]
    fn test_delegation_token_serialization() {
        let token = make_test_token(
            "did:key:z6MkTest",
            "did:key:z6MkSubject",
            &["https://arkavo.net/attr/role/value/admin"],
            3600,
        );

        let json = token.to_json().unwrap();
        let parsed = DelegationToken::from_json(&json).unwrap();

        assert_eq!(parsed.issuer_did, token.issuer_did);
        assert_eq!(parsed.subject_did, token.subject_did);
        assert_eq!(parsed.entitlements, token.entitlements);
    }

    #[spec("TDFS-008")]
    #[test]
    fn test_subject_mismatch() {
        let token = make_test_token(
            "did:key:z6MkIssuer",
            "did:key:z6MkSubject",
            &["https://arkavo.net/attr/role/value/user"],
            3600,
        );

        let verifier = DelegationVerifier::empty();
        let result = verifier.verify(&token, "did:key:z6MkWrongCaller");

        assert!(matches!(
            result,
            Err(DelegationError::SubjectMismatch { .. })
        ));
    }

    #[spec("TDFS-009")]
    #[test]
    fn test_expired_token() {
        let mut token = make_test_token(
            "did:key:z6MkIssuer",
            "did:key:z6MkSubject",
            &["https://arkavo.net/attr/role/value/user"],
            -3600, // Expired 1 hour ago
        );
        // Set a valid-looking signature to get past initial checks
        token.signature = general_purpose::STANDARD.encode(vec![0u8; 64]);

        let verifier = DelegationVerifier::empty();
        let result = verifier.verify(&token, "did:key:z6MkSubject");

        assert!(matches!(result, Err(DelegationError::Expired(_))));
    }

    #[spec("TDFS-009")]
    #[test]
    fn test_expired_parent_token_in_chain() {
        use arkavo_crypto::AgentKeypair;

        // Root authority issues an expired delegation to an intermediate.
        let root_key = AgentKeypair::generate();
        let root_did = root_key.public_key().to_did_key();
        let intermediate_key = AgentKeypair::generate();
        let intermediate_did = intermediate_key.public_key().to_did_key();
        let caller_did = "did:key:z6MkCaller";

        let mut parent = DelegationToken {
            issuer_did: root_did.clone(),
            subject_did: intermediate_did.clone(),
            entitlements: vec!["https://arkavo.net/attr/role/value/user".to_string()],
            expires_at: Utc::now() - chrono::Duration::seconds(3600),
            signature: String::new(),
            parent: None,
        };
        let parent_payload = parent.payload_bytes().unwrap();
        parent.signature = general_purpose::STANDARD.encode(root_key.sign(&parent_payload));

        // Leaf token is valid, but its parent has expired.
        let mut leaf = DelegationToken {
            issuer_did: intermediate_did,
            subject_did: caller_did.to_string(),
            entitlements: vec!["https://arkavo.net/attr/role/value/user".to_string()],
            expires_at: Utc::now() + chrono::Duration::seconds(3600),
            signature: String::new(),
            parent: Some(Box::new(parent)),
        };
        let leaf_payload = leaf.payload_bytes().unwrap();
        leaf.signature = general_purpose::STANDARD.encode(intermediate_key.sign(&leaf_payload));

        let verifier = DelegationVerifier::new(vec![TrustedRoot {
            did: root_did,
            public_key_bytes: root_key.public_key().to_bytes(),
        }]);
        let result = verifier.verify(&leaf, caller_did);

        assert!(matches!(result, Err(DelegationError::Expired(_))));
    }

    #[spec("TDFS-008")]
    #[test]
    fn test_entitlement_intersection() {
        let a = vec![
            "https://arkavo.net/attr/role/value/admin".to_string(),
            "https://arkavo.net/attr/role/value/user".to_string(),
            "https://arkavo.net/attr/clearance/value/secret".to_string(),
        ];
        let b = vec![
            "https://arkavo.net/attr/role/value/user".to_string(),
            "https://arkavo.net/attr/clearance/value/secret".to_string(),
            "https://arkavo.net/attr/department/value/engineering".to_string(),
        ];

        let result = intersect_entitlements(&a, &b);

        assert_eq!(result.len(), 2);
        assert!(result.contains(&"https://arkavo.net/attr/role/value/user".to_string()));
        assert!(result.contains(&"https://arkavo.net/attr/clearance/value/secret".to_string()));
    }

    #[spec("TDFS-008")]
    #[test]
    fn test_trusted_root_check() {
        let root = TrustedRoot {
            did: "did:key:z6MkRoot".to_string(),
            public_key_bytes: vec![],
        };

        let verifier = DelegationVerifier::new(vec![root]);

        assert!(verifier.is_trusted_root("did:key:z6MkRoot"));
        assert!(!verifier.is_trusted_root("did:key:z6MkOther"));
    }

    #[spec("TDFS-008")]
    #[test]
    fn test_payload_bytes_deterministic() {
        let token = make_test_token(
            "did:key:z6MkIssuer",
            "did:key:z6MkSubject",
            &["https://arkavo.net/attr/role/value/admin"],
            3600,
        );

        let bytes1 = token.payload_bytes().unwrap();
        let bytes2 = token.payload_bytes().unwrap();

        assert_eq!(bytes1, bytes2);
    }
}
