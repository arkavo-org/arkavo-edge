pub mod error;
pub mod event;
pub mod payload;
pub mod writer;

pub use error::EventError;
pub use event::{Event, EventMetadata};
pub use payload::EventPayload;
pub use payload::{SequenceBaseline, TaintRecord};
pub use writer::{EventWriter, EventWriterConfig};

pub const SCHEMA_VERSION: &str = "1.0.0";
