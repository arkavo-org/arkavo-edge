pub mod chat;
pub mod client;
pub mod error;
pub mod image;
#[cfg(feature = "llm-local")]
pub mod local;
pub mod message;
#[cfg(feature = "llm-remote")]
pub mod ollama;
pub mod provider;
pub mod stream;
pub mod stream_adapter;
pub mod stream_model;

pub use chat::ChatRequest;
pub use client::LlmClient;
pub use error::{Error, Result};
pub use image::{ImageFormat, decode_image, encode_image_bytes, encode_image_file};
pub use message::{Message, Role};
pub use provider::Provider;
pub use stream::StreamResponse;
pub use stream_adapter::LlmClientAdapter;
pub use stream_model::{
    DeltaStream, DeltaType, EndReason, StreamControl, StreamDelta, StreamError, StreamId,
    StreamLlmModel, create_bounded_stream, text_to_delta_stream,
};
