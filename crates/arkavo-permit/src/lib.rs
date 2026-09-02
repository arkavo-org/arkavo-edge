//! CWT permits for permit-bound tool execution.
//!
//! A permit is a CBOR Web Token (RFC 8392) signed as COSE_Sign1 (RFC 8152).
//! Every permit carries a `cnf` confirmation claim (RFC 8747) holding the
//! COSE_Key proof-of-possession key; the signature is verified against that
//! key, so a permit is bound to the key that minted it.
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
pub use permit::{MAX_PERMIT_BYTES, Permit, decode, mint, verify};
pub use pop::{invocation_digest, prove_invocation, verify_invocation_proof};
