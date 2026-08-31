//! Keyed reference index: the DLP cascade's fast tier (KP-009, KP-010, KP-011).
//!
//! The question this crate answers is "have I seen this content before, and
//! what was it labelled" — fast enough to sit on the per-call path, and without
//! storing anything an attacker who steals the index can read.
//!
//! Every digest is keyed with a tenant key. An unkeyed index of sensitive
//! content is a dictionary: hash a guess, look for the digest, learn whether
//! the guess was in the corpus. Keying is what makes a stolen index inert, so
//! there is no path through this crate that produces an unkeyed hash.

pub mod index;
pub mod key;
pub mod shingle;
pub mod tier;

pub use index::{
    EntryMeta, INDEX_FORMAT_VERSION, IndexError, MatchSummary, ReferenceIndex,
    ReferenceIndexBuilder, SuppressionIndex, match_span,
};
pub use key::{IndexKey, KeyError, MIN_SECRET_BYTES, ShingleHash};
pub use shingle::{SHINGLE_WORDS, normalize, shingle_text, shingles, windows};
pub use tier::{ReferenceTier, TIER_BUDGET, TIER_NAME, evidence_for};
