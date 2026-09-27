//! Canonical AG-UI protocol types.
//!
//! Mirrors the Agent-User Interaction Protocol specification so that Arkavo
//! Edge can emit and consume standard agent-to-frontend event streams.

pub mod event;
pub mod input;
pub mod message;
pub mod patch;

#[cfg(test)]
mod tests;

pub use event::{BaseEvent, EventPayload};
pub use input::*;
pub use message::*;
pub use patch::JsonPatch;
