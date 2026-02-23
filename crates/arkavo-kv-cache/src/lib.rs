//! Composable KV cache context slots for multi-sequence inference
//!
//! Named regions of pre-encoded KV cache that can be loaded, composed,
//! and swapped independently. Agent identity, user preferences, policy
//! context, domain knowledge — all the same operation.

pub mod context_manager;
pub mod manifest;

pub use context_manager::{ContextManager, ContextSlot};
pub use manifest::{ContextEntry, ContextManifest};
