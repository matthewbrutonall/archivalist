use thiserror::Error;

#[derive(Error, Debug)]
pub enum ASpaceError {
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("API error (HTTP {status}): {message}")]
    Api { status: u16, message: String },
    #[error("JSON serialization error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Not authenticated. Please call login() first.")]
    NotAuthenticated,
    #[error("Solr error (HTTP {status}): {message}")]
    Solr { status: u16, message: String },
    #[error("Batch size exceeded: requested {requested}, maximum allowed {max}")]
    BatchSizeExceeded { requested: usize, max: usize },
    #[error("Invalid request: {0}")]
    InvalidRequest(String),
    #[error("Hydration validation failed: {0}")]
    HydrationValidation(String),
}

pub type Result<T> = std::result::Result<T, ASpaceError>;
