pub mod llm_integration;
pub mod types;

#[cfg(any(feature = "cef-ui", feature = "web-ui"))]
pub mod adapters;

pub use llm_integration::LlmIntegration;
pub use types::{UIContent, UIEvent, UserInterface};
