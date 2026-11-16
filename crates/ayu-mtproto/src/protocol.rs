use std::fmt;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;

use crate::error::MtprotoError;

/// 8-byte MTProto 2.0 authorization key fingerprint.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct AuthKeyId(pub [u8; 8]);

/// 256-byte MTProto 2.0 permanent authorization key.
#[derive(Clone)]
pub struct AuthKey {
    bytes: Box<[u8; 256]>,
    key_id: AuthKeyId,
}

impl fmt::Debug for AuthKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthKey")
            .field("key_id", &self.key_id)
            .finish_non_exhaustive()
    }
}

impl AuthKey {
    pub fn from_slice(slice: &[u8]) -> Result<Self, MtprotoError> {
        if slice.len() != 256 {
            return Err(MtprotoError::InvalidAuthKeyLength(slice.len()));
        }
        let mut arr = [0u8; 256];
        arr.copy_from_slice(slice);
        let mut id = [0u8; 8];
        // Placeholder fingerprint for Stage 1 domain/multiplexer wiring; SHA-1 key_id lands in Stage 4.
        id.copy_from_slice(&arr[248..256]);
        Ok(Self {
            bytes: Box::new(arr),
            key_id: AuthKeyId(id),
        })
    }

    #[must_use]
    pub const fn id(&self) -> AuthKeyId {
        self.key_id
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8; 256] {
        &self.bytes
    }
}

/// 64-bit MTProto session identifier.
#[repr(transparent)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub struct SessionId(pub u64);

/// Monotonic client message ID generator (divisible by 4 per MTProto 2.0 spec).
#[derive(Debug, Default)]
pub struct MsgIdGenerator {
    last_id: AtomicI64,
}

impl MsgIdGenerator {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            last_id: AtomicI64::new(0),
        }
    }

    /// Generates a strictly increasing 64-bit client `msg_id` with `msg_id % 4 == 0`.
    pub fn next_msg_id(&self) -> i64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        let secs = now.as_secs() as i64;
        let nanos_fraction = ((now.subsec_nanos() as u64) << 32) / 1_000_000_000;
        let candidate = ((secs << 32) | (nanos_fraction as i64)) & !3;

        let mut prev = self.last_id.load(Ordering::Relaxed);
        loop {
            let next = if candidate > prev {
                candidate
            } else {
                prev.saturating_add(4)
            };
            match self.last_id.compare_exchange_weak(
                prev,
                next,
                Ordering::SeqCst,
                Ordering::Relaxed,
            ) {
                Ok(_) => return next,
                Err(actual) => prev = actual,
            }
        }
    }
}

/// High-level TL method classifier used by the Ghost Mode packet interceptor.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub enum TlMethodTag {
    ReadHistory,
    ReadChannelHistory,
    ReadStories,
    UpdateStatusOnline,
    UpdateStatusOffline,
    SetTyping,
    SendMessage,
    Ping,
    Raw(u32),
}

/// Framed MTProto 2.0 outbound packet envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptedEnvelope {
    pub dc_id: u16,
    pub auth_key_id: AuthKeyId,
    pub msg_key: [u8; 16],
    pub session_id: SessionId,
    pub msg_id: i64,
    pub seq_no: i32,
    pub method: TlMethodTag,
    pub payload: Bytes,
}

/// Exponential backoff with bounded jitter for resilient DC reconnects.
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ReconnectPolicy {
    pub base_delay_ms: u64,
    pub max_delay_ms: u64,
    pub max_attempts: u32,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            base_delay_ms: 250,
            max_delay_ms: 30_000,
            max_attempts: 12,
        }
    }
}

impl ReconnectPolicy {
    #[must_use]
    pub fn delay_for_attempt(&self, attempt: u32, jitter_seed: u64) -> Duration {
        let shift = attempt.min(16);
        let raw = self.base_delay_ms.saturating_mul(1u64 << shift);
        let capped = raw.min(self.max_delay_ms);
        let jitter_cap = (capped / 5).max(1);
        let jitter = jitter_seed % jitter_cap;
        Duration::from_millis(capped.saturating_add(jitter).min(self.max_delay_ms))
    }
}
