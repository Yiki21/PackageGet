use thiserror::Error;

/// Failures produced by core configuration storage.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CoreError {
    /// Configuration validation or persistence failed.
    #[error("Configuration error: {0}")]
    ConfigError(String),
}
