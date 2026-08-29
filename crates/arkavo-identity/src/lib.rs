//! OIDC session for `identity.arkavo.net`.
//!
//! The CLI is public client `arkavo-edge`. Arkavo Creator runs the passkey
//! ceremony; this crate never sees Creator's session CWT.

mod broker;
mod discovery;
mod error;
mod loopback;
mod pkce;
mod store;
mod token;

pub use broker::{CLIENT_ID, CREATOR_BUNDLE_ID, SCOPE, authorize_url, creator_url, launch_creator};
pub use discovery::{
    DEFAULT_IDENTITY_HOST, DEFAULT_PLATFORM_URL, IdentityEndpoints, discover, host_of,
};
pub use error::{IdentityError, Prompt};
pub use loopback::{
    BoundCallback, CALLBACK_DEADLINE, Callback, LOOPBACK_PORTS, bind, wait_for_callback,
};
pub use pkce::Pkce;
pub use store::{StoredTokens, delete, load, save, token_path};
pub use token::{exchange_code, refresh};
