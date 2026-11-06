//! Trusted Agent Protocol (TAP) for Arkavo: an agent identifies itself to a merchant while
//! acting for a person, and the merchant tells the acting agent, its provider, the
//! represented person and the authority granted for a purchase apart.
//!
//! - [`signer`] signs outbound requests under a profile, through a signer that enforces its
//!   own destination policy.
//! - [`verifier`] checks a received request: strict RFC 9421 parsing, the profile's coverage,
//!   time bounds, a trusted key, the signature, the content digest, then a durable nonce
//!   claim. Success is a [`VerifiedAgent`], which proves who asked and nothing more.
//! - [`authorize`] decides, from separate evidence, whether that agent may commit a purchase.
//!
//! A successful identity check never, by itself, authorizes a charge. The HTTP framework,
//! the payment provider and the model runtime are all outside this crate.

pub mod authorize;
pub mod base;
pub mod clock;
pub mod context;
pub mod digest;
pub mod envelope;
pub mod error;
pub mod evidence;
pub mod fields;
pub mod keys;
pub mod profile;
pub mod registry;
pub mod replay;
pub mod signer;
pub mod verifier;

pub use authorize::{
    AuthorizationDecision, CheckoutLine, CheckoutView, GrantLedger, GrantState, LOCAL_TEST_GRANT,
    LocalGrantAuthorizer, LocalTestGrant, PurchaseAuthorizer,
};
pub use clock::{Clock, ManualClock, SystemClock};
pub use context::{ExternalOrigin, RequestContext};
pub use envelope::SignedEnvelope;
pub use error::{RejectKind, Rejection, SignError};
pub use evidence::{DecisionRecord, RequestBinding, VerifiedAgent};
pub use keys::{
    ActorBinding, Environment, KeyFields, KeyRecord, KeyStatus, ResolveError, TrustAuthority,
    TrustedKeyResolver, thumbprint,
};
pub use profile::{Operation, Profile, Purpose};
pub use registry::{KeyDocument, RegistryConfig, RegistryResolver};
pub use replay::{ClaimOutcome, ReplayScope, ReplayStore, SqliteReplayStore};
pub use signer::{Ed25519Signer, SignableOperation, SignatureHeaders, SigningPolicy, TapSigner};
pub use verifier::{RoutePolicy, TapVerifier, VerifierConfig};
