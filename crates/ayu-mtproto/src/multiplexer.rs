use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use ayu_core::{AccountId, GhostModeConfig, NetworkTransport};
use bytes::{BufMut, BytesMut};

use crate::error::MtprotoError;
use crate::interceptor::{GhostPacketInterceptor, InterceptDecision};
use crate::protocol::{
    AuthKey, EncryptedEnvelope, MsgIdGenerator, ReconnectPolicy, SessionId, TlMethodTag,
};

pub type TransportFuture<'a> =
    Pin<Box<dyn Future<Output = Result<usize, MtprotoError>> + Send + 'a>>;

/// Asynchronous transport abstraction for sending serialized MTProto envelopes over TCP/TLS/WS.
pub trait AsyncTransport: Send + Sync {
    fn send_frame<'a>(
        &'a self,
        account_id: AccountId,
        dc_id: u16,
        frame: &'a [u8],
    ) -> TransportFuture<'a>;

    fn is_connected(&self, account_id: AccountId) -> bool;
}

/// Shared reusable scratch buffer pool preventing per-account buffer allocation bloat.
pub struct BufferPool {
    pool: Mutex<VecDeque<BytesMut>>,
    max_pooled: usize,
    default_capacity: usize,
}

impl BufferPool {
    #[must_use]
    pub fn new(max_pooled: usize, default_capacity: usize) -> Self {
        Self {
            pool: Mutex::new(VecDeque::with_capacity(max_pooled)),
            max_pooled,
            default_capacity,
        }
    }

    pub fn checkout(&self) -> BytesMut {
        if let Ok(mut guard) = self.pool.lock() {
            if let Some(mut buf) = guard.pop_front() {
                buf.clear();
                return buf;
            }
        }
        BytesMut::with_capacity(self.default_capacity)
    }

    pub fn checkin(&self, mut buf: BytesMut) {
        if buf.capacity() > self.default_capacity.saturating_mul(8) {
            return;
        }
        buf.clear();
        if let Ok(mut guard) = self.pool.lock() {
            if guard.len() < self.max_pooled {
                guard.push_back(buf);
            }
        }
    }

    #[must_use]
    pub fn pooled_count(&self) -> usize {
        self.pool.lock().map(|g| g.len()).unwrap_or(0)
    }
}

#[derive(Debug)]
struct AccountSessionSlot {
    dc_id: u16,
    session_id: SessionId,
    ghost_config: GhostModeConfig,
    reconnect_attempts: u32,
    queued_outbound: usize,
    suppressed_packets: u64,
    sent_packets: u64,
    dc_keys: HashMap<i32, AuthKey>,
    keys_to_destroy: Vec<(i32, AuthKey)>,
}

/// Outcome of dispatching an outbound envelope through [`MtprotoMultiplexer`].
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum DispatchOutcome {
    Sent { bytes_written: usize, scheduled_offline: bool },
    SuppressedByGhostMode,
}

/// Multiplexes multiple Telegram accounts over one shared Tokio runtime and buffer pool.
pub struct MtprotoMultiplexer<T: AsyncTransport> {
    transport: Arc<T>,
    buffer_pool: Arc<BufferPool>,
    msg_ids: MsgIdGenerator,
    reconnect_policy: ReconnectPolicy,
    sessions: Mutex<HashMap<AccountId, AccountSessionSlot>>,
}

impl<T: AsyncTransport> MtprotoMultiplexer<T> {
    #[must_use]
    pub fn new(transport: Arc<T>, max_pooled_buffers: usize) -> Self {
        Self {
            transport,
            buffer_pool: Arc::new(BufferPool::new(max_pooled_buffers, 4096)),
            msg_ids: MsgIdGenerator::new(),
            reconnect_policy: ReconnectPolicy::default(),
            sessions: Mutex::new(HashMap::new()),
        }
    }

    #[must_use]
    pub fn buffer_pool(&self) -> &Arc<BufferPool> {
        &self.buffer_pool
    }

    pub fn register_session(
        &self,
        account_id: AccountId,
        dc_id: u16,
        session_id: SessionId,
        ghost_config: GhostModeConfig,
    ) -> Result<(), MtprotoError> {
        let mut guard = self
            .sessions
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        guard.insert(
            account_id,
            AccountSessionSlot {
                dc_id,
                session_id,
                ghost_config,
                reconnect_attempts: 0,
                queued_outbound: 0,
                suppressed_packets: 0,
                sent_packets: 0,
                dc_keys: HashMap::new(),
                keys_to_destroy: Vec::new(),
            },
        );
        Ok(())
    }

