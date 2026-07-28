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

impl CoreError {
    pub fn from_command_failure(detail: String) -> Self {
        if let Some(message) = classify_command_failure(&detail) {
            return CoreError::CommandError(message);
        }

        CoreError::CommandError(detail)
    }
}

pub fn classify_command_failure(detail: &str) -> Option<String> {
    let lower = detail.to_ascii_lowercase();

    if lower.contains("operation was cancelled")
        || lower.contains("authentication failure")
        || lower.contains("not authorized")
        || lower.contains("authorization failed")
        || lower.contains("user of caller and user of subject differs")
        || lower.contains("cancelled")
        || lower.contains("canceled")
    {
        return Some(
            "authorization was cancelled or denied. Please retry and approve the pkexec prompt"
                .to_owned(),
        );
    }

    if lower.contains("no authentication agent found")
        || lower.contains("polkit")
        || lower.contains("pkexec must be setuid root")
    {
        return Some(
            "no polkit authentication agent is available. Start a desktop polkit agent or install polkit/pkexec"
                .to_owned(),
        );
    }

    if lower.contains("could not get lock")
        || lower.contains("unable to acquire the dpkg frontend lock")
        || lower.contains("unable to lock the administration directory")
        || lower.contains("could not acquire lock")
        || lower.contains("failed to init transaction")
        || lower.contains("unable to lock database")
        || lower.contains("database is locked")
        || lower.contains("another app is currently holding the yum lock")
        || lower.contains("another app is currently holding the dnf lock")
        || lower.contains("system management is locked")
    {
        return Some(
            "package database is locked by another package operation. Close other installers or wait for the current operation to finish"
                .to_owned(),
        );
    }

    None
}

impl From<reqwest::Error> for CoreError {
    fn from(e: reqwest::Error) -> Self {
        CoreError::RequestError(e.to_string())
    }
}

impl From<std::io::Error> for CoreError {
    fn from(e: std::io::Error) -> Self {
        CoreError::from_command_failure(e.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_pkexec_cancelled() {
        let message =
            classify_command_failure("Error executing command as another user: Not authorized");

        assert_eq!(
            message.as_deref(),
            Some(
                "authorization was cancelled or denied. Please retry and approve the pkexec prompt"
            )
        );
    }

    #[test]
    fn classifies_missing_polkit_agent() {
        let message = classify_command_failure("No authentication agent found");

        assert_eq!(
            message.as_deref(),
            Some(
                "no polkit authentication agent is available. Start a desktop polkit agent or install polkit/pkexec"
            )
        );
    }

    #[test]
    fn classifies_package_database_lock() {
        let message = classify_command_failure(
            "E: Could not get lock /var/lib/dpkg/lock-frontend. It is held by process 123",
        );

        assert_eq!(
            message.as_deref(),
            Some(
                "package database is locked by another package operation. Close other installers or wait for the current operation to finish"
            )
        );
    }
}
