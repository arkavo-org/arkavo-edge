//! Process-wide security policy, installed before any routers are constructed.
//!
//! The CLI owns the classifier (above this crate in the dependency graph).
//! An immutable registration covers routers created by both the server and
//! embedded engines without letting a request replace or disable the policy.

use std::sync::{Arc, OnceLock};

use arkavo_llm::{GuardedProvider, Provider, ReleaseGateFactory};

static POLICY: OnceLock<Arc<dyn ReleaseGateFactory>> = OnceLock::new();

pub fn install(gates: Arc<dyn ReleaseGateFactory>) -> Result<(), &'static str> {
    POLICY
        .set(gates)
        .map_err(|_| "response policy already installed")
}

pub fn protect(provider: Box<dyn Provider>) -> Box<dyn Provider> {
    match POLICY.get() {
        Some(policy) => Box::new(GuardedProvider::new(provider, policy.clone())),
        None => provider,
    }
}
