pub mod client;
pub mod error;
pub mod image;
pub mod message;
pub mod ollama;
pub mod provider;
pub mod stream;

pub use client::LlmClient;
pub use error::{Error, Result};
pub use image::{encode_image_file, encode_image_bytes, decode_image, ImageFormat};
pub use message::{Message, Role};
pub use provider::Provider;
pub use stream::StreamResponse;
