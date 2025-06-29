pub mod chat;
pub mod code;
pub mod dataflow;
pub mod debug;
pub mod diff;
pub mod task_window;
pub mod widgets;

pub use chat::ChatView;
pub use code::CodeView;
pub use dataflow::DataflowView;
pub use debug::DebugView;
pub use diff::DiffView;
pub use task_window::{TaskManager, TaskWindow};
