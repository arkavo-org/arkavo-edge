//! An agent's part in a pipeline run.
//!
//! Each role of a kit runs as its own agent. When the kit's topology is
//! `pipeline`, the roles answer one request in turn, each handed the request
//! and what the roles before it produced. An agent takes part by answering
//! the step it is sent: it receives a message, answers it, and is polled.

mod marker;
mod step;

pub use marker::{MARKER_KEY, is_marked, step_timeout};
pub(super) use step::answer_step;
