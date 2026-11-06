//! Signing outbound requests.
//!
//! A signer never signs bytes it is handed. It signs a [`SignableOperation`], which only this
//! module can build: from a request, the profile's coverage for the operation, and fresh
//! parameters. The signer then checks the operation against its own policy (which
//! destinations, which operations) before signing, so it cannot be used as a general
//! signing oracle, by a model-generated tool call or anything else. It returns the signature
//! and public metadata, never key bytes.

use crate::base::signature_base;
use crate::context::RequestContext;
use crate::error::SignError;
use crate::fields::{self, SignatureParams};
use crate::profile::{Operation, Profile, Purpose};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ed25519_dalek::{Signer as _, SigningKey};
use rand::RngCore;
use rand::rngs::OsRng;
use std::fmt;

/// The label this implementation signs under.
pub const LABEL: &str = "tap";

/// A request ready to sign: its context, coverage, parameters and exact signature base.
#[derive(Debug, Clone)]
pub struct SignableOperation {
    context: RequestContext,
    profile: Profile,
    operation: Operation,
    params: SignatureParams,
    signature_input: String,
    base: Vec<u8>,
}

impl SignableOperation {
    pub fn context(&self) -> &RequestContext {
        &self.context
    }

    pub fn profile(&self) -> Profile {
        self.profile
    }

    pub fn operation(&self) -> Operation {
        self.operation
    }

    pub fn params(&self) -> &SignatureParams {
        &self.params
    }

    /// The exact bytes to sign.
    pub fn signature_base(&self) -> &[u8] {
        &self.base
    }
}

/// Something that signs prepared operations, enforcing its own destination policy.
pub trait TapSigner: Send + Sync {
    /// The key ID the signature input names.
    fn key_id(&self) -> &str;
    /// Signs `operation`'s signature base, or refuses.
    fn sign(&self, operation: &SignableOperation) -> Result<Vec<u8>, SignError>;
}

/// Which destinations and operations a signer accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SigningPolicy {
    /// Allowed merchant origins, `scheme://authority`.
    pub destinations: Vec<String>,
    pub operations: Vec<Operation>,
}

/// An Ed25519 key held in process memory, for local development and tests. Production
/// signing goes through the protected-key abstraction instead.
pub struct Ed25519Signer {
    key: SigningKey,
    key_id: String,
    policy: SigningPolicy,
}

impl fmt::Debug for Ed25519Signer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Ed25519Signer")
            .field("key_id", &self.key_id)
            .field("key", &"<redacted>")
            .field("policy", &self.policy)
            .finish()
    }
}

impl Ed25519Signer {
    pub fn new(secret: &[u8; 32], key_id: impl Into<String>, policy: SigningPolicy) -> Self {
        Ed25519Signer {
            key: SigningKey::from_bytes(secret),
            key_id: key_id.into(),
            policy,
        }
    }

    pub fn public_key(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }
}

impl TapSigner for Ed25519Signer {
    fn key_id(&self) -> &str {
        &self.key_id
    }

    fn sign(&self, operation: &SignableOperation) -> Result<Vec<u8>, SignError> {
        let destination = operation.context.origin().as_origin();
        if !self.policy.destinations.contains(&destination) {
            return Err(SignError::Refused(format!(
                "{destination} is not an allowed destination"
            )));
        }
        if !self.policy.operations.contains(&operation.operation) {
            return Err(SignError::Refused(format!(
                "{:?} is not an allowed operation",
                operation.operation
            )));
        }
        if operation.params.keyid != self.key_id {
            return Err(SignError::Refused("the operation names another key".into()));
        }
        Ok(self.key.sign(&operation.base).to_bytes().to_vec())
    }
}

/// The two header fields a signed request carries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureHeaders {
    pub signature_input: String,
    pub signature: String,
    /// The nonce, for linking other evidence to this interaction.
    pub nonce: String,
}

/// A fresh nonce: 256 bits from the operating system's generator.
pub fn fresh_nonce() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Prepares a request for signing under `profile`. Build the request completely first: the
/// signature covers its final URL and, for writes, its `Content-Digest` and
/// `Idempotency-Key` header fields.
pub fn prepare(
    context: RequestContext,
    profile: Profile,
    operation: Operation,
    purpose: Purpose,
    key_id: &str,
    created: i64,
    lifetime: i64,
) -> Result<SignableOperation, SignError> {
    if !(1..=profile.max_lifetime()).contains(&lifetime) {
        return Err(SignError::InvalidRequest(format!(
            "lifetime must be 1 to {} seconds",
            profile.max_lifetime()
        )));
    }
    let components = profile.required_components(operation);
    let params = SignatureParams {
        created,
        expires: created + lifetime,
        keyid: key_id.to_owned(),
        alg: profile.algorithm().to_owned(),
        nonce: fresh_nonce(),
        tag: purpose.tag().to_owned(),
    };
    let (signature_input, signature_params) = fields::signature_input(LABEL, components, &params)
        .map_err(|e| SignError::InvalidRequest(e.detail))?;
    let owned: Vec<String> = components.iter().map(|c| (*c).to_owned()).collect();
    let base = signature_base(&context, &owned, &signature_params)
        .map_err(|e| SignError::InvalidRequest(e.detail))?;
    Ok(SignableOperation {
        context,
        profile,
        operation,
        params,
        signature_input,
        base,
    })
}