    pub fn unregister_session(&self, account_id: AccountId) -> Result<(), MtprotoError> {
        let mut guard = self
            .sessions
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        guard.remove(&account_id);
        Ok(())
    }

    pub fn update_ghost_config(
        &self,
        account_id: AccountId,
        config: GhostModeConfig,
    ) -> Result<(), MtprotoError> {
        let mut guard = self
            .sessions
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let slot = guard
            .get_mut(&account_id)
            .ok_or(MtprotoError::SessionNotRegistered(account_id.0))?;
        slot.ghost_config = config;
        Ok(())
    }

    #[must_use]
    pub fn next_msg_id(&self) -> i64 {
        self.msg_ids.next_msg_id()
    }

    #[must_use]
    pub const fn reconnect_policy(&self) -> ReconnectPolicy {
        self.reconnect_policy
    }

    pub fn suppressed_count(&self, account_id: AccountId) -> Result<u64, MtprotoError> {
        let guard = self
            .sessions
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        guard
            .get(&account_id)
            .map(|s| s.suppressed_packets)
            .ok_or(MtprotoError::SessionNotRegistered(account_id.0))
    }

    pub fn sent_count(&self, account_id: AccountId) -> Result<u64, MtprotoError> {
        let guard = self
            .sessions
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        guard
            .get(&account_id)
            .map(|s| s.sent_packets)
            .ok_or(MtprotoError::SessionNotRegistered(account_id.0))
    }

    pub fn session_id(&self, account_id: AccountId) -> Result<SessionId, MtprotoError> {
        let guard = self
            .sessions
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        guard
            .get(&account_id)
            .map(|s| s.session_id)
            .ok_or(MtprotoError::SessionNotRegistered(account_id.0))
    }

    pub fn bind_dc_key(
        &self,
        account_id: AccountId,
        dc_id: i32,
        key: AuthKey,
    ) -> Result<(), MtprotoError> {
        let mut guard = self
            .sessions
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let slot = guard
            .get_mut(&account_id)
            .ok_or(MtprotoError::SessionNotRegistered(account_id.0))?;
        slot.dc_keys.insert(dc_id, key);
        Ok(())
    }

    pub fn set_main_dc(&self, account_id: AccountId, dc_id: u16) -> Result<(), MtprotoError> {
        let mut guard = self
            .sessions
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let slot = guard
            .get_mut(&account_id)
            .ok_or(MtprotoError::SessionNotRegistered(account_id.0))?;
        slot.dc_id = dc_id;
        Ok(())
    }

    pub fn schedule_keys_destroy(
        &self,
        account_id: AccountId,
        keys: Vec<(i32, AuthKey)>,
    ) -> Result<usize, MtprotoError> {
        let mut guard = self
            .sessions
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let slot = guard
            .get_mut(&account_id)
            .ok_or(MtprotoError::SessionNotRegistered(account_id.0))?;
        for (dc_id, key) in keys {
            slot.dc_keys.remove(&dc_id);
            slot.keys_to_destroy.push((dc_id, key));
        }
        Ok(slot.keys_to_destroy.len())
    }

    pub fn drain_keys_to_destroy(
        &self,
        account_id: AccountId,
    ) -> Result<Vec<(i32, AuthKey)>, MtprotoError> {
        let mut guard = self
            .sessions
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let slot = guard
            .get_mut(&account_id)
            .ok_or(MtprotoError::SessionNotRegistered(account_id.0))?;
        Ok(std::mem::take(&mut slot.keys_to_destroy))
    }

