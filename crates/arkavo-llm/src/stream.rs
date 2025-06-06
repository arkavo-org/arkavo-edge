use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamResponse {
    pub content: String,
    pub done: bool,
}
