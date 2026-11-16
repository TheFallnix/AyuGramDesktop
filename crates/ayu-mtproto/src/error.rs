use ayu_core::CoreError;
use thiserror::Error;

/// MTProto networking, session multiplexing, and protocol framing errors.
#[derive(Debug, Error)]
pub enum MtprotoError {
    #[error("account {0} has no active transport session")]
    SessionNotRegistered(u64),

    #[error("transport disconnected for dc {dc_id}: {reason}")]
    Disconnected { dc_id: u16, reason: String },

    #[error("connection timeout after {timeout_ms} ms")]
    Timeout { timeout_ms: u64 },

    #[error("invalid auth key length: expected 256 bytes, got {0}")]
    InvalidAuthKeyLength(usize),

    #[error("protocol framing error: {0}")]
    Framing(String),

    #[error("multiplexer mutex poisoned")]
    LockPoisoned,
}

impl From<MtprotoError> for CoreError {
    fn from(err: MtprotoError) -> Self {
        CoreError::Transport(err.to_string())
    }
}
