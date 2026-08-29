//! OIDC session for `identity.arkavo.net`.
//!
//! The CLI is public client `arkavo-edge`. Arkavo Creator runs the passkey
//! ceremony; this crate never sees Creator's session CWT.

mod error;

pub use error::{IdentityError, Prompt};
