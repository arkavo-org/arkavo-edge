//! The local Trusted Agent Protocol harness: a development key registry, a merchant fixture
//! with simulated payment, a buyer that signs its requests, and the acceptance suite.
//!
//! Development only. The registry is a local trust authority, the grant is a
//! `local-test-grant`, and payment is simulated: nothing here is a Visa registration, an AP2
//! mandate or a payment.

pub mod buyer;
pub mod config;
pub mod init;
pub mod keyfile;
pub mod merchant;
pub mod registry;
pub mod server;
pub mod suite;
