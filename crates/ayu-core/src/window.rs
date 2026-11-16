use std::collections::VecDeque;

use crate::error::CoreError;
use crate::ids::{AccountId, DialogId, MsgId};
use crate::models::{CompactDialog, CompactMessage};
use crate::traits::{DialogRepository, MessageRepository};

/// Sliding in-memory viewport over a dialog's message history.
///
/// Only `capacity` messages around the visible scroll region are retained in RAM;
/// older or off-screen messages reside on disk in `MessageRepository` and are loaded lazily.
pub struct VirtualizedHistoryWindow<R: MessageRepository> {
    repo: R,
    account_id: AccountId,
    dialog_id: DialogId,
    capacity: usize,
    items: VecDeque<CompactMessage>,
}

impl<R: MessageRepository> VirtualizedHistoryWindow<R> {
    /// Creates a new sliding window backed by `repo` with a strict upper item bound.
    pub fn new(
        repo: R,
        account_id: AccountId,
        dialog_id: DialogId,
        capacity: usize,
    ) -> Result<Self, CoreError> {
        if capacity == 0 {
            return Err(CoreError::InvalidWindowCapacity(capacity));
        }
        Ok(Self {
            repo,
            account_id,
            dialog_id,
            capacity,
            items: VecDeque::with_capacity(capacity.min(128)),
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    #[must_use]
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub fn dialog_id(&self) -> DialogId {
        self.dialog_id
    }

    #[must_use]
    pub fn account_id(&self) -> AccountId {
        self.account_id
    }

    /// Returns a read-only view of messages currently in RAM without cloning.
    #[must_use]
    pub fn as_slices(&self) -> (&[CompactMessage], &[CompactMessage]) {
        self.items.as_slices()
    }

    /// Estimates the exact RAM bytes currently pinned by this viewport.
    #[must_use]
    pub fn estimated_ram_bytes(&self) -> usize {
        self.items.iter().map(CompactMessage::estimated_memory_bytes).sum()
    }

    /// Replaces the current viewport by jumping directly around `center_msg_id`.
    pub fn jump_to(&mut self, center_msg_id: MsgId) -> Result<&[CompactMessage], CoreError> {
        let loaded = self.repo.load_slice_around(
            self.account_id,
            self.dialog_id,
            center_msg_id,
            self.capacity,
        )?;
        self.items.clear();
        for msg in loaded.into_iter().take(self.capacity) {
            self.items.push_back(msg);
        }
        self.items.make_contiguous();
        Ok(self.items.as_slices().0)
    }

    /// Loads up to `batch_size` older messages and evicts newest off-screen entries above `capacity`.
    pub fn scroll_up(&mut self, batch_size: usize) -> Result<usize, CoreError> {
        if batch_size == 0 {
            return Ok(0);
        }
        let anchor = self.items.front().map(|m| m.id).unwrap_or(MsgId(i32::MAX));
        let older = self.repo.load_older_than(
            self.account_id,
            self.dialog_id,
            anchor,
            batch_size.min(self.capacity),
        )?;
        let inserted = older.len();
        for msg in older.into_iter().rev() {
            if self.items.len() >= self.capacity {
                self.items.pop_back();
            }
            self.items.push_front(msg);
        }
        self.items.make_contiguous();
        Ok(inserted)
    }

    /// Loads up to `batch_size` newer messages and evicts oldest off-screen entries above `capacity`.
    pub fn scroll_down(&mut self, batch_size: usize) -> Result<usize, CoreError> {
        if batch_size == 0 {
            return Ok(0);
        }
        let anchor = self.items.back().map(|m| m.id).unwrap_or(MsgId(0));
        let newer = self.repo.load_newer_than(
            self.account_id,
            self.dialog_id,
            anchor,
            batch_size.min(self.capacity),
        )?;
        let inserted = newer.len();
        for msg in newer {
            if self.items.len() >= self.capacity {
                self.items.pop_front();
            }
            self.items.push_back(msg);
        }
        self.items.make_contiguous();
        Ok(inserted)
    }

    /// Appends a live incoming message at the bottom while honoring `capacity`.
    pub fn push_live_message(&mut self, msg: CompactMessage) {
        if let Some(existing) = self.items.iter_mut().find(|m| m.id == msg.id) {
            *existing = msg;
            return;
        }
        if self.items.len() >= self.capacity {
            self.items.pop_front();
        }
        self.items.push_back(msg);
    }

    /// Removes matching message IDs from the resident viewport without disk I/O.
    pub fn remove_messages_in_ram(&mut self, msg_ids: &[MsgId]) -> usize {
        let before = self.items.len();
        self.items.retain(|m| !msg_ids.contains(&m.id));
        before.saturating_sub(self.items.len())
    }

    /// Evicts resident messages up to `max_msg_id` inclusive after history clear.
    pub fn clear_up_till_in_ram(&mut self, max_msg_id: MsgId) -> usize {
        let before = self.items.len();
        self.items.retain(|m| m.id > max_msg_id);
        before.saturating_sub(self.items.len())
    }

    /// Evicts all messages from RAM when the user switches away from this chat or account.
    pub fn clear_ram(&mut self) {
        self.items.clear();
        self.items.shrink_to_fit();
    }
}

/// Sliding in-memory viewport over an account's dialog list.
///
/// Only `capacity` sorted dialog rows around the visible scroll offset are retained in RAM;
/// remaining dialogs stay indexed in SQLite WAL.
pub struct VirtualizedDialogWindow<D: DialogRepository> {
    repo: D,
    account_id: AccountId,
    folder_id: i32,
    capacity: usize,
    offset: usize,
    items: VecDeque<CompactDialog>,
}

impl<D: DialogRepository> VirtualizedDialogWindow<D> {
    pub fn new(
        repo: D,
        account_id: AccountId,
        folder_id: i32,
        capacity: usize,
    ) -> Result<Self, CoreError> {
        if capacity == 0 {
            return Err(CoreError::InvalidWindowCapacity(capacity));
        }
        Ok(Self {
            repo,
            account_id,
            folder_id,
            capacity,
            offset: 0,
            items: VecDeque::with_capacity(capacity.min(64)),
        })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    #[must_use]
    pub const fn offset(&self) -> usize {
        self.offset
    }

    #[must_use]
    pub const fn folder_id(&self) -> i32 {
        self.folder_id
    }

    #[must_use]
    pub const fn account_id(&self) -> AccountId {
        self.account_id
    }

    #[must_use]
    pub fn as_slices(&self) -> (&[CompactDialog], &[CompactDialog]) {
        self.items.as_slices()
    }

    #[must_use]
    pub fn estimated_ram_bytes(&self) -> usize {
        self.items.iter().map(CompactDialog::estimated_memory_bytes).sum()
    }

    pub fn load_page(&mut self, offset: usize) -> Result<&[CompactDialog], CoreError> {
        let rows = self.repo.load_dialog_slice(
            self.account_id,
            self.folder_id,
            offset,
            self.capacity,
        )?;
        self.offset = offset;
        self.items.clear();
        for dialog in rows.into_iter().take(self.capacity) {
            self.items.push_back(dialog);
        }
        self.items.make_contiguous();
        Ok(self.items.as_slices().0)
    }

    pub fn scroll_down(&mut self, step: usize) -> Result<&[CompactDialog], CoreError> {
        let next_offset = self.offset.saturating_add(step);
        self.load_page(next_offset)
    }

    pub fn scroll_up(&mut self, step: usize) -> Result<&[CompactDialog], CoreError> {
        let next_offset = self.offset.saturating_sub(step);
        self.load_page(next_offset)
    }

    pub fn clear_ram(&mut self) {
        self.items.clear();
        self.items.shrink_to_fit();
        self.offset = 0;
    }
}
