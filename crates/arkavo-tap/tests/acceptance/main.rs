//! Acceptance tests for the core library, named by the scenario IDs in
//! `docs/tap/specification.md` (T01 to T30) where one applies.
#![allow(clippy::disallowed_methods)] // Tokio test entrypoints own their runtime.
#![allow(unreachable_pub)] // The shared harness is `pub` to the test modules beside it.

mod authorize;
mod common;
mod interop;
mod registry;
mod verifier;
