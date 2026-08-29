//! OIDC session for `identity.arkavo.net`.
//!
//! The CLI is public client `arkavo-edge`. Arkavo Creator runs the passkey
//! ceremony; this crate never sees Creator's session CWT.

mod discovery;
mod error;
mod pkce;
mod store;

pub use discovery::{
    DEFAULT_IDENTITY_HOST, DEFAULT_PLATFORM_URL, IdentityEndpoints, discover, host_of,
};
pub use error::{IdentityError, Prompt};
pub use pkce::Pkce;
pub use store::{StoredTokens, delete, load, save, token_path};
