/// Errors at the core boundary. Never includes token material.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("keychain error: {0}")]
    Keychain(String),

    #[error("no Claude Code credentials in the login keychain (run `claude` and log in first)")]
    NoCredentials,

    #[error("credential blob is not in the expected format: {0}")]
    BadBlob(String),

    #[error("HTTP {status} from the {endpoint} endpoint")]
    Http { endpoint: &'static str, status: u16 },

    #[error("network error reaching the {endpoint} endpoint: {detail}")]
    Network {
        endpoint: &'static str,
        detail: String,
    },

    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error("account name {0:?} is invalid (use 1-64 chars: letters, digits, '.', '_', '-')")]
    BadAccountName(String),

    #[error("account '{0}' already exists (remove it first to re-capture)")]
    AccountExists(String),

    #[error("account '{0}' not found")]
    AccountNotFound(String),

    #[error("HOME is not set")]
    NoHome,
}
