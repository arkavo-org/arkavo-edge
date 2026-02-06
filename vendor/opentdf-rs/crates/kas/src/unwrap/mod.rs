//! Key unwrap operations for KAS server
//!
//! Provides EC (NanoTDF) and RSA (Standard TDF) unwrap functionality.

pub mod ec_unwrap;

#[cfg(feature = "rsa")]
pub mod rsa_unwrap;

pub use ec_unwrap::{custom_ecdh, ec_unwrap};

#[cfg(feature = "rsa")]
pub use rsa_unwrap::rsa_unwrap;
