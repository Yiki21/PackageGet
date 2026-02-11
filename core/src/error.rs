use thiserror::Error;

#[derive(Error, Debug, Clone)]
pub enum CoreError {
    #[error("Failed to execute command: {0}")]
    CommandError(String),

    #[error("Invalid UTF-8 output: {0}")]
    Utf8Error(String),

    #[error("Parse error: {0}")]
    ParseError(String),

    #[error("Unknown error: {0}")]
    UnknownError(String),

    #[error("Serialization error: {0}")]
    SerializationError(String),

    #[error("request error: {0}")]
    RequestError(String),
}

impl From<reqwest::Error> for CoreError {
    fn from(e: reqwest::Error) -> Self {
        CoreError::RequestError(e.to_string())
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::CommandError(e.to_string())
    }
}

impl From<std::string::FromUtf8Error> for CoreError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        CoreError::Utf8Error(e.to_string())
    }
}

impl From<serde_json::Error> for CoreError {
    fn from(e: serde_json::Error) -> Self {
        CoreError::SerializationError(e.to_string())
    }
}
