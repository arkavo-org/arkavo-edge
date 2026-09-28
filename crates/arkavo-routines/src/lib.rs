//! Learn bounded tool sequences from execution evidence, then replay through
//! the caller's current authorization boundary. Stored templates carry no
//! free-text instructions or string argument values.

pub mod evaluation;
mod library;
mod model;
mod session;
pub mod template;
mod tools;

pub use library::{Library, Record};
pub use model::{Check, Executor, Routine, Step};
pub use session::{Metrics, Session};
pub use tools::register_tools;

pub type Result<T> = std::result::Result<T, String>;
