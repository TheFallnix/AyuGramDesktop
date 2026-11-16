use std::cmp::Reverse;
use std::collections::HashMap;
use std::sync::Mutex;

use ayu_core::AccountId;

use crate::error::MtprotoError;

pub const DOWNLOAD_PART_SIZE: usize = 128 * 1024;
pub const UPLOAD_PART_SIZE_TINY: usize = 32 * 1024;
pub const UPLOAD_PART_SIZE_SMALL: usize = 64 * 1024;
pub const UPLOAD_PART_SIZE_MEDIUM: usize = 128 * 1024;
pub const UPLOAD_PART_SIZE_LARGE: usize = 256 * 1024;
pub const UPLOAD_PART_SIZE_XLARGE: usize = 512 * 1024;
pub const UPLOAD_MAX_PARTS_COUNT: u64 = 4000;
pub const BIG_FILE_THRESHOLD: u64 = 30 * 1024 * 1024;
pub const DEFAULT_MAX_INFLIGHT_BYTES: usize = 8 * 1024 * 1024;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TransferDirection {
    Download = 0,
    Upload = 1,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TransferState {
    Queued = 0,
    Active = 1,
    Completed = 2,
    Cancelled = 3,
    Failed = 4,
}

/// Snapshot of a chunked download/upload task's state without holding file payloads.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TransferProgress {
    pub task_id: u64,
    pub account_id: AccountId,
    pub file_id: u64,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub part_size: usize,
    pub priority: i32,
    pub parts_total: u32,
    pub parts_completed: u32,
    pub dc_id: u16,
    pub direction: TransferDirection,
    pub state: TransferState,
    pub is_big_file: bool,
}

struct TransferTask {
    account_id: AccountId,
    file_id: u64,
    transferred_bytes: u64,
    total_bytes: u64,
    part_size: usize,
    reserved_inflight_bytes: usize,
    priority: i32,
    parts_total: u32,
    parts_completed: u32,
    dc_id: u16,
    direction: TransferDirection,
    state: TransferState,
    is_big_file: bool,
}

impl TransferTask {
    fn snapshot(&self, task_id: u64) -> TransferProgress {
        TransferProgress {
            task_id,
            account_id: self.account_id,
            file_id: self.file_id,
            transferred_bytes: self.transferred_bytes,
            total_bytes: self.total_bytes,
            part_size: self.part_size,
            priority: self.priority,
            parts_total: self.parts_total,
            parts_completed: self.parts_completed,
            dc_id: self.dc_id,
            direction: self.direction,
            state: self.state,
            is_big_file: self.is_big_file,
        }
    }
}

struct TransferRegistry {
    tasks: HashMap<u64, TransferTask>,
    next_task_id: u64,
    active_inflight_bytes: usize,
}

/// Computes Telegram MTProto upload chunk size so parts remain within the 4000-part limit.
#[must_use]
pub fn compute_upload_part_size(total_bytes: u64) -> usize {
    const LIMIT_0: u64 = 1024 * 1024;
    const LIMIT_1: u64 = 32 * LIMIT_0;

    let fits = |part_size: usize| -> bool {
        total_bytes.div_ceil(part_size as u64) <= UPLOAD_MAX_PARTS_COUNT
    };

    if total_bytes < LIMIT_0 && fits(UPLOAD_PART_SIZE_TINY) {
        return UPLOAD_PART_SIZE_TINY;
    }
    if total_bytes <= LIMIT_1 && fits(UPLOAD_PART_SIZE_SMALL) {
        return UPLOAD_PART_SIZE_SMALL;
    }
    if fits(UPLOAD_PART_SIZE_MEDIUM) {
        return UPLOAD_PART_SIZE_MEDIUM;
    }
    if fits(UPLOAD_PART_SIZE_LARGE) {
        return UPLOAD_PART_SIZE_LARGE;
    }
    UPLOAD_PART_SIZE_XLARGE
}

