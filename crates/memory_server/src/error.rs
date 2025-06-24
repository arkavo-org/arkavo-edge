use actix_web::{error::ResponseError, http::StatusCode, HttpResponse};

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Storage error: {0}")]
    Storage(String),

    #[error("Embedding error: {0}")]
    Embedding(String),

    #[error("Memory not found")]
    NotFound,

    #[error("Invalid request: {0}")]
    #[allow(dead_code)]
    BadRequest(String),

    #[error("Internal server error: {0}")]
    Internal(#[from] anyhow::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl ResponseError for MemoryError {
    fn error_response(&self) -> HttpResponse {
        let status = self.status_code();
        HttpResponse::build(status).json(serde_json::json!({
            "error": self.to_string(),
            "code": status.as_u16()
        }))
    }

    fn status_code(&self) -> StatusCode {
        match self {
            MemoryError::NotFound => StatusCode::NOT_FOUND,
            MemoryError::BadRequest(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

pub type Result<T> = std::result::Result<T, MemoryError>;