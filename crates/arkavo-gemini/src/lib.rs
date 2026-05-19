pub mod dispatcher;
pub mod error;
pub mod live_client;
pub mod rest_client;
pub mod sse_stream;
pub mod types;

pub use dispatcher::{ToolDefinition, ToolDispatcher, ToolRegistry};
pub use error::{GeminiError, Result};
pub use live_client::{DEFAULT_LIVE_MODEL, GEMINI_WS_ENDPOINT, LiveModality, LiveSessionClient};
pub use rest_client::{RestClient, ThinkingConfig};
pub use sse_stream::GeminiSseStream;
pub use types::{
    ClientContent, FunctionCall, FunctionDeclaration, GenerationConfig, ServerMessage, SetupConfig,
    StreamResponse, Tool, ToolResponse, UsageMetadata,
};