/// Priority-ordered, RAM-bounded coordinator for chunked MTProto file downloads and uploads.
pub struct FileTransferManager {
    registry: Mutex<TransferRegistry>,
    max_inflight_bytes: usize,
}

impl FileTransferManager {
    #[must_use]
    pub fn new(max_inflight_bytes: usize) -> Self {
        Self {
            registry: Mutex::new(TransferRegistry {
                tasks: HashMap::new(),
                next_task_id: 1,
                active_inflight_bytes: 0,
            }),
            max_inflight_bytes: max_inflight_bytes.max(UPLOAD_PART_SIZE_TINY),
        }
    }

    pub fn enqueue_download(
        &self,
        account_id: AccountId,
        dc_id: u16,
        file_id: u64,
        total_bytes: u64,
        priority: i32,
    ) -> Result<u64, MtprotoError> {
        self.enqueue(
            account_id,
            dc_id,
            file_id,
            total_bytes,
            priority,
            DOWNLOAD_PART_SIZE,
            TransferDirection::Download,
        )
    }

    pub fn enqueue_upload(
        &self,
        account_id: AccountId,
        dc_id: u16,
        file_id: u64,
        total_bytes: u64,
    ) -> Result<u64, MtprotoError> {
        let part_size = compute_upload_part_size(total_bytes);
        self.enqueue(
            account_id,
            dc_id,
            file_id,
            total_bytes,
            0,
            part_size,
            TransferDirection::Upload,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn enqueue(
        &self,
        account_id: AccountId,
        dc_id: u16,
        file_id: u64,
        total_bytes: u64,
        priority: i32,
        part_size: usize,
        direction: TransferDirection,
    ) -> Result<u64, MtprotoError> {
        let mut guard = self
            .registry
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let task_id = guard.next_task_id;
        guard.next_task_id = guard.next_task_id.saturating_add(1);
        let parts_total = if total_bytes == 0 {
            0
        } else {
            u32::try_from(total_bytes.div_ceil(part_size as u64)).unwrap_or(u32::MAX)
        };
        guard.tasks.insert(
            task_id,
            TransferTask {
                account_id,
                file_id,
                transferred_bytes: 0,
                total_bytes,
                part_size,
                reserved_inflight_bytes: 0,
                priority,
                parts_total,
                parts_completed: 0,
                dc_id,
                direction,
                state: TransferState::Queued,
                is_big_file: total_bytes > BIG_FILE_THRESHOLD,
            },
        );
        Ok(task_id)
    }

    pub fn advance_transfer(
        &self,
        task_id: u64,
        chunk_bytes: usize,
    ) -> Result<TransferProgress, MtprotoError> {
        let mut guard = self
            .registry
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let reg = &mut *guard;
        let task = reg
            .tasks
            .get_mut(&task_id)
            .ok_or_else(|| MtprotoError::Framing(format!("unknown transfer task {task_id}")))?;

        match task.state {
            TransferState::Completed => return Ok(task.snapshot(task_id)),
            TransferState::Cancelled => {
                return Err(MtprotoError::Framing(format!(
                    "transfer task {task_id} was cancelled"
                )));
            }
            TransferState::Failed => {
                return Err(MtprotoError::Framing(format!(
                    "transfer task {task_id} failed"
                )));
            }
            TransferState::Queued | TransferState::Active => {}
        }

        if chunk_bytes > self.max_inflight_bytes {
            return Err(MtprotoError::Framing(format!(
                "chunk size {chunk_bytes} exceeds max inflight limit {}",
                self.max_inflight_bytes
            )));
        }

        let prev_reserved = task.reserved_inflight_bytes;
        let base_inflight = reg.active_inflight_bytes.saturating_sub(prev_reserved);
        if task.state == TransferState::Queued {
            let needed = usize::try_from(task.total_bytes)
                .unwrap_or(task.part_size)
                .min(task.part_size);
            if base_inflight.saturating_add(needed) > self.max_inflight_bytes {
                return Err(MtprotoError::Framing(
                    "in-flight transfer buffer budget exhausted".to_owned(),
                ));
            }
        }

        task.transferred_bytes = task
            .transferred_bytes
            .saturating_add(chunk_bytes as u64)
            .min(task.total_bytes);
        if chunk_bytes > 0 {
            let completed = u32::try_from(task.transferred_bytes.div_ceil(task.part_size as u64))
                .unwrap_or(task.parts_total);
            task.parts_completed = completed.min(task.parts_total);
        }

        let new_reserved = if task.transferred_bytes >= task.total_bytes {
            task.state = TransferState::Completed;
            0
        } else {
            task.state = TransferState::Active;
            let remaining = task.total_bytes.saturating_sub(task.transferred_bytes);
            usize::try_from(remaining)
                .unwrap_or(task.part_size)
                .min(task.part_size)
        };

        let next_inflight = base_inflight.saturating_add(new_reserved);
        if next_inflight > self.max_inflight_bytes {
            return Err(MtprotoError::Framing(
                "in-flight transfer buffer budget exhausted".to_owned(),
            ));
        }
        reg.active_inflight_bytes = next_inflight;
        task.reserved_inflight_bytes = new_reserved;

        Ok(task.snapshot(task_id))
    }

    pub fn cancel_transfer(&self, task_id: u64) -> Result<bool, MtprotoError> {
        let mut guard = self
            .registry
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let reg = &mut *guard;
        let Some(task) = reg.tasks.get_mut(&task_id) else {
            return Ok(false);
        };
        if matches!(
            task.state,
            TransferState::Completed | TransferState::Cancelled | TransferState::Failed
        ) {
            return Ok(false);
        }
        reg.active_inflight_bytes = reg
            .active_inflight_bytes
            .saturating_sub(task.reserved_inflight_bytes);
        task.reserved_inflight_bytes = 0;
        task.state = TransferState::Cancelled;
        Ok(true)
    }

    pub fn cancel_account_transfers(&self, account_id: AccountId) -> Result<usize, MtprotoError> {
        let mut guard = self
            .registry
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let reg = &mut *guard;
        let mut cancelled = 0usize;
        for task in reg.tasks.values_mut() {
            if task.account_id == account_id
                && matches!(task.state, TransferState::Queued | TransferState::Active)
            {
                reg.active_inflight_bytes = reg
                    .active_inflight_bytes
                    .saturating_sub(task.reserved_inflight_bytes);
                task.reserved_inflight_bytes = 0;
                task.state = TransferState::Cancelled;
                cancelled = cancelled.saturating_add(1);
            }
        }
        Ok(cancelled)
    }

    pub fn progress(&self, task_id: u64) -> Result<Option<TransferProgress>, MtprotoError> {
        let guard = self
            .registry
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        Ok(guard.tasks.get(&task_id).map(|t| t.snapshot(task_id)))
    }

    pub fn next_ready_download(&self, dc_id: u16) -> Result<Option<TransferProgress>, MtprotoError> {
        let guard = self
            .registry
            .lock()
            .map_err(|_| MtprotoError::LockPoisoned)?;
        let best = guard
            .tasks
            .iter()
            .filter(|(_, task)| {
                task.dc_id == dc_id
                    && task.direction == TransferDirection::Download
                    && matches!(task.state, TransferState::Queued | TransferState::Active)
            })
            .max_by_key(|(id, task)| (task.priority, Reverse(**id)))
            .map(|(id, task)| task.snapshot(*id));
        Ok(best)
    }

    #[must_use]
    pub fn active_inflight_bytes(&self) -> usize {
        self.registry
            .lock()
            .map(|r| r.active_inflight_bytes)
            .unwrap_or(0)
    }
}

impl Default for FileTransferManager {
    fn default() -> Self {
        Self::new(DEFAULT_MAX_INFLIGHT_BYTES)
    }
}
