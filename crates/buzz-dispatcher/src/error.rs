//! Error type for the triage dispatcher.

/// Errors produced by the dispatcher.
#[derive(Debug, thiserror::Error)]
pub enum DispatchError {
    /// Configuration file or validation problem.
    #[error("config: {0}")]
    Config(String),
    /// Service identity problem (BUZZ_PRIVATE_KEY / BUZZ_AUTH_TAG).
    #[error("identity: {0}")]
    Identity(String),
    /// SDK builder/parser rejected an event.
    #[error("sdk: {0}")]
    Sdk(#[from] buzz_sdk::SdkError),
    /// Relay rejected or failed a request.
    #[error("relay: {0}")]
    Relay(String),
    /// HTTP transport failure.
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    /// Local state I/O failure.
    #[error("state io: {0}")]
    Io(#[from] std::io::Error),
    /// JSON serialization problem.
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    /// Anything else.
    #[error("{0}")]
    Other(String),
}
