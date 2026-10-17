//! This host's generator and whether it serves generation now.
//!
//! A kit role decides whether an agent generates (ASSET-010), and a role
//! assigned while the agent runs can change that (SKD-011). The tool, the
//! asset service and the capability report all read it here at the moment
//! they act, so turning serving off takes effect on the next request.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::generator::Generator;

pub struct HostGenerator {
    generator: Option<Arc<Generator>>,
    serving: AtomicBool,
}

impl HostGenerator {
    /// `generator` is `None` when the host does not qualify (IMG-001).
    pub fn new(generator: Option<Arc<Generator>>, serving: bool) -> Self {
        Self {
            generator,
            serving: AtomicBool::new(serving),
        }
    }

    /// The generator, when the host has one and serves now.
    pub fn serving(&self) -> Option<Arc<Generator>> {
        self.generator
            .clone()
            .filter(|_| self.serving.load(Ordering::SeqCst))
    }

    /// The generator whether or not it serves: what this host could do.
    pub fn qualified(&self) -> Option<&Arc<Generator>> {
        self.generator.as_ref()
    }

    pub fn set_serving(&self, serving: bool) {
        self.serving.store(serving, Ordering::SeqCst);
    }
}
