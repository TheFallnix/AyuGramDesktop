use ayu_core::CoreError;
use thiserror::Error;

/// Storage and media cache errors.
#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("storage mutex poisoned")]
    LockPoisoned,

    #[error("payload size {payload_bytes} bytes exceeds cache hard limit {max_bytes} bytes")]
    PayloadExceedsCacheLimit { payload_bytes: usize, max_bytes: usize },

    #[error("corrupt entity span blob length: {0}")]
    CorruptEntityBlob(usize),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<StorageError> for CoreError {
    fn from(err: StorageError) -> Self {
        match err {
            StorageError::PayloadExceedsCacheLimit { .. } => CoreError::Cache(err.to_string()),
            other => CoreError::Storage(other.to_string()),
        }
    }
}
