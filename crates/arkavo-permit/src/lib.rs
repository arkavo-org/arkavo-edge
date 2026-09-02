//! CWT permits for permit-bound tool execution.
//!
//! A permit is a CBOR Web Token (RFC 8392) signed as COSE_Sign1 (RFC 8152).
//! The **issuer** signs it and is named in the protected header by `kid`,
//! the SHA-256 of its public key bytes ([`issuer_kid`]). The `cnf`
//! confirmation claim (RFC 8747) holds a different key: the COSE_Key of the
//! **presenter** who will exercise the permit.
//!
//! [`verify`] therefore takes a list of trusted issuer keys and refuses any
//! permit whose `kid` names none of them. Signing a permit with the key it
//! confirms proves only that the minter holds a keypair, and such a permit is
//! rejected unless that key is itself a trusted issuer.
//!
//! See `docs/permit-cwt-schema.md` for the full claim schema and
//! canonicalization rules.

mod canonical;
mod claims;
mod error;
mod hash;
mod keys;
mod permit;
mod pop;

// `PermitVerifier` wraps this type in its public field, so callers can name it
// without taking their own dependency on `arkavo-cwt`.
pub use arkavo_cwt::VerifyingKey;
pub use arkavo_cwt::sign1::{CWT_TAG_PREFIX, MAX_TOKEN_BYTES};
pub use canonical::{argument_hash, canonical_json, canonicalize_json_text};
pub use claims::{
    BUDGET_COST_MICRO_USD, BUDGET_MAX_INVOCATIONS, BUDGET_TOKEN_CEILING, Budget,
    CLAIM_AGENT_WORKLOAD_ID, CLAIM_ARGUMENT_HASH, CLAIM_BUDGET, CLAIM_CONFIRMATION,
    CLAIM_DATA_CLASSIFICATIONS, CLAIM_EXPIRATION, CLAIM_ISSUED_AT, CLAIM_ISSUER, CLAIM_NOT_BEFORE,
    CLAIM_PARENT_PERMIT, CLAIM_POLICY_BUNDLE_HASH, CLAIM_SEQUENCE_STATE_HASH, CLAIM_SUBJECT,
    CLAIM_TOOL_NAME, CNF_COSE_KEY, PermitClaims,
};
pub use error::PermitError;
pub use hash::HashAlgorithm;
pub use keys::{PermitSigner, PermitVerifier};
pub use permit::{MAX_PERMIT_BYTES, Permit, decode, issuer_kid, mint, verify};
pub use pop::{invocation_digest, prove_invocation, verify_invocation_proof};
