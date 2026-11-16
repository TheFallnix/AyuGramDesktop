use std::collections::HashMap;
use std::sync::Mutex;

use ayu_core::{AccountId, DialogId, MsgId, SyncStateSnapshot};

use crate::error::MtprotoError;

/// Result of checking an incoming `(pts, pts_count)` against the local MTProto sync cursor.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PtsCheckResult {
    Uninited,
    Applied { new_pts: i32 },
    DuplicateIgnored { current_pts: i32 },
    GapDetected { expected_pts: i32, received_pts: i32 },
}

/// Outbound message awaiting server confirmation (`random_id -> MsgId`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingOutgoingMessage {
    pub random_id: u64,
    pub dialog_id: DialogId,
    pub local_msg_id: MsgId,
    pub queued_at_unix: i32,
}

#[derive(Default)]
struct AccountUpdatesState {
    channel_pts: HashMap<u64, i32>,
    pending_outgoing: HashMap<u64, PendingOutgoingMessage>,
    pending_gaps: usize,
    state: SyncStateSnapshot,
    requesting_difference: bool,
}

/// Coordinates per-account `pts`/`qts`/`date`/`seq`, channel `pts`, and `random_id` acks.
pub struct UpdatesSynchronizer {
    accounts: Mutex<HashMap<AccountId, AccountUpdatesState>>,
}

impl UpdatesSynchronizer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            accounts: Mutex::new(HashMap::new()),
        }
    }

    pub fn init_state(
        &self,
        account_id: AccountId,
        snapshot: SyncStateSnapshot,
    ) -> Result<(), MtprotoError> {
        let mut guard = self
            .accounts
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let entry = guard.entry(account_id).or_default();
        if snapshot.pts > 0 {
            entry.state.pts = snapshot.pts;
        }
        if snapshot.qts > 0 {
            entry.state.qts = snapshot.qts;
        }
        if snapshot.date > entry.state.date {
            entry.state.date = snapshot.date;
        }
        if snapshot.seq > 0 {
            entry.state.seq = snapshot.seq;
        }
        entry.pending_gaps = 0;
        entry.requesting_difference = false;
        Ok(())
    }

    pub fn state_snapshot(&self, account_id: AccountId) -> Result<SyncStateSnapshot, MtprotoError> {
        let guard = self
            .accounts
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        Ok(guard
            .get(&account_id)
            .map(|entry| entry.state)
            .unwrap_or_default())
    }

    pub fn init_channel_pts(
        &self,
        account_id: AccountId,
        channel_id: u64,
        pts: i32,
    ) -> Result<(), MtprotoError> {
        let mut guard = self
            .accounts
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let entry = guard.entry(account_id).or_default();
        if pts > 0 {
            entry.channel_pts.insert(channel_id, pts);
        }
        Ok(())
    }

    pub fn channel_pts(&self, account_id: AccountId, channel_id: u64) -> Result<i32, MtprotoError> {
        let guard = self
            .accounts
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        Ok(guard
            .get(&account_id)
            .and_then(|entry| entry.channel_pts.get(&channel_id).copied())
            .unwrap_or(0))
    }

    pub fn pending_gaps(&self, account_id: AccountId) -> Result<usize, MtprotoError> {
        let guard = self
            .accounts
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        Ok(guard
            .get(&account_id)
            .map(|entry| entry.pending_gaps)
            .unwrap_or(0))
    }

    pub fn set_requesting_difference(
        &self,
        account_id: AccountId,
        requesting: bool,
    ) -> Result<(), MtprotoError> {
        let mut guard = self
            .accounts
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let entry = guard.entry(account_id).or_default();
        entry.requesting_difference = requesting;
        Ok(())
    }

    pub fn is_requesting_difference(&self, account_id: AccountId) -> Result<bool, MtprotoError> {
        let guard = self
            .accounts
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        Ok(guard
            .get(&account_id)
            .map(|entry| entry.requesting_difference)
            .unwrap_or(false))
    }

    pub fn apply_pts(
        &self,
        account_id: AccountId,
        channel_id: u64,
        pts: i32,
        pts_count: i32,
    ) -> Result<PtsCheckResult, MtprotoError> {
        let mut guard = self
            .accounts
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let entry = guard.entry(account_id).or_default();
        let current = if channel_id == 0 {
            &mut entry.state.pts
        } else {
            entry.channel_pts.entry(channel_id).or_insert(0)
        };

        if *current == 0 {
            if pts > 0 {
                *current = pts;
                return Ok(PtsCheckResult::Applied { new_pts: pts });
            }
            return Ok(PtsCheckResult::Uninited);
        }

        if pts <= 0 || pts_count <= 0 {
            return Ok(PtsCheckResult::Applied { new_pts: *current });
        }

        if pts <= *current {
            return Ok(PtsCheckResult::DuplicateIgnored {
                current_pts: *current,
            });
        }

        let expected = current.saturating_add(pts_count);
        if pts <= expected {
            *current = pts;
            return Ok(PtsCheckResult::Applied { new_pts: pts });
        }

        entry.pending_gaps = entry.pending_gaps.saturating_add(1);
        Ok(PtsCheckResult::GapDetected {
            expected_pts: expected,
            received_pts: pts,
        })
    }

    pub fn queue_outgoing(
        &self,
        account_id: AccountId,
        random_id: u64,
        dialog_id: DialogId,
        local_msg_id: MsgId,
        date_unix: i32,
    ) -> Result<(), MtprotoError> {
        let mut guard = self
            .accounts
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let entry = guard.entry(account_id).or_default();
        entry.pending_outgoing.insert(
            random_id,
            PendingOutgoingMessage {
                random_id,
                dialog_id,
                local_msg_id,
                queued_at_unix: date_unix,
            },
        );
        Ok(())
    }

    pub fn confirm_outgoing(
        &self,
        account_id: AccountId,
        random_id: u64,
    ) -> Result<Option<PendingOutgoingMessage>, MtprotoError> {
        let mut guard = self
            .accounts
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        Ok(guard
            .get_mut(&account_id)
            .and_then(|entry| entry.pending_outgoing.remove(&random_id)))
    }

    pub fn remove_account(&self, account_id: AccountId) -> Result<(), MtprotoError> {
        let mut guard = self
            .accounts
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        guard.remove(&account_id);
        Ok(())
    }
}

impl Default for UpdatesSynchronizer {
    fn default() -> Self {
        Self::new()
    }
}
