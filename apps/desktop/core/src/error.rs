//! One error for the whole core. Its text is what the person reads, so it says what to do next
//! where there is something to do.

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error("Not connected to a team-hub server. Add the server address and an API token first.")]
    NotConnected,
    #[error("The server cannot be reached: {0}")]
    Offline(String),
    #[error("{message}")]
    Api { status: u16, code: String, message: String },
    #[error("git {command}: {reason}")]
    Git { command: String, reason: String },
    #[error("Keychain: {0}")]
    Keychain(String),
    #[error("Local database: {0}")]
    Store(#[from] rusqlite::Error),
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
    /// The push or pull request touches something a person has to confirm by name.
    #[error("This needs an explicit confirmation: {0}")]
    NeedsConfirmation(String),
    #[error("{0}")]
    NotPossible(String),
}

pub type CoreResult<T> = Result<T, CoreError>;

impl CoreError {
    pub fn not_possible(reason: impl Into<String>) -> Self {
        Self::NotPossible(reason.into())
    }

    pub fn git(command: &str, reason: impl Into<String>) -> Self {
        Self::Git { command: command.to_string(), reason: reason.into() }
    }

    /// The server answered that the thing is not there: a missing issue, or an API it does not have.
    pub fn is_not_found(&self) -> bool {
        matches!(self, Self::Api { status: 404 | 405, .. })
    }

    pub fn is_offline(&self) -> bool {
        matches!(self, Self::Offline(_))
    }

    pub fn is_unauthorized(&self) -> bool {
        matches!(self, Self::Api { status: 401, .. })
    }
}

impl From<tokio::task::JoinError> for CoreError {
    fn from(error: tokio::task::JoinError) -> Self {
        Self::NotPossible(format!("a background task failed: {error}"))
    }
}
