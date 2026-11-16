use thiserror::Error;

/// Domain, state orchestration, and repository contract errors.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("account {0} is not registered")]
    AccountNotFound(u64),

    #[error("account {0} is already registered")]
    AccountAlreadyExists(u64),

    #[error("invalid viewport window capacity: {0}")]
    InvalidWindowCapacity(usize),

    #[error("storage backend error: {0}")]
    Storage(String),

    #[error("cache backend error: {0}")]
    Cache(String),

    #[error("transport error: {0}")]
    Transport(String),

    #[error("invalid filter pattern: {0}")]
    InvalidFilterPattern(String),

    #[error("corrupt authorization blob: {0}")]
    CorruptAuthBlob(&'static str),

    #[error("invalid auth state transition from {from} to {to}")]
    InvalidAuthStateTransition { from: u8, to: u8 },

    #[error("maximum accounts limit reached: {max}")]
    MaxAccountsReached { max: usize },
}
