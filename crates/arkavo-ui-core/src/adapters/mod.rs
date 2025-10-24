#[cfg(feature = "cef-ui")]
pub mod cef;

#[cfg(feature = "web-ui")]
pub mod web;

#[cfg(feature = "cef-ui")]
pub use cef::CEFAdapter;

#[cfg(feature = "web-ui")]
pub use web::WebAdapter;