    pub async fn dispatch_envelope(
        &self,
        account_id: AccountId,
        mut envelope: EncryptedEnvelope,
    ) -> Result<DispatchOutcome, MtprotoError> {
        let (dc_id, session_id, ghost_config) = {
            let mut guard = self
                .sessions
                .lock()
                .map_err(|_| MtprotoError::LockPoisoned)?;
            let slot = guard
                .get_mut(&account_id)
                .ok_or(MtprotoError::SessionNotRegistered(account_id.0))?;
            slot.queued_outbound = slot.queued_outbound.saturating_add(1);
            (slot.dc_id, slot.session_id, slot.ghost_config)
        };
        if envelope.session_id.0 == 0 {
            envelope.session_id = session_id;
        }

        let decision = GhostPacketInterceptor::evaluate(ghost_config, &envelope);
        let scheduled_offline = match decision {
            InterceptDecision::DropSuppressed => {
                let mut guard = self
                    .sessions
                    .lock()
                    .map_err(|_| MtprotoError::LockPoisoned)?;
                if let Some(slot) = guard.get_mut(&account_id) {
                    slot.queued_outbound = slot.queued_outbound.saturating_sub(1);
                    slot.suppressed_packets = slot.suppressed_packets.saturating_add(1);
                }
                return Ok(DispatchOutcome::SuppressedByGhostMode);
            }
            InterceptDecision::Rewrite(new_tag, new_payload) => {
                envelope.method = new_tag;
                envelope.payload = new_payload;
                false
            }
            InterceptDecision::PassAndScheduleOffline => true,
            InterceptDecision::Pass => false,
        };

        let mut frame = self.buffer_pool.checkout();
        Self::encode_frame(&mut frame, &envelope);
        let send_res = self.transport.send_frame(account_id, dc_id, &frame).await;
        self.buffer_pool.checkin(frame);

        let mut guard = self
            .sessions
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        if let Some(slot) = guard.get_mut(&account_id) {
            slot.queued_outbound = slot.queued_outbound.saturating_sub(1);
            match &send_res {
                Ok(_) => {
                    slot.sent_packets = slot.sent_packets.saturating_add(1);
                    slot.reconnect_attempts = 0;
                }
                Err(_) => {
                    slot.reconnect_attempts = slot.reconnect_attempts.saturating_add(1);
                }
            }
        }

        let bytes_written = send_res?;
        Ok(DispatchOutcome::Sent {
            bytes_written,
            scheduled_offline,
        })
    }

    /// Dispatches an envelope with bounded exponential backoff retries on transient disconnects.
    pub async fn dispatch_with_retry(
        &self,
        account_id: AccountId,
        envelope: EncryptedEnvelope,
        max_retries: u32,
    ) -> Result<DispatchOutcome, MtprotoError> {
        let mut attempt = 0u32;
        loop {
            match self.dispatch_envelope(account_id, envelope.clone()).await {
                Ok(outcome) => return Ok(outcome),
                Err(err @ MtprotoError::Disconnected { .. }) if attempt < max_retries => {
                    let delay = self
                        .reconnect_policy
                        .delay_for_attempt(attempt, account_id.0);
                    tokio::time::sleep(delay).await;
                    attempt = attempt.saturating_add(1);
                    let _ = err;
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn encode_frame(buf: &mut BytesMut, envelope: &EncryptedEnvelope) {
        buf.put_slice(&envelope.auth_key_id.0);
        buf.put_slice(&envelope.msg_key);
        buf.put_u64_le(envelope.session_id.0);
        buf.put_i64_le(envelope.msg_id);
        buf.put_i32_le(envelope.seq_no);
        let tag_code = match envelope.method {
            TlMethodTag::ReadHistory => 1u32,
            TlMethodTag::ReadChannelHistory => 2u32,
            TlMethodTag::ReadStories => 3u32,
            TlMethodTag::UpdateStatusOnline => 4u32,
            TlMethodTag::UpdateStatusOffline => 5u32,
            TlMethodTag::SetTyping => 6u32,
            TlMethodTag::SendMessage => 7u32,
            TlMethodTag::Ping => 8u32,
            TlMethodTag::Raw(raw) => raw,
        };
        buf.put_u32_le(tag_code);
        buf.put_slice(&envelope.payload);
    }
}

impl<T: AsyncTransport> NetworkTransport for MtprotoMultiplexer<T> {
    fn is_connected(&self, account_id: AccountId) -> bool {
        self.transport.is_connected(account_id)
    }

    fn queued_outbound_count(&self, account_id: AccountId) -> usize {
        self.sessions
            .lock()
            .ok()
            .and_then(|g| g.get(&account_id).map(|s| s.queued_outbound))
            .unwrap_or(0)
    }
}