/// Signs a prepared operation and returns the header fields to send.
pub fn sign(
    signer: &impl TapSigner,
    operation: &SignableOperation,
) -> Result<SignatureHeaders, SignError> {
    let signature = signer.sign(operation)?;
    let signature =
        fields::signature(LABEL, &signature).map_err(|e| SignError::InvalidRequest(e.detail))?;
    Ok(SignatureHeaders {
        signature_input: operation.signature_input.clone(),
        signature,
        nonce: operation.params.nonce.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::ExternalOrigin;

    fn signer() -> Ed25519Signer {
        Ed25519Signer::new(
            &[9u8; 32],
            "agent-a",
            SigningPolicy {
                destinations: vec!["http://127.0.0.1:3001".into()],
                operations: vec![Operation::Read],
            },
        )
    }

    fn read(origin: &str) -> RequestContext {
        RequestContext::new(
            ExternalOrigin::parse(origin).unwrap(),
            "GET",
            "/demo/v1/products/demo-001",
            vec![],
        )
        .unwrap()
    }

    fn prep(ctx: RequestContext, op: Operation, key: &str) -> Result<SignableOperation, SignError> {
        prepare(
            ctx,
            Profile::ArkavoRfc9421V1,
            op,
            Purpose::Browse,
            key,
            1_000,
            120,
        )
    }

    #[test]
    fn signs_an_allowed_read() {
        let s = signer();
        let op = prep(read("http://127.0.0.1:3001"), Operation::Read, "agent-a").unwrap();
        assert_eq!(op.params().expires, 1_120);
        assert_eq!(op.profile(), Profile::ArkavoRfc9421V1);
        assert_eq!(op.operation(), Operation::Read);
        assert_eq!(op.context().method(), "GET");
        let headers = sign(&s, &op).unwrap();
        assert!(headers.signature_input.starts_with("tap=(\"@authority\""));
        assert!(headers.signature.starts_with("tap=:"));
        assert_eq!(headers.nonce.len(), 43);
        assert!(
            op.signature_base()
                .ends_with(&op.signature_input.as_bytes()[4..])
        );
    }

    #[test]
    fn refuses_other_destinations_operations_and_keys() {
        let s = signer();
        let elsewhere = prep(read("https://evil.example"), Operation::Read, "agent-a").unwrap();
        assert!(matches!(sign(&s, &elsewhere), Err(SignError::Refused(_))));
        let ctx = RequestContext::new(
            ExternalOrigin::parse("http://127.0.0.1:3001").unwrap(),
            "POST",
            "/demo/v1/orders",
            vec![
                ("content-type".into(), "application/json".into()),
                (
                    "content-digest".into(),
                    crate::digest::content_digest(b"{}"),
                ),
                ("idempotency-key".into(), "k".into()),
            ],
        )
        .unwrap();
        let commit = prep(ctx, Operation::Commit, "agent-a").unwrap();
        assert!(matches!(sign(&s, &commit), Err(SignError::Refused(_))));
        let other_key = prep(read("http://127.0.0.1:3001"), Operation::Read, "agent-b").unwrap();
        assert!(matches!(sign(&s, &other_key), Err(SignError::Refused(_))));
    }

    #[test]
    fn refuses_lifetimes_and_missing_coverage() {
        let ctx = read("http://127.0.0.1:3001");
        assert!(prep(ctx.clone(), Operation::Read, "agent-a").is_ok());
        for lifetime in [0, 481] {
            assert!(
                prepare(
                    ctx.clone(),
                    Profile::ArkavoRfc9421V1,
                    Operation::Read,
                    Purpose::Browse,
                    "agent-a",
                    0,
                    lifetime
                )
                .is_err()
            );
        }
        // A write without its Content-Digest cannot be prepared.
        assert!(matches!(
            prep(ctx, Operation::Checkout, "agent-a"),
            Err(SignError::InvalidRequest(_))
        ));
    }

    #[test]
    fn debug_never_shows_the_key() {
        let s = signer();
        let debug = format!("{s:?}");
        assert!(debug.contains("<redacted>"));
        assert!(!debug.contains("[9, 9"));
        assert_eq!(s.key_id(), "agent-a");
        assert_eq!(s.public_key().len(), 32);
        let nonces: std::collections::HashSet<_> = (0..64).map(|_| fresh_nonce()).collect();
        assert_eq!(nonces.len(), 64);
    }
}
