use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use ayu_core::{
    AccountController, AccountId, AuthStateRepository, AyuAuditRepository, AyuDeletedMessage,
    AyuEditedRevision, AyuFilterEngine, CompactDialog, CompactMessage, DialogFlags, DialogId,
    DialogRepository, GhostModeConfig, GhostModeFlags, MediaCacheKind, MediaDescriptor, MediaKind,
    MessageFlags, MessageRepository, MsgId, MtpAuthorizationBlob, MultiAccountOrchestrator, PeerId,
    RegexFilterGlobalExclusion, RegexFilterRule, SendWithoutSoundOption, SyncStateRepository,
    SyncStateSnapshot, TopicId, UserId, VirtualizedDialogWindow, VirtualizedHistoryWindow,
};
use ayu_mtproto::{
    AsyncTransport, AuthKey, FileTransferManager, MtprotoError, MtprotoMultiplexer, PtsCheckResult,
    SessionId, TransferProgress, TransportFuture, UpdatesSynchronizer, DEFAULT_MAX_INFLIGHT_BYTES,
};
use ayu_storage::{ByteBoundedMediaCache, SqliteStorage};
use smallvec::SmallVec;
use tokio::runtime::Runtime;

#[cfg(all(target_os = "linux", feature = "jemalloc"))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[allow(clippy::too_many_arguments)]
#[cxx::bridge(namespace = "ayu::ffi")]
mod ffi {
    #[derive(Clone, Debug)]
    pub struct FfiMessageRow {
        pub dialog_id: i64,
        pub msg_id: i32,
        pub sender_id: u64,
        pub reply_to_msg_id: i32,
        pub date_unix: i32,
        pub edit_date_unix: i32,
        pub views: u32,
        pub flags: u32,
        pub grouped_id: u64,
        pub author_name: String,
        pub text: String,
        pub media_kind: u8,
        pub media_path: String,
    }

    #[derive(Clone, Debug)]
    pub struct FfiDialogRow {
        pub dialog_id: i64,
        pub peer_id: u64,
        pub folder_id: i32,
        pub title: String,
        pub draft_text: String,
        pub top_message_id: i32,
        pub read_inbox_max_id: i32,
        pub read_outbox_max_id: i32,
        pub last_message_date: i32,
        pub unread_count: u32,
        pub unread_mentions_count: u32,
        pub flags: u16,
    }

    #[derive(Copy, Clone, Debug)]
    pub struct FfiGhostConfig {
        pub ghost_mode_active: bool,
        pub send_read_messages: bool,
        pub send_read_stories: bool,
        pub send_online_packets: bool,
        pub send_upload_progress: bool,
        pub send_offline_after_online: bool,
        pub mark_read_after_action: bool,
        pub use_scheduled_messages: bool,
        pub send_without_sound: u8,
    }

    #[derive(Copy, Clone, Debug)]
    pub struct FfiMemoryStats {
        pub active_accounts: usize,
        pub resident_windows: usize,
        pub cached_window_messages: usize,
        pub cached_window_bytes: usize,
        pub resident_dialog_windows: usize,
        pub cached_dialog_rows: usize,
        pub media_cache_entries: usize,
        pub media_cache_bytes: usize,
        pub media_cache_limit_bytes: usize,
        pub process_rss_bytes: usize,
    }

    #[derive(Clone, Debug)]
    pub struct FfiResultStatus {
        pub ok: bool,
        pub error_message: String,
    }

    #[derive(Copy, Clone, Debug)]
    pub struct FfiAccountSnapshot {
        pub account_id: u64,
        pub user_id: u64,
        pub session_unique_id: u64,
        pub main_dc_id: i32,
        pub auth_state: u8,
        pub lifecycle_state: u8,
        pub is_test_dc: bool,
        pub is_premium: bool,
        pub unread_count: u32,
        pub unread_muted: bool,
    }

    #[derive(Copy, Clone, Debug)]
    pub struct FfiUnreadBadge {
        pub total_unread: u32,
        pub all_muted: bool,
    }

    #[derive(Copy, Clone, Debug)]
    pub struct FfiSyncState {
        pub pts: i32,
        pub qts: i32,
        pub date: i32,
        pub seq: i32,
    }

    #[derive(Copy, Clone, Debug)]
    pub struct FfiPtsOutcome {
        pub applied: bool,
        pub gap_detected: bool,
        pub duplicate: bool,
        pub current_pts: i32,
    }

    #[derive(Copy, Clone, Debug)]
    pub struct FfiTransferProgress {
        pub task_id: u64,
        pub file_id: u64,
        pub transferred_bytes: u64,
        pub total_bytes: u64,
        pub part_size: usize,
        pub parts_total: u32,
        pub parts_completed: u32,
        pub state: u8,
        pub is_big_file: bool,
    }

    #[derive(Clone, Debug)]
    pub struct FfiRegexFilter {
        pub id: Vec<u8>,
        pub pattern: String,
        pub enabled: bool,
        pub reversed: bool,
        pub case_insensitive: bool,
        pub has_dialog_id: bool,
        pub dialog_id: i64,
    }

    extern "Rust" {
        type AyuRustEngine;

        fn create_ayu_engine(
            db_path: &str,
            media_cache_limit_mb: usize,
        ) -> Result<Box<AyuRustEngine>>;

        fn health_check(self: &AyuRustEngine) -> FfiResultStatus;

        fn register_account(
            self: &AyuRustEngine,
            account_id: u64,
            user_id: u64,
            is_test_dc: bool,
        ) -> Result<()>;
        fn activate_account(self: &AyuRustEngine, account_id: u64) -> Result<()>;
        fn remove_account(self: &AyuRustEngine, account_id: u64) -> Result<()>;

        fn import_mtp_authorization(
            self: &AyuRustEngine,
            account_id: u64,
            serialized: &[u8],
        ) -> Result<FfiAccountSnapshot>;
        fn export_mtp_authorization(
            self: &AyuRustEngine,
            account_id: u64,
        ) -> Result<Vec<u8>>;
        fn complete_authorization(
            self: &AyuRustEngine,
            account_id: u64,
            user_id: u64,
            main_dc_id: i32,
            is_premium: bool,
        ) -> Result<()>;
        fn begin_logout(self: &AyuRustEngine, account_id: u64) -> Result<()>;
        fn finish_logout(self: &AyuRustEngine, account_id: u64) -> Result<()>;
        fn force_logout(self: &AyuRustEngine, account_id: u64) -> Result<()>;
        fn push_login_code(
            self: &AyuRustEngine,
            account_id: u64,
            code: &str,
        ) -> Result<()>;
        fn take_login_code(self: &AyuRustEngine, account_id: u64) -> Result<String>;
        fn set_account_unread_state(
            self: &AyuRustEngine,
            account_id: u64,
            unread_count: u32,
            muted: bool,
        ) -> Result<()>;
        fn set_account_separate_window(
            self: &AyuRustEngine,
            account_id: u64,
            has_window: bool,
        ) -> Result<()>;
        fn mark_account_online(self: &AyuRustEngine, account_id: u64) -> Result<()>;
        fn collect_accounts_for_offline_packet(self: &AyuRustEngine) -> Vec<u64>;
        fn compute_unread_badge(self: &AyuRustEngine) -> FfiUnreadBadge;
        fn ordered_account_ids(
            self: &AyuRustEngine,
            order_unique_ids: &[u64],
        ) -> Vec<u64>;
        fn redundant_account_ids(self: &AyuRustEngine) -> Vec<u64>;
        fn max_accounts_limit(self: &AyuRustEngine) -> usize;
        fn authed_accounts_count(self: &AyuRustEngine) -> usize;
        fn get_account_snapshot(
            self: &AyuRustEngine,
            account_id: u64,
        ) -> Result<FfiAccountSnapshot>;

        fn set_ghost_config(
            self: &AyuRustEngine,
            account_id: u64,
            config: FfiGhostConfig,
        ) -> Result<()>;
        fn get_ghost_config(self: &AyuRustEngine, account_id: u64) -> Result<FfiGhostConfig>;

        fn should_send_read_ack(self: &AyuRustEngine, account_id: u64) -> bool;
        fn should_send_online_packet(self: &AyuRustEngine, account_id: u64) -> bool;

        fn upsert_message(
            self: &AyuRustEngine,
            account_id: u64,
            msg: FfiMessageRow,
        ) -> Result<()>;
        fn upsert_message_batch(
            self: &AyuRustEngine,
            account_id: u64,
            msgs: &[FfiMessageRow],
        ) -> Result<usize>;
        fn load_viewport_messages(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            center_msg_id: i32,
            limit: usize,
        ) -> Result<Vec<FfiMessageRow>>;
        fn scroll_viewport_older(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            batch_size: usize,
        ) -> Result<Vec<FfiMessageRow>>;
        fn scroll_viewport_newer(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            batch_size: usize,
        ) -> Result<Vec<FfiMessageRow>>;
        fn delete_messages(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            msg_ids: &[i32],
        ) -> Result<usize>;
        fn clear_dialog_history_up_to(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            max_msg_id: i32,
        ) -> Result<usize>;
        fn unload_dialog_viewport(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
        );
        fn search_dialog_messages(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            query: &str,
            limit: usize,
        ) -> Result<Vec<FfiMessageRow>>;

        fn upsert_dialog(
            self: &AyuRustEngine,
            account_id: u64,
            dialog: FfiDialogRow,
        ) -> Result<()>;
        fn upsert_dialog_batch(
            self: &AyuRustEngine,
            account_id: u64,
            dialogs: &[FfiDialogRow],
        ) -> Result<usize>;
        fn update_dialog_read_state(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            read_inbox_max_id: i32,
            unread_count: u32,
        ) -> Result<bool>;
        fn remove_dialog(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
        ) -> Result<bool>;
        fn load_dialog_slice(
            self: &AyuRustEngine,
            account_id: u64,
            folder_id: i32,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<FfiDialogRow>>;
        fn search_dialogs(
            self: &AyuRustEngine,
            account_id: u64,
            query: &str,
            limit: usize,
        ) -> Result<Vec<FfiDialogRow>>;

        fn record_deleted_message(
            self: &AyuRustEngine,
            account_id: u64,
            msg: FfiMessageRow,
        ) -> Result<()>;
        fn load_deleted_messages(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            limit: usize,
        ) -> Result<Vec<FfiMessageRow>>;
        fn has_deleted_messages(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
        ) -> bool;
        fn record_edited_message(
            self: &AyuRustEngine,
            account_id: u64,
            msg: FfiMessageRow,
        ) -> Result<()>;
        fn load_edited_messages(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            msg_id: i32,
            limit: usize,
        ) -> Result<Vec<FfiMessageRow>>;
        fn has_edited_revisions(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            msg_id: i32,
        ) -> bool;

        fn put_media_cache(self: &AyuRustEngine, key: u64, data: &[u8]) -> Result<()>;
        fn put_scoped_media_cache(
            self: &AyuRustEngine,
            account_id: u64,
            kind: u8,
            resource_id: u64,
            data: &[u8],
        ) -> Result<()>;
        fn get_scoped_media_cache(
            self: &AyuRustEngine,
            account_id: u64,
            kind: u8,
            resource_id: u64,
        ) -> Vec<u8>;
        fn evict_account_media_cache(
            self: &AyuRustEngine,
            account_id: u64,
            heavy_only: bool,
        ) -> usize;
        fn evict_inactive_caches(self: &AyuRustEngine) -> usize;
        fn get_memory_stats(self: &AyuRustEngine) -> FfiMemoryStats;

        fn init_sync_state(
            self: &AyuRustEngine,
            account_id: u64,
            pts: i32,
            qts: i32,
            date: i32,
            seq: i32,
        ) -> Result<()>;
        fn get_sync_state(self: &AyuRustEngine, account_id: u64) -> Result<FfiSyncState>;
        fn apply_pts_update(
            self: &AyuRustEngine,
            account_id: u64,
            channel_id: u64,
            pts: i32,
            pts_count: i32,
        ) -> Result<FfiPtsOutcome>;
        fn queue_outgoing_message(
            self: &AyuRustEngine,
            account_id: u64,
            random_id: u64,
            msg: FfiMessageRow,
        ) -> Result<i32>;
        fn confirm_outgoing_message(
            self: &AyuRustEngine,
            account_id: u64,
            random_id: u64,
            server_msg_id: i32,
            date_unix: i32,
            pts: i32,
            pts_count: i32,
        ) -> Result<bool>;
        fn receive_incoming_message(
            self: &AyuRustEngine,
            account_id: u64,
            channel_id: u64,
            pts: i32,
            pts_count: i32,
            msg: FfiMessageRow,
        ) -> Result<FfiPtsOutcome>;
        fn enqueue_file_download(
            self: &AyuRustEngine,
            account_id: u64,
            dc_id: u16,
            file_id: u64,
            total_bytes: u64,
            priority: i32,
        ) -> Result<u64>;
        fn enqueue_file_upload(
            self: &AyuRustEngine,
            account_id: u64,
            dc_id: u16,
            file_id: u64,
            total_bytes: u64,
        ) -> Result<u64>;
        fn advance_file_transfer(
            self: &AyuRustEngine,
            task_id: u64,
            chunk_bytes: usize,
        ) -> Result<FfiTransferProgress>;
        fn cancel_file_transfer(self: &AyuRustEngine, task_id: u64) -> bool;
        fn active_transfer_inflight_bytes(self: &AyuRustEngine) -> usize;

        fn query_deleted_messages(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            topic_id: i64,
            min_id: i32,
            max_id: i32,
            limit: usize,
            search_query: &str,
        ) -> Result<Vec<FfiMessageRow>>;
        fn remove_deleted_message(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            msg_id: i32,
        ) -> Result<usize>;
        fn clear_deleted_messages(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            topic_id: i64,
        ) -> Result<usize>;
        fn query_edited_messages(
            self: &AyuRustEngine,
            account_id: u64,
            dialog_id: i64,
            msg_id: i32,
            min_id: i64,
            max_id: i64,
            limit: usize,
        ) -> Result<Vec<FfiMessageRow>>;
        fn upsert_regex_filter(self: &AyuRustEngine, filter: FfiRegexFilter) -> Result<()>;
        fn delete_regex_filter(self: &AyuRustEngine, id: &[u8]) -> Result<bool>;
        fn clear_regex_filters(self: &AyuRustEngine) -> Result<usize>;
        fn list_regex_filters(self: &AyuRustEngine) -> Result<Vec<FfiRegexFilter>>;
        fn add_regex_exclusion(
            self: &AyuRustEngine,
            dialog_id: i64,
            filter_id: &[u8],
        ) -> Result<()>;
        fn remove_regex_exclusion(
            self: &AyuRustEngine,
            dialog_id: i64,
            filter_id: &[u8],
        ) -> Result<bool>;
        fn delete_exclusions_by_filter_id(
            self: &AyuRustEngine,
            filter_id: &[u8],
        ) -> Result<usize>;
        fn clear_regex_exclusions(self: &AyuRustEngine) -> Result<usize>;
        fn rebuild_filters_cache(self: &AyuRustEngine) -> Result<usize>;
        fn set_filters_enabled(
            self: &AyuRustEngine,
            enabled: bool,
            enabled_in_chats: bool,
            hide_from_blocked: bool,
        );
        fn set_shadow_banned(self: &AyuRustEngine, peer_dialog_id: i64, banned: bool);
        fn is_shadow_banned(self: &AyuRustEngine, peer_dialog_id: i64) -> bool;
        fn evaluate_message_filter(
            self: &AyuRustEngine,
            dialog_id: i64,
            msg_id: i32,
            sender_dialog_id: i64,
            is_outgoing: bool,
            text: &str,
        ) -> bool;
        fn invalidate_filtered_message(self: &AyuRustEngine, dialog_id: i64, msg_id: i32);
        fn has_filtered_messages(self: &AyuRustEngine, dialog_id: i64) -> bool;
        fn toggle_filtered_messages_shown(self: &AyuRustEngine, dialog_id: i64) -> bool;
        fn hide_message(self: &AyuRustEngine, dialog_id: i64, msg_id: i32);
        fn is_message_hidden(self: &AyuRustEngine, dialog_id: i64, msg_id: i32) -> bool;
        fn arm_disable_ghost_on_story_close(self: &AyuRustEngine, account_id: u64);
        fn consume_disable_ghost_on_story_close(self: &AyuRustEngine, account_id: u64) -> bool;
    }
}

pub use ffi::{
    FfiAccountSnapshot, FfiDialogRow, FfiGhostConfig, FfiMemoryStats, FfiMessageRow,
    FfiPtsOutcome, FfiRegexFilter, FfiResultStatus, FfiSyncState, FfiTransferProgress,
    FfiUnreadBadge,
};

struct Stage1BridgeTransport {
    connected: AtomicBool,
}

impl Stage1BridgeTransport {
    fn new() -> Self {
        Self {
            connected: AtomicBool::new(true),
        }
    }
}

impl AsyncTransport for Stage1BridgeTransport {
    fn send_frame<'a>(
        &'a self,
        _account_id: AccountId,
        dc_id: u16,
        frame: &'a [u8],
    ) -> TransportFuture<'a> {
        Box::pin(async move {
            if !self.connected.load(Ordering::Relaxed) {
                return Err(MtprotoError::Disconnected {
                    dc_id,
                    reason: "bridge transport offline".to_owned(),
                });
            }
            Ok(frame.len())
        })
    }

    fn is_connected(&self, _account_id: AccountId) -> bool {
        self.connected.load(Ordering::Relaxed)
    }
}

type ActiveWindowMap = HashMap<(AccountId, DialogId), VirtualizedHistoryWindow<Arc<SqliteStorage>>>;
type ActiveDialogWindowMap = HashMap<(AccountId, i32), VirtualizedDialogWindow<Arc<SqliteStorage>>>;

/// Core Rust backend engine exposed safely to C++/Qt through `cxx`.
pub struct AyuRustEngine {
    _runtime: Runtime,
    orchestrator: Mutex<MultiAccountOrchestrator>,
    storage: Arc<SqliteStorage>,
    media_cache: ByteBoundedMediaCache,
    multiplexer: MtprotoMultiplexer<Stage1BridgeTransport>,
    sync: UpdatesSynchronizer,
    transfers: FileTransferManager,
    filters: Mutex<AyuFilterEngine>,
    next_temp_msg_id: AtomicI32,
    windows: Mutex<ActiveWindowMap>,
    dialog_windows: Mutex<ActiveDialogWindowMap>,
}

const DEFAULT_MAX_RESIDENT_WINDOWS: usize = 2;
const DEFAULT_MAX_RESIDENT_DIALOG_WINDOWS: usize = 2;

impl AyuRustEngine {
    pub fn new(db_path: &str, media_cache_limit_mb: usize) -> Result<Self> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .thread_name("ayu-tokio")
            .build()
            .context("failed to build shared tokio runtime")?;

        let storage = if db_path == ":memory:" {
            Arc::new(SqliteStorage::open_in_memory().context("failed to open in-memory sqlite")?)
        } else {
            Arc::new(SqliteStorage::open(db_path).context("failed to open sqlite WAL database")?)
        };

        let limit_bytes = media_cache_limit_mb
            .max(1)
            .saturating_mul(1024 * 1024);
        let transport = Arc::new(Stage1BridgeTransport::new());

        let mut filters_engine = AyuFilterEngine::new();
        if let (Ok(rules), Ok(exclusions)) = (
            storage.load_regex_filters(),
            storage.load_regex_exclusions(),
        ) {
            let _ = filters_engine.rebuild(&rules, &exclusions);
        }

        Ok(Self {
            _runtime: runtime,
            orchestrator: Mutex::new(MultiAccountOrchestrator::new()),
            storage,
            media_cache: ByteBoundedMediaCache::new(limit_bytes),
            multiplexer: MtprotoMultiplexer::new(transport, 16),
            sync: UpdatesSynchronizer::new(),
            transfers: FileTransferManager::new(DEFAULT_MAX_INFLIGHT_BYTES),
            filters: Mutex::new(filters_engine),
            next_temp_msg_id: AtomicI32::new(-1),
            windows: Mutex::new(HashMap::new()),
            dialog_windows: Mutex::new(HashMap::new()),
        })
    }

    pub fn health_check(&self) -> FfiResultStatus {
        let wal_ok = self.storage.journal_mode().is_ok();
        FfiResultStatus {
            ok: wal_ok,
            error_message: if wal_ok {
                String::new()
            } else {
                "sqlite storage unavailable".to_owned()
            },
        }
    }

    pub fn register_account(
        &self,
        account_id: u64,
        user_id: u64,
        is_test_dc: bool,
    ) -> Result<()> {
        let id = AccountId(account_id);
        let mut orchestrator = self
            .orchestrator
            .lock()
            .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
        orchestrator.register_account(id, UserId(user_id), is_test_dc)?;
        self.multiplexer
            .register_session(id, 2, SessionId(account_id.wrapping_add(1)), GhostModeConfig::default())
            .map_err(|e| anyhow!(e))?;
        Ok(())
    }

    pub fn activate_account(&self, account_id: u64) -> Result<()> {
        let id = AccountId(account_id);
        {
            let mut orchestrator = self
                .orchestrator
                .lock()
                .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
            orchestrator.activate_account(id)?;
        }
        let mut windows = self
            .windows
            .lock()
            .map_err(|_| anyhow!("windows mutex poisoned"))?;
        windows.retain(|(acc, _), win| {
            let keep = *acc == id;
            if !keep {
                win.clear_ram();
            }
            keep
        });
        if let Ok(mut d_windows) = self.dialog_windows.lock() {
            d_windows.retain(|(acc, _), win| {
                let keep = *acc == id;
                if !keep {
                    win.clear_ram();
                }
                keep
            });
        }
        Ok(())
    }

    fn clear_account_windows(&self, id: AccountId) {
        if let Ok(mut windows) = self.windows.lock() {
            windows.retain(|(acc, _), win| {
                let keep = *acc != id;
                if !keep {
                    win.clear_ram();
                }
                keep
            });
        }
        if let Ok(mut d_windows) = self.dialog_windows.lock() {
            d_windows.retain(|(acc, _), win| {
                let keep = *acc != id;
                if !keep {
                    win.clear_ram();
                }
                keep
            });
        }
    }

    pub fn remove_account(&self, account_id: u64) -> Result<()> {
        let id = AccountId(account_id);
        {
            let mut orchestrator = self
                .orchestrator
                .lock()
                .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
            orchestrator.remove_account(id)?;
        }
        self.multiplexer
            .unregister_session(id)
            .map_err(|e| anyhow!(e))?;
        self.clear_account_windows(id);
        let _ = self.sync.remove_account(id);
        let _ = self.transfers.cancel_account_transfers(id);
        let _ = self.media_cache.evict_account(id, false);
        let _ = self.storage.delete_account_auth(id);
        Ok(())
    }

    pub fn import_mtp_authorization(
        &self,
        account_id: u64,
        serialized: &[u8],
    ) -> Result<FfiAccountSnapshot> {
        let id = AccountId(account_id);
        let blob = MtpAuthorizationBlob::decode_tdesktop(serialized)?;
        let snapshot = {
            let mut orchestrator = self
                .orchestrator
                .lock()
                .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
            let ctrl = orchestrator.get_mut(id)?;
            ctrl.apply_mtp_authorization(blob);
            self.sync_and_persist_auth(ctrl)?;
            snapshot_from_controller(ctrl)
        };
        Ok(snapshot)
    }

    pub fn export_mtp_authorization(&self, account_id: u64) -> Result<Vec<u8>> {
        let id = AccountId(account_id);
        let orchestrator = self
            .orchestrator
            .lock()
            .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
        let ctrl = orchestrator
            .get(id)
            .ok_or_else(|| anyhow!("account {account_id} is not registered"))?;
        Ok(ctrl.export_mtp_authorization().encode_tdesktop())
    }

    pub fn complete_authorization(
        &self,
        account_id: u64,
        user_id: u64,
        main_dc_id: i32,
        is_premium: bool,
    ) -> Result<()> {
        let id = AccountId(account_id);
        let mut orchestrator = self
            .orchestrator
            .lock()
            .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
        let ctrl = orchestrator.get_mut(id)?;
        ctrl.complete_authorization(UserId(user_id), main_dc_id, is_premium);
        self.sync_and_persist_auth(ctrl)?;
        Ok(())
    }

    pub fn begin_logout(&self, account_id: u64) -> Result<()> {
        let id = AccountId(account_id);
        let mut orchestrator = self
            .orchestrator
            .lock()
            .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
        let ctrl = orchestrator.get_mut(id)?;
        ctrl.begin_logout();
        self.sync_and_persist_auth(ctrl)?;
        Ok(())
    }

    pub fn finish_logout(&self, account_id: u64) -> Result<()> {
        let id = AccountId(account_id);
        {
            let mut orchestrator = self
                .orchestrator
                .lock()
                .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
            let ctrl = orchestrator.get_mut(id)?;
            ctrl.finish_logout();
        }
        self.storage.delete_account_auth(id)?;
        self.clear_account_windows(id);
        let _ = self.sync.remove_account(id);
        let _ = self.transfers.cancel_account_transfers(id);
        let _ = self.media_cache.evict_account(id, false);
        Ok(())
    }

    pub fn force_logout(&self, account_id: u64) -> Result<()> {
        let id = AccountId(account_id);
        {
            let mut orchestrator = self
                .orchestrator
                .lock()
                .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
            let ctrl = orchestrator.get_mut(id)?;
            ctrl.force_logout();
            self.sync_and_persist_auth(ctrl)?;
        }
        self.clear_account_windows(id);
        let _ = self.sync.remove_account(id);
        let _ = self.transfers.cancel_account_transfers(id);
        let _ = self.media_cache.evict_account(id, false);
        Ok(())
    }

    pub fn push_login_code(&self, account_id: u64, code: &str) -> Result<()> {
        let id = AccountId(account_id);
        let mut orchestrator = self
            .orchestrator
            .lock()
            .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
        orchestrator.get_mut(id)?.push_login_code(code);
        Ok(())
    }

    pub fn take_login_code(&self, account_id: u64) -> Result<String> {
        let id = AccountId(account_id);
        let mut orchestrator = self
            .orchestrator
            .lock()
            .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
        let code = orchestrator
            .get_mut(id)?
            .take_login_code()
            .map(|c| c.into_string())
            .unwrap_or_default();
        Ok(code)
    }

    pub fn set_account_unread_state(
        &self,
        account_id: u64,
        unread_count: u32,
        muted: bool,
    ) -> Result<()> {
        let mut orchestrator = self
            .orchestrator
            .lock()
            .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
        orchestrator
            .get_mut(AccountId(account_id))?
            .set_unread_state(unread_count, muted);
        Ok(())
    }

    pub fn set_account_separate_window(&self, account_id: u64, has_window: bool) -> Result<()> {
        let mut orchestrator = self
            .orchestrator
            .lock()
            .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
        orchestrator
            .get_mut(AccountId(account_id))?
            .set_separate_window(has_window);
        Ok(())
    }

    pub fn mark_account_online(&self, account_id: u64) -> Result<()> {
        let mut orchestrator = self
            .orchestrator
            .lock()
            .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
        orchestrator.get_mut(AccountId(account_id))?.mark_online();
        Ok(())
    }

    pub fn collect_accounts_for_offline_packet(&self) -> Vec<u64> {
        let Ok(mut orchestrator) = self.orchestrator.lock() else {
            return Vec::new();
        };
        orchestrator
            .collect_accounts_for_offline_packet()
            .into_iter()
            .map(|id| id.0)
            .collect()
    }

    pub fn compute_unread_badge(&self) -> FfiUnreadBadge {
        let Ok(orchestrator) = self.orchestrator.lock() else {
            return FfiUnreadBadge {
                total_unread: 0,
                all_muted: true,
            };
        };
        let summary = orchestrator.compute_unread_badge();
        FfiUnreadBadge {
            total_unread: summary.total_unread,
            all_muted: summary.all_muted,
        }
    }

    pub fn ordered_account_ids(&self, order_unique_ids: &[u64]) -> Vec<u64> {
        let Ok(orchestrator) = self.orchestrator.lock() else {
            return Vec::new();
        };
        orchestrator
            .ordered_account_ids(order_unique_ids)
            .into_iter()
            .map(|id| id.0)
            .collect()
    }

    pub fn redundant_account_ids(&self) -> Vec<u64> {
        let Ok(orchestrator) = self.orchestrator.lock() else {
            return Vec::new();
        };
        orchestrator
            .redundant_account_ids()
            .into_iter()
            .map(|id| id.0)
            .collect()
    }

    pub fn max_accounts_limit(&self) -> usize {
        self.orchestrator
            .lock()
            .map(|o| o.max_accounts())
            .unwrap_or(ayu_core::MAX_ACCOUNTS_FREE)
    }

    pub fn authed_accounts_count(&self) -> usize {
        self.orchestrator
            .lock()
            .map(|o| o.authed_accounts_count())
            .unwrap_or(0)
    }

    pub fn get_account_snapshot(&self, account_id: u64) -> Result<FfiAccountSnapshot> {
        let orchestrator = self
            .orchestrator
            .lock()
            .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
        let ctrl = orchestrator
            .get(AccountId(account_id))
            .ok_or_else(|| anyhow!("account {account_id} is not registered"))?;
        Ok(snapshot_from_controller(ctrl))
    }

    fn sync_and_persist_auth(&self, ctrl: &AccountController) -> Result<()> {
        let blob = ctrl.export_mtp_authorization();
        self.storage.save_account_auth(
            ctrl.account_id(),
            ctrl.index(),
            ctrl.is_test_dc(),
            ctrl.is_premium(),
            ctrl.auth_state(),
            &blob,
        )?;
        if blob.main_dc_id > 0 {
            let dc = blob.main_dc_id.min(i32::from(u16::MAX)) as u16;
            let _ = self.multiplexer.set_main_dc(ctrl.account_id(), dc);
        }
        for entry in &blob.keys {
            if let Ok(key) = AuthKey::from_slice(entry.key_bytes.as_slice()) {
                let _ = self
                    .multiplexer
                    .bind_dc_key(ctrl.account_id(), entry.dc_id, key);
            }
        }
        if !blob.keys_to_destroy.is_empty() {
            let mut destroy_list = Vec::with_capacity(blob.keys_to_destroy.len());
            for entry in &blob.keys_to_destroy {
                if let Ok(key) = AuthKey::from_slice(entry.key_bytes.as_slice()) {
                    destroy_list.push((entry.dc_id, key));
                }
            }
            if !destroy_list.is_empty() {
                let _ = self
                    .multiplexer
                    .schedule_keys_destroy(ctrl.account_id(), destroy_list);
            }
        }
        Ok(())
    }

    pub fn set_ghost_config(&self, account_id: u64, config: FfiGhostConfig) -> Result<()> {
        let id = AccountId(account_id);
        let core_cfg = ghost_from_ffi(config);
        {
            let mut orchestrator = self
                .orchestrator
                .lock()
                .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
            orchestrator.get_mut(id)?.set_ghost_config(core_cfg);
        }
        self.multiplexer
            .update_ghost_config(id, core_cfg)
            .map_err(|e| anyhow!(e))?;
        Ok(())
    }

    pub fn get_ghost_config(&self, account_id: u64) -> Result<FfiGhostConfig> {
        let id = AccountId(account_id);
        let orchestrator = self
            .orchestrator
            .lock()
            .map_err(|_| anyhow!("orchestrator mutex poisoned"))?;
        let ctrl = orchestrator
            .get(id)
            .ok_or_else(|| anyhow!("account {account_id} is not registered"))?;
        Ok(ghost_to_ffi(ctrl.ghost_config()))
    }

    pub fn should_send_read_ack(&self, account_id: u64) -> bool {
        let Ok(orchestrator) = self.orchestrator.lock() else {
            return true;
        };
        orchestrator
            .get(AccountId(account_id))
            .map(|c| c.ghost_config().should_send_read_messages())
            .unwrap_or(true)
    }

    pub fn should_send_online_packet(&self, account_id: u64) -> bool {
        let Ok(orchestrator) = self.orchestrator.lock() else {
            return true;
        };
        orchestrator
            .get(AccountId(account_id))
            .map(|c| c.ghost_config().should_send_online_packets())
            .unwrap_or(true)
    }

    pub fn upsert_message(&self, account_id: u64, msg: FfiMessageRow) -> Result<()> {
        let compact = message_from_ffi(&msg);
        self.storage
            .upsert_message(AccountId(account_id), &compact)?;
        Ok(())
    }

    pub fn upsert_message_batch(
        &self,
        account_id: u64,
        msgs: &[FfiMessageRow],
    ) -> Result<usize> {
        let batch: Vec<CompactMessage> = msgs.iter().map(message_from_ffi).collect();
        let written = self
            .storage
            .upsert_batch(AccountId(account_id), &batch)?;
        Ok(written)
    }

    pub fn load_viewport_messages(
        &self,
        account_id: u64,
        dialog_id: i64,
        center_msg_id: i32,
        limit: usize,
    ) -> Result<Vec<FfiMessageRow>> {
        let acc = AccountId(account_id);
        let dlg = DialogId(dialog_id);
        let window_cap = limit.max(1);

        let mut windows = self
            .windows
            .lock()
            .map_err(|_| anyhow!("windows mutex poisoned"))?;

        let key = (acc, dlg);
        let needs_recreate = match windows.get(&key) {
            Some(existing) => existing.capacity() != window_cap,
            None => true,
        };

        if needs_recreate {
            if windows.len() >= DEFAULT_MAX_RESIDENT_WINDOWS {
                let keys_to_evict: Vec<(AccountId, DialogId)> =
                    windows.keys().filter(|k| *k != &key).copied().collect();
                for old_key in keys_to_evict {
                    if let Some(mut old_win) = windows.remove(&old_key) {
                        old_win.clear_ram();
                    }
                }
            }
            let win = VirtualizedHistoryWindow::new(Arc::clone(&self.storage), acc, dlg, window_cap)?;
            windows.insert(key, win);
        }

        let win = windows
            .get_mut(&key)
            .ok_or_else(|| anyhow!("failed to access viewport window"))?;
        let slice = win.jump_to(MsgId(center_msg_id))?;
        Ok(slice.iter().map(message_to_ffi).collect())
    }

    pub fn scroll_viewport_older(
        &self,
        account_id: u64,
        dialog_id: i64,
        batch_size: usize,
    ) -> Result<Vec<FfiMessageRow>> {
        let acc = AccountId(account_id);
        let dlg = DialogId(dialog_id);
        let mut windows = self
            .windows
            .lock()
            .map_err(|_| anyhow!("windows mutex poisoned"))?;
        let win = windows
            .get_mut(&(acc, dlg))
            .ok_or_else(|| anyhow!("viewport window not initialized for dialog {dialog_id}"))?;
        let _ = win.scroll_up(batch_size)?;
        let (front, back) = win.as_slices();
        let mut out = Vec::with_capacity(front.len() + back.len());
        out.extend(front.iter().map(message_to_ffi));
        out.extend(back.iter().map(message_to_ffi));
        Ok(out)
    }

    pub fn scroll_viewport_newer(
        &self,
        account_id: u64,
        dialog_id: i64,
        batch_size: usize,
    ) -> Result<Vec<FfiMessageRow>> {
        let acc = AccountId(account_id);
        let dlg = DialogId(dialog_id);
        let mut windows = self
            .windows
            .lock()
            .map_err(|_| anyhow!("windows mutex poisoned"))?;
        let win = windows
            .get_mut(&(acc, dlg))
            .ok_or_else(|| anyhow!("viewport window not initialized for dialog {dialog_id}"))?;
        let _ = win.scroll_down(batch_size)?;
        let (front, back) = win.as_slices();
        let mut out = Vec::with_capacity(front.len() + back.len());
        out.extend(front.iter().map(message_to_ffi));
        out.extend(back.iter().map(message_to_ffi));
        Ok(out)
    }

    pub fn delete_messages(
        &self,
        account_id: u64,
        dialog_id: i64,
        msg_ids: &[i32],
    ) -> Result<usize> {
        let acc = AccountId(account_id);
        let dlg = DialogId(dialog_id);
        let ids: Vec<MsgId> = msg_ids.iter().copied().map(MsgId).collect();
        let deleted = self.storage.delete_messages(acc, dlg, &ids)?;
        if let Ok(mut windows) = self.windows.lock() {
            if let Some(win) = windows.get_mut(&(acc, dlg)) {
                let _ = win.remove_messages_in_ram(&ids);
            }
        }
        Ok(deleted)
    }

    pub fn clear_dialog_history_up_to(
        &self,
        account_id: u64,
        dialog_id: i64,
        max_msg_id: i32,
    ) -> Result<usize> {
        let acc = AccountId(account_id);
        let dlg = DialogId(dialog_id);
        let max_id = MsgId(max_msg_id);
        let deleted = self.storage.delete_messages_up_to(acc, dlg, max_id)?;
        if let Ok(mut windows) = self.windows.lock() {
            if let Some(win) = windows.get_mut(&(acc, dlg)) {
                let _ = win.clear_up_till_in_ram(max_id);
            }
        }
        Ok(deleted)
    }

    pub fn unload_dialog_viewport(&self, account_id: u64, dialog_id: i64) {
        let key = (AccountId(account_id), DialogId(dialog_id));
        if let Ok(mut windows) = self.windows.lock() {
            if let Some(mut win) = windows.remove(&key) {
                win.clear_ram();
            }
        }
    }

    pub fn search_dialog_messages(
        &self,
        account_id: u64,
        dialog_id: i64,
        query: &str,
        limit: usize,
    ) -> Result<Vec<FfiMessageRow>> {
        let rows = self.storage.search_dialog_messages(
            AccountId(account_id),
            DialogId(dialog_id),
            query,
            limit,
        )?;
        Ok(rows.iter().map(message_to_ffi).collect())
    }

    pub fn upsert_dialog(&self, account_id: u64, dialog: FfiDialogRow) -> Result<()> {
        let compact = dialog_from_ffi(&dialog);
        self.storage
            .upsert_dialog(AccountId(account_id), &compact)?;
        Ok(())
    }

    pub fn upsert_dialog_batch(
        &self,
        account_id: u64,
        dialogs: &[FfiDialogRow],
    ) -> Result<usize> {
        let batch: Vec<CompactDialog> = dialogs.iter().map(dialog_from_ffi).collect();
        let written = self
            .storage
            .upsert_dialog_batch(AccountId(account_id), &batch)?;
        Ok(written)
    }

    pub fn update_dialog_read_state(
        &self,
        account_id: u64,
        dialog_id: i64,
        read_inbox_max_id: i32,
        unread_count: u32,
    ) -> Result<bool> {
        let updated = self.storage.update_dialog_read_state(
            AccountId(account_id),
            DialogId(dialog_id),
            MsgId(read_inbox_max_id),
            unread_count,
        )?;
        Ok(updated)
    }

    pub fn remove_dialog(&self, account_id: u64, dialog_id: i64) -> Result<bool> {
        let acc = AccountId(account_id);
        let dlg = DialogId(dialog_id);
        let removed = self.storage.remove_dialog(acc, dlg)?;
        self.unload_dialog_viewport(account_id, dialog_id);
        Ok(removed)
    }

    pub fn load_dialog_slice(
        &self,
        account_id: u64,
        folder_id: i32,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<FfiDialogRow>> {
        let acc = AccountId(account_id);
        let window_cap = limit.max(1);
        let mut d_windows = self
            .dialog_windows
            .lock()
            .map_err(|_| anyhow!("dialog windows mutex poisoned"))?;

        let key = (acc, folder_id);
        let needs_recreate = match d_windows.get(&key) {
            Some(existing) => existing.capacity() != window_cap,
            None => true,
        };
        if needs_recreate {
            if d_windows.len() >= DEFAULT_MAX_RESIDENT_DIALOG_WINDOWS {
                let keys_to_evict: Vec<(AccountId, i32)> =
                    d_windows.keys().filter(|k| *k != &key).copied().collect();
                for old_key in keys_to_evict {
                    if let Some(mut old_win) = d_windows.remove(&old_key) {
                        old_win.clear_ram();
                    }
                }
            }
            let win = VirtualizedDialogWindow::new(
                Arc::clone(&self.storage),
                acc,
                folder_id,
                window_cap,
            )?;
            d_windows.insert(key, win);
        }

        let win = d_windows
            .get_mut(&key)
            .ok_or_else(|| anyhow!("failed to access dialog window"))?;
        let slice = win.load_page(offset)?;
        Ok(slice.iter().map(dialog_to_ffi).collect())
    }

    pub fn search_dialogs(
        &self,
        account_id: u64,
        query: &str,
        limit: usize,
    ) -> Result<Vec<FfiDialogRow>> {
        let rows = self
            .storage
            .search_dialogs(AccountId(account_id), query, limit)?;
        Ok(rows.iter().map(dialog_to_ffi).collect())
    }

    pub fn record_deleted_message(&self, account_id: u64, msg: FfiMessageRow) -> Result<()> {
        let compact = message_from_ffi(&msg);
        let entry = AyuDeletedMessage {
            user_id: UserId(account_id),
            topic_id: TopicId(0),
            entity_create_date: compact.date_unix,
            message: compact,
        };
        self.storage
            .record_deleted_message(AccountId(account_id), &entry)?;
        Ok(())
    }

    pub fn load_deleted_messages(
        &self,
        account_id: u64,
        dialog_id: i64,
        limit: usize,
    ) -> Result<Vec<FfiMessageRow>> {
        let rows = self.storage.load_deleted_messages(
            AccountId(account_id),
            DialogId(dialog_id),
            TopicId(0),
            limit,
        )?;
        Ok(rows
            .iter()
            .map(|entry| message_to_ffi(&entry.message))
            .collect())
    }

    pub fn has_deleted_messages(&self, account_id: u64, dialog_id: i64) -> bool {
        self.storage
            .has_deleted_messages(AccountId(account_id), DialogId(dialog_id), TopicId(0))
            .unwrap_or(false)
    }

    pub fn record_edited_message(&self, account_id: u64, msg: FfiMessageRow) -> Result<()> {
        let compact = message_from_ffi(&msg);
        let entry = AyuEditedRevision {
            user_id: UserId(account_id),
            entity_create_date: compact.edit_date_unix,
            message: compact,
        };
        self.storage
            .record_edited_revision(AccountId(account_id), &entry)?;
        Ok(())
    }

    pub fn load_edited_messages(
        &self,
        account_id: u64,
        dialog_id: i64,
        msg_id: i32,
        limit: usize,
    ) -> Result<Vec<FfiMessageRow>> {
        let rows = self.storage.load_edited_revisions(
            UserId(account_id),
            DialogId(dialog_id),
            MsgId(msg_id),
            limit,
        )?;
        Ok(rows
            .iter()
            .map(|entry| message_to_ffi(&entry.message))
            .collect())
    }

    pub fn has_edited_revisions(
        &self,
        account_id: u64,
        dialog_id: i64,
        msg_id: i32,
    ) -> bool {
        self.storage
            .has_edited_revisions(UserId(account_id), DialogId(dialog_id), MsgId(msg_id))
            .unwrap_or(false)
    }

    pub fn put_media_cache(&self, key: u64, data: &[u8]) -> Result<()> {
        self.media_cache.put_bytes(key, data)?;
        Ok(())
    }

    pub fn put_scoped_media_cache(
        &self,
        account_id: u64,
        kind: u8,
        resource_id: u64,
        data: &[u8],
    ) -> Result<()> {
        self.media_cache.put_scoped(
            AccountId(account_id),
            MediaCacheKind::from_u8(kind),
            resource_id,
            data,
        )?;
        Ok(())
    }

    pub fn get_scoped_media_cache(
        &self,
        account_id: u64,
        kind: u8,
        resource_id: u64,
    ) -> Vec<u8> {
        self.media_cache
            .get_scoped(
                AccountId(account_id),
                MediaCacheKind::from_u8(kind),
                resource_id,
            )
            .unwrap_or_default()
    }

    pub fn evict_account_media_cache(&self, account_id: u64, heavy_only: bool) -> usize {
        self.media_cache
            .evict_account(AccountId(account_id), heavy_only)
    }

    pub fn evict_inactive_caches(&self) -> usize {
        let active_acc = self
            .orchestrator
            .lock()
            .ok()
            .and_then(|o| o.active_account());

        let mut freed_count = 0usize;
        if let Ok(mut windows) = self.windows.lock() {
            windows.retain(|(acc, _), win| {
                let keep = Some(*acc) == active_acc;
                if !keep {
                    freed_count = freed_count.saturating_add(win.len());
                    win.clear_ram();
                }
                keep
            });
        }
        if let Ok(mut d_windows) = self.dialog_windows.lock() {
            d_windows.retain(|(acc, _), win| {
                let keep = Some(*acc) == active_acc;
                if !keep {
                    freed_count = freed_count.saturating_add(win.len());
                    win.clear_ram();
                }
                keep
            });
        }
        let target = self.media_cache.max_bytes() / 2;
        let freed_bytes = self.media_cache.shrink_for_background_account(target);
        freed_count.saturating_add(freed_bytes)
    }

    pub fn get_memory_stats(&self) -> FfiMemoryStats {
        let active_accounts = self
            .orchestrator
            .lock()
            .map(|o| o.account_count())
            .unwrap_or(0);
        let (resident_windows, cached_window_messages, cached_window_bytes) = self
            .windows
            .lock()
            .map(|w| {
                let msgs: usize = w.values().map(VirtualizedHistoryWindow::len).sum();
                let bytes: usize = w
                    .values()
                    .map(VirtualizedHistoryWindow::estimated_ram_bytes)
                    .sum();
                (w.len(), msgs, bytes)
            })
            .unwrap_or((0, 0, 0));
        let (resident_dialog_windows, cached_dialog_rows) = self
            .dialog_windows
            .lock()
            .map(|w| {
                let rows: usize = w.values().map(VirtualizedDialogWindow::len).sum();
                (w.len(), rows)
            })
            .unwrap_or((0, 0));

        FfiMemoryStats {
            active_accounts,
            resident_windows,
            cached_window_messages,
            cached_window_bytes,
            resident_dialog_windows,
            cached_dialog_rows,
            media_cache_entries: self.media_cache.entry_count(),
            media_cache_bytes: self.media_cache.current_bytes(),
            media_cache_limit_bytes: self.media_cache.max_bytes(),
            process_rss_bytes: current_process_rss_bytes(),
        }
    }

    pub fn init_sync_state(
        &self,
        account_id: u64,
        pts: i32,
        qts: i32,
        date: i32,
        seq: i32,
    ) -> Result<()> {
        let acc = AccountId(account_id);
        let snapshot = SyncStateSnapshot {
            pts,
            qts,
            date,
            seq,
        };
        self.sync.init_state(acc, snapshot)?;
        let merged = self.sync.state_snapshot(acc)?;
        self.storage.save_sync_state(acc, merged)?;
        Ok(())
    }

    pub fn get_sync_state(&self, account_id: u64) -> Result<FfiSyncState> {
        let acc = AccountId(account_id);
        let mut snapshot = self.sync.state_snapshot(acc)?;
        if snapshot.pts == 0 && snapshot.qts == 0 && snapshot.date == 0 && snapshot.seq == 0 {
            if let Some(persisted) = self.storage.load_sync_state(acc)? {
                self.sync.init_state(acc, persisted)?;
                snapshot = self.sync.state_snapshot(acc)?;
            }
        }
        Ok(FfiSyncState {
            pts: snapshot.pts,
            qts: snapshot.qts,
            date: snapshot.date,
            seq: snapshot.seq,
        })
    }

    pub fn apply_pts_update(
        &self,
        account_id: u64,
        channel_id: u64,
        pts: i32,
        pts_count: i32,
    ) -> Result<FfiPtsOutcome> {
        let acc = AccountId(account_id);
        if channel_id == 0 {
            let current = self.sync.state_snapshot(acc)?;
            if current.pts == 0 {
                if let Some(persisted) = self.storage.load_sync_state(acc)? {
                    self.sync.init_state(acc, persisted)?;
                }
            }
        } else if self.sync.channel_pts(acc, channel_id)? == 0 {
            if let Some(persisted) = self.storage.load_channel_pts(acc, channel_id)? {
                self.sync.init_channel_pts(acc, channel_id, persisted)?;
            }
        }

        let res = self.sync.apply_pts(acc, channel_id, pts, pts_count)?;
        match res {
            PtsCheckResult::Uninited => Ok(FfiPtsOutcome {
                applied: false,
                gap_detected: false,
                duplicate: false,
                current_pts: 0,
            }),
            PtsCheckResult::Applied { new_pts } => {
                if channel_id == 0 {
                    let snapshot = self.sync.state_snapshot(acc)?;
                    self.storage.save_sync_state(acc, snapshot)?;
                } else {
                    self.storage.save_channel_pts(acc, channel_id, new_pts)?;
                }
                Ok(FfiPtsOutcome {
                    applied: true,
                    gap_detected: false,
                    duplicate: false,
                    current_pts: new_pts,
                })
            }
            PtsCheckResult::DuplicateIgnored { current_pts } => Ok(FfiPtsOutcome {
                applied: false,
                gap_detected: false,
                duplicate: true,
                current_pts,
            }),
            PtsCheckResult::GapDetected { expected_pts, .. } => {
                let current_pts = if channel_id == 0 {
                    self.sync.state_snapshot(acc)?.pts
                } else {
                    self.sync.channel_pts(acc, channel_id)?
                };
                let _ = expected_pts;
                Ok(FfiPtsOutcome {
                    applied: false,
                    gap_detected: true,
                    duplicate: false,
                    current_pts,
                })
            }
        }
    }

    pub fn queue_outgoing_message(
        &self,
        account_id: u64,
        random_id: u64,
        msg: FfiMessageRow,
    ) -> Result<i32> {
        let acc = AccountId(account_id);
        let mut compact = message_from_ffi(&msg);
        if compact.id.0 == 0 {
            let next_raw = self.next_temp_msg_id.fetch_sub(1, Ordering::Relaxed);
            compact.id = MsgId(if next_raw < 0 { next_raw } else { -1 });
        }
        let local_id = compact.id;
        self.storage.upsert_message(acc, &compact)?;
        self.sync
            .queue_outgoing(acc, random_id, compact.dialog_id, local_id, compact.date_unix)?;
        if let Ok(mut windows) = self.windows.lock() {
            if let Some(win) = windows.get_mut(&(acc, compact.dialog_id)) {
                win.push_live_message(compact);
            }
        }
        Ok(local_id.0)
    }

    pub fn confirm_outgoing_message(
        &self,
        account_id: u64,
        random_id: u64,
        server_msg_id: i32,
        date_unix: i32,
        pts: i32,
        pts_count: i32,
    ) -> Result<bool> {
        let acc = AccountId(account_id);
        let Some(pending) = self.sync.confirm_outgoing(acc, random_id)? else {
            return Ok(false);
        };
        let loaded = self
            .storage
            .load_slice_around(acc, pending.dialog_id, pending.local_msg_id, 1)?;
        let mut confirmed = loaded
            .into_iter()
            .find(|m| m.id == pending.local_msg_id)
            .unwrap_or_else(|| CompactMessage {
                dialog_id: pending.dialog_id,
                sender_id: PeerId::from_user(UserId(account_id)),
                grouped_id: 0,
                text: Box::from(""),
                author_name: None,
                media: None,
                entities: SmallVec::new(),
                id: MsgId(server_msg_id),
                reply_to_msg_id: MsgId(0),
                date_unix,
                edit_date_unix: 0,
                views: 0,
                flags: MessageFlags::OUTGOING,
            });
        if pending.local_msg_id.0 != server_msg_id {
            let _ = self
                .storage
                .delete_messages(acc, pending.dialog_id, &[pending.local_msg_id])?;
        }
        confirmed.id = MsgId(server_msg_id);
        if date_unix > 0 {
            confirmed.date_unix = date_unix;
        }
        self.storage.upsert_message(acc, &confirmed)?;
        if let Ok(mut windows) = self.windows.lock() {
            if let Some(win) = windows.get_mut(&(acc, pending.dialog_id)) {
                let _ = win.remove_messages_in_ram(&[pending.local_msg_id]);
                win.push_live_message(confirmed);
            }
        }
        if pts > 0 {
            let _ = self.apply_pts_update(account_id, 0, pts, pts_count)?;
        }
        Ok(true)
    }

    pub fn receive_incoming_message(
        &self,
        account_id: u64,
        channel_id: u64,
        pts: i32,
        pts_count: i32,
        msg: FfiMessageRow,
    ) -> Result<FfiPtsOutcome> {
        let outcome = self.apply_pts_update(account_id, channel_id, pts, pts_count)?;
        if !outcome.gap_detected && !outcome.duplicate {
            let acc = AccountId(account_id);
            let compact = message_from_ffi(&msg);
            self.storage.upsert_message(acc, &compact)?;
            if let Ok(mut windows) = self.windows.lock() {
                if let Some(win) = windows.get_mut(&(acc, compact.dialog_id)) {
                    win.push_live_message(compact);
                }
            }
        }
        Ok(outcome)
    }

    pub fn enqueue_file_download(
        &self,
        account_id: u64,
        dc_id: u16,
        file_id: u64,
        total_bytes: u64,
        priority: i32,
    ) -> Result<u64> {
        Ok(self
            .transfers
            .enqueue_download(AccountId(account_id), dc_id, file_id, total_bytes, priority)?)
    }

    pub fn enqueue_file_upload(
        &self,
        account_id: u64,
        dc_id: u16,
        file_id: u64,
        total_bytes: u64,
    ) -> Result<u64> {
        Ok(self
            .transfers
            .enqueue_upload(AccountId(account_id), dc_id, file_id, total_bytes)?)
    }

    pub fn advance_file_transfer(
        &self,
        task_id: u64,
        chunk_bytes: usize,
    ) -> Result<FfiTransferProgress> {
        let progress = self.transfers.advance_transfer(task_id, chunk_bytes)?;
        Ok(transfer_to_ffi(progress))
    }

    pub fn cancel_file_transfer(&self, task_id: u64) -> bool {
        self.transfers.cancel_transfer(task_id).unwrap_or(false)
    }

    pub fn active_transfer_inflight_bytes(&self) -> usize {
        self.transfers.active_inflight_bytes()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn query_deleted_messages(
        &self,
        account_id: u64,
        dialog_id: i64,
        topic_id: i64,
        min_id: i32,
        max_id: i32,
        limit: usize,
        search_query: &str,
    ) -> Result<Vec<FfiMessageRow>> {
        let rows = self.storage.query_deleted_messages(
            AccountId(account_id),
            DialogId(dialog_id),
            TopicId(topic_id),
            MsgId(min_id),
            MsgId(max_id),
            limit,
            search_query,
        )?;
        Ok(rows
            .iter()
            .map(|entry| message_to_ffi(&entry.message))
            .collect())
    }

    pub fn remove_deleted_message(
        &self,
        account_id: u64,
        dialog_id: i64,
        msg_id: i32,
    ) -> Result<usize> {
        Ok(self.storage.remove_deleted_message(
            AccountId(account_id),
            DialogId(dialog_id),
            MsgId(msg_id),
        )?)
    }

    pub fn clear_deleted_messages(
        &self,
        account_id: u64,
        dialog_id: i64,
        topic_id: i64,
    ) -> Result<usize> {
        Ok(self.storage.clear_deleted_messages(
            AccountId(account_id),
            DialogId(dialog_id),
            TopicId(topic_id),
        )?)
    }

    pub fn query_edited_messages(
        &self,
        account_id: u64,
        dialog_id: i64,
        msg_id: i32,
        min_id: i64,
        max_id: i64,
        limit: usize,
    ) -> Result<Vec<FfiMessageRow>> {
        let rows = self.storage.query_edited_revisions(
            UserId(account_id),
            DialogId(dialog_id),
            MsgId(msg_id),
            min_id,
            max_id,
            limit,
        )?;
        Ok(rows
            .iter()
            .map(|entry| message_to_ffi(&entry.message))
            .collect())
    }

    pub fn upsert_regex_filter(&self, filter: FfiRegexFilter) -> Result<()> {
        let rule = RegexFilterRule {
            id: filter.id.into_boxed_slice(),
            pattern: filter.pattern.into_boxed_str(),
            dialog_id: filter.has_dialog_id.then_some(DialogId(filter.dialog_id)),
            enabled: filter.enabled,
            reversed: filter.reversed,
            case_insensitive: filter.case_insensitive,
        };
        self.storage.save_regex_filter(&rule)?;
        Ok(())
    }

    pub fn delete_regex_filter(&self, id: &[u8]) -> Result<bool> {
        Ok(self.storage.delete_regex_filter(id)?)
    }

    pub fn clear_regex_filters(&self) -> Result<usize> {
        Ok(self.storage.delete_all_regex_filters()?)
    }

    pub fn list_regex_filters(&self) -> Result<Vec<FfiRegexFilter>> {
        let rules = self.storage.load_regex_filters()?;
        Ok(rules
            .into_iter()
            .map(|rule| FfiRegexFilter {
                id: rule.id.into_vec(),
                pattern: rule.pattern.into_string(),
                enabled: rule.enabled,
                reversed: rule.reversed,
                case_insensitive: rule.case_insensitive,
                has_dialog_id: rule.dialog_id.is_some(),
                dialog_id: rule.dialog_id.map_or(0, |d| d.0),
            })
            .collect())
    }

    pub fn add_regex_exclusion(&self, dialog_id: i64, filter_id: &[u8]) -> Result<()> {
        let exclusion = RegexFilterGlobalExclusion {
            dialog_id: DialogId(dialog_id),
            filter_id: filter_id.to_vec().into_boxed_slice(),
        };
        self.storage.save_regex_exclusion(&exclusion)?;
        Ok(())
    }

    pub fn remove_regex_exclusion(&self, dialog_id: i64, filter_id: &[u8]) -> Result<bool> {
        Ok(self
            .storage
            .delete_regex_exclusion(DialogId(dialog_id), filter_id)?)
    }

    pub fn delete_exclusions_by_filter_id(&self, filter_id: &[u8]) -> Result<usize> {
        Ok(self.storage.delete_exclusions_by_filter_id(filter_id)?)
    }

    pub fn clear_regex_exclusions(&self) -> Result<usize> {
        Ok(self.storage.delete_all_regex_exclusions()?)
    }

    pub fn rebuild_filters_cache(&self) -> Result<usize> {
        let rules = self.storage.load_regex_filters()?;
        let exclusions = self.storage.load_regex_exclusions()?;
        let mut guard = self
            .filters
            .lock()
            .map_err(|_| anyhow!("filters mutex poisoned"))?;
        Ok(guard.rebuild(&rules, &exclusions))
    }

    pub fn set_filters_enabled(
        &self,
        enabled: bool,
        enabled_in_chats: bool,
        hide_from_blocked: bool,
    ) {
        if let Ok(mut guard) = self.filters.lock() {
            guard.set_filters_enabled(enabled, enabled_in_chats, hide_from_blocked);
        }
    }

    pub fn set_shadow_banned(&self, peer_dialog_id: i64, banned: bool) {
        if let Ok(mut guard) = self.filters.lock() {
            guard.set_shadow_banned(peer_dialog_id, banned);
        }
    }

    pub fn is_shadow_banned(&self, peer_dialog_id: i64) -> bool {
        self.filters
            .lock()
            .map(|guard| guard.is_shadow_banned(peer_dialog_id))
            .unwrap_or(false)
    }

    pub fn evaluate_message_filter(
        &self,
        dialog_id: i64,
        msg_id: i32,
        sender_dialog_id: i64,
        is_outgoing: bool,
        text: &str,
    ) -> bool {
        self.filters
            .lock()
            .map(|mut guard| {
                guard.evaluate_message(
                    DialogId(dialog_id),
                    MsgId(msg_id),
                    sender_dialog_id,
                    is_outgoing,
                    text,
                )
            })
            .unwrap_or(false)
    }

    pub fn invalidate_filtered_message(&self, dialog_id: i64, msg_id: i32) {
        if let Ok(mut guard) = self.filters.lock() {
            guard.invalidate_message(DialogId(dialog_id), MsgId(msg_id));
        }
    }

    pub fn has_filtered_messages(&self, dialog_id: i64) -> bool {
        self.filters
            .lock()
            .map(|guard| guard.has_filtered_messages(DialogId(dialog_id)))
            .unwrap_or(false)
    }

    pub fn toggle_filtered_messages_shown(&self, dialog_id: i64) -> bool {
        self.filters
            .lock()
            .map(|mut guard| guard.toggle_filtered_shown(DialogId(dialog_id)))
            .unwrap_or(false)
    }

    pub fn hide_message(&self, dialog_id: i64, msg_id: i32) {
        if let Ok(mut guard) = self.filters.lock() {
            guard.hide_message(DialogId(dialog_id), MsgId(msg_id));
        }
    }

    pub fn is_message_hidden(&self, dialog_id: i64, msg_id: i32) -> bool {
        self.filters
            .lock()
            .map(|guard| guard.is_message_hidden(DialogId(dialog_id), MsgId(msg_id)))
            .unwrap_or(false)
    }

    pub fn arm_disable_ghost_on_story_close(&self, account_id: u64) {
        if let Ok(mut guard) = self.filters.lock() {
            guard.arm_disable_ghost_on_story_close(AccountId(account_id));
        }
    }

    pub fn consume_disable_ghost_on_story_close(&self, account_id: u64) -> bool {
        self.filters
            .lock()
            .map(|mut guard| guard.consume_disable_ghost_on_story_close(AccountId(account_id)))
            .unwrap_or(false)
    }
}

fn transfer_to_ffi(p: TransferProgress) -> FfiTransferProgress {
    FfiTransferProgress {
        task_id: p.task_id,
        file_id: p.file_id,
        transferred_bytes: p.transferred_bytes,
        total_bytes: p.total_bytes,
        part_size: p.part_size,
        parts_total: p.parts_total,
        parts_completed: p.parts_completed,
        state: p.state as u8,
        is_big_file: p.is_big_file,
    }
}

pub fn create_ayu_engine(db_path: &str, media_cache_limit_mb: usize) -> Result<Box<AyuRustEngine>> {
    Ok(Box::new(AyuRustEngine::new(db_path, media_cache_limit_mb)?))
}

#[cfg(target_os = "linux")]
fn current_process_rss_bytes() -> usize {
    let Ok(statm) = std::fs::read_to_string("/proc/self/statm") else {
        return 0;
    };
    let Some(resident_pages_str) = statm.split_whitespace().nth(1) else {
        return 0;
    };
    let Ok(resident_pages) = resident_pages_str.parse::<usize>() else {
        return 0;
    };
    resident_pages.saturating_mul(4096)
}

#[cfg(not(target_os = "linux"))]
fn current_process_rss_bytes() -> usize {
    0
}

fn snapshot_from_controller(ctrl: &AccountController) -> FfiAccountSnapshot {
    FfiAccountSnapshot {
        account_id: ctrl.account_id().0,
        user_id: ctrl.user_id().0,
        session_unique_id: ctrl.session_unique_id(),
        main_dc_id: ctrl.main_dc_id(),
        auth_state: ctrl.auth_state() as u8,
        lifecycle_state: ctrl.state() as u8,
        is_test_dc: ctrl.is_test_dc(),
        is_premium: ctrl.is_premium(),
        unread_count: ctrl.unread_count(),
        unread_muted: ctrl.unread_muted(),
    }
}

fn ghost_from_ffi(cfg: FfiGhostConfig) -> GhostModeConfig {
    let mut flags = GhostModeFlags::empty();
    if cfg.ghost_mode_active {
        flags |= GhostModeFlags::GHOST_MODE_ACTIVE;
    }
    if cfg.send_read_messages {
        flags |= GhostModeFlags::SEND_READ_MESSAGES;
    }
    if cfg.send_read_stories {
        flags |= GhostModeFlags::SEND_READ_STORIES;
    }
    if cfg.send_online_packets {
        flags |= GhostModeFlags::SEND_ONLINE_PACKETS;
    }
    if cfg.send_upload_progress {
        flags |= GhostModeFlags::SEND_UPLOAD_PROGRESS;
    }
    if cfg.send_offline_after_online {
        flags |= GhostModeFlags::SEND_OFFLINE_AFTER_ONLINE;
    }
    if cfg.mark_read_after_action {
        flags |= GhostModeFlags::MARK_READ_AFTER_ACTION;
    }
    if cfg.use_scheduled_messages {
        flags |= GhostModeFlags::USE_SCHEDULED_MESSAGES;
    }
    GhostModeConfig::new(flags, SendWithoutSoundOption::from_u8(cfg.send_without_sound))
}

fn ghost_to_ffi(cfg: GhostModeConfig) -> FfiGhostConfig {
    let flags = cfg.flags();
    FfiGhostConfig {
        ghost_mode_active: flags.contains(GhostModeFlags::GHOST_MODE_ACTIVE),
        send_read_messages: flags.contains(GhostModeFlags::SEND_READ_MESSAGES),
        send_read_stories: flags.contains(GhostModeFlags::SEND_READ_STORIES),
        send_online_packets: flags.contains(GhostModeFlags::SEND_ONLINE_PACKETS),
        send_upload_progress: flags.contains(GhostModeFlags::SEND_UPLOAD_PROGRESS),
        send_offline_after_online: flags.contains(GhostModeFlags::SEND_OFFLINE_AFTER_ONLINE),
        mark_read_after_action: flags.contains(GhostModeFlags::MARK_READ_AFTER_ACTION),
        use_scheduled_messages: flags.contains(GhostModeFlags::USE_SCHEDULED_MESSAGES),
        send_without_sound: cfg.send_without_sound() as u8,
    }
}

fn message_from_ffi(row: &FfiMessageRow) -> CompactMessage {
    let media = if row.media_kind > 0 || !row.media_path.is_empty() {
        Some(Box::new(MediaDescriptor {
            file_id: 0,
            access_hash: 0,
            size_bytes: 0,
            mime_type: Box::from(""),
            local_path: if row.media_path.is_empty() {
                None
            } else {
                Some(row.media_path.as_str().into())
            },
            thumb_path: None,
            width: 0,
            height: 0,
            duration_secs: 0,
            kind: MediaKind::from_u8(row.media_kind),
            has_spoiler: false,
        }))
    } else {
        None
    };

    CompactMessage {
        dialog_id: DialogId(row.dialog_id),
        sender_id: PeerId(row.sender_id),
        grouped_id: row.grouped_id,
        text: row.text.as_str().into(),
        author_name: if row.author_name.is_empty() {
            None
        } else {
            Some(Arc::from(row.author_name.as_str()))
        },
        media,
        entities: SmallVec::new(),
        id: MsgId(row.msg_id),
        reply_to_msg_id: MsgId(row.reply_to_msg_id),
        date_unix: row.date_unix,
        edit_date_unix: row.edit_date_unix,
        views: row.views,
        flags: MessageFlags::from_bits_truncate(row.flags),
    }
}

fn dialog_from_ffi(row: &FfiDialogRow) -> CompactDialog {
    CompactDialog {
        dialog_id: DialogId(row.dialog_id),
        peer_id: PeerId(row.peer_id),
        title: row.title.as_str().into(),
        draft_text: if row.draft_text.is_empty() {
            None
        } else {
            Some(row.draft_text.as_str().into())
        },
        top_message_id: MsgId(row.top_message_id),
        read_inbox_max_id: MsgId(row.read_inbox_max_id),
        read_outbox_max_id: MsgId(row.read_outbox_max_id),
        last_message_date: row.last_message_date,
        unread_count: row.unread_count,
        unread_mentions_count: row.unread_mentions_count,
        folder_id: row.folder_id,
        flags: DialogFlags::from_bits_truncate(row.flags),
    }
}

fn dialog_to_ffi(d: &CompactDialog) -> FfiDialogRow {
    FfiDialogRow {
        dialog_id: d.dialog_id.0,
        peer_id: d.peer_id.0,
        folder_id: d.folder_id,
        title: d.title.as_ref().to_owned(),
        draft_text: d.draft_text.as_deref().unwrap_or_default().to_owned(),
        top_message_id: d.top_message_id.0,
        read_inbox_max_id: d.read_inbox_max_id.0,
        read_outbox_max_id: d.read_outbox_max_id.0,
        last_message_date: d.last_message_date,
        unread_count: d.unread_count,
        unread_mentions_count: d.unread_mentions_count,
        flags: d.flags.bits(),
    }
}

fn message_to_ffi(msg: &CompactMessage) -> FfiMessageRow {
    let (media_kind, media_path) = match msg.media.as_deref() {
        Some(m) => (
            m.kind as u8,
            m.local_path.as_deref().unwrap_or_default().to_owned(),
        ),
        None => (0u8, String::new()),
    };
    FfiMessageRow {
        dialog_id: msg.dialog_id.0,
        msg_id: msg.id.0,
        sender_id: msg.sender_id.0,
        reply_to_msg_id: msg.reply_to_msg_id.0,
        date_unix: msg.date_unix,
        edit_date_unix: msg.edit_date_unix,
        views: msg.views,
        flags: msg.flags.bits(),
        grouped_id: msg.grouped_id,
        author_name: msg.author_name.as_deref().unwrap_or_default().to_owned(),
        text: msg.text.as_ref().to_owned(),
        media_kind,
        media_path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ffi_engine_smoke_test() -> Result<()> {
        let engine = create_ayu_engine(":memory:", 4)?;
        assert!(engine.health_check().ok);

        engine.register_account(1, 10001, false)?;
        engine.register_account(2, 10002, false)?;

        engine.set_ghost_config(
            1,
            FfiGhostConfig {
                ghost_mode_active: true,
                send_read_messages: true,
                send_read_stories: true,
                send_online_packets: true,
                send_upload_progress: true,
                send_offline_after_online: true,
                mark_read_after_action: false,
                use_scheduled_messages: false,
                send_without_sound: 1,
            },
        )?;
        assert!(!engine.should_send_read_ack(1));
        assert!(!engine.should_send_online_packet(1));
        assert!(engine.should_send_read_ack(2));

        let loaded_cfg = engine.get_ghost_config(1)?;
        assert!(loaded_cfg.ghost_mode_active);
        assert_eq!(loaded_cfg.send_without_sound, 1);

        engine.upsert_dialog(
            1,
            FfiDialogRow {
                dialog_id: 50,
                peer_id: 50,
                folder_id: 0,
                title: "Work".to_owned(),
                draft_text: String::new(),
                top_message_id: 1,
                read_inbox_max_id: 0,
                read_outbox_max_id: 0,
                last_message_date: 1_700_000_000,
                unread_count: 1,
                unread_mentions_count: 0,
                flags: 0,
            },
        )?;
        assert_eq!(engine.load_dialog_slice(1, 0, 0, 10)?.len(), 1);

        engine.upsert_message(
            1,
            FfiMessageRow {
                dialog_id: 50,
                msg_id: 1,
                sender_id: 10001,
                reply_to_msg_id: 0,
                date_unix: 1_700_000_000,
                edit_date_unix: 0,
                views: 0,
                flags: 1,
                grouped_id: 0,
                author_name: "Alice".to_owned(),
                text: "Hello FFI".to_owned(),
                media_kind: 0,
                media_path: String::new(),
            },
        )?;
        assert_eq!(engine.load_viewport_messages(1, 50, 1, 50)?.len(), 1);

        engine.record_deleted_message(
            1,
            FfiMessageRow {
                dialog_id: 50,
                msg_id: 1,
                sender_id: 10001,
                reply_to_msg_id: 0,
                date_unix: 1_700_000_000,
                edit_date_unix: 0,
                views: 0,
                flags: 1,
                grouped_id: 0,
                author_name: "Alice".to_owned(),
                text: "Deleted msg".to_owned(),
                media_kind: 0,
                media_path: String::new(),
            },
        )?;
        assert_eq!(engine.load_deleted_messages(1, 50, 10)?.len(), 1);

        engine.remove_account(2)?;
        assert_eq!(engine.get_memory_stats().active_accounts, 1);
        Ok(())
    }

    #[test]
    fn ffi_auth_and_multi_account_lifecycle_test() -> Result<()> {
        let engine = create_ayu_engine(":memory:", 4)?;
        engine.register_account(0, 0, false)?;
        engine.register_account(1, 0, true)?;

        let mut raw_blob = Vec::new();
        raw_blob.extend_from_slice(&u64::MAX.to_be_bytes());
        raw_blob.extend_from_slice(&555_001u64.to_be_bytes());
        raw_blob.extend_from_slice(&2i32.to_be_bytes());
        raw_blob.extend_from_slice(&1i32.to_be_bytes());
        raw_blob.extend_from_slice(&2i32.to_be_bytes());
        raw_blob.extend_from_slice(&[0x33u8; 256]);
        raw_blob.extend_from_slice(&0i32.to_be_bytes());

        let snap = engine.import_mtp_authorization(0, &raw_blob)?;
        assert_eq!(snap.user_id, 555_001);
        assert_eq!(snap.main_dc_id, 2);

        engine.complete_authorization(1, 555_002, 4, true)?;
        engine.set_account_unread_state(0, 4, false)?;
        engine.set_account_unread_state(1, 2, true)?;
        let badge = engine.compute_unread_badge();
        assert_eq!(badge.total_unread, 6);
        assert!(!badge.all_muted);

        engine.push_login_code(0, "12345")?;
        assert_eq!(engine.take_login_code(0)?, "12345");
        assert_eq!(engine.take_login_code(0)?, "");

        let exported = engine.export_mtp_authorization(0)?;
        assert!(!exported.is_empty());

        engine.force_logout(0)?;
        assert_eq!(engine.authed_accounts_count(), 1);
        assert_eq!(engine.redundant_account_ids(), vec![0]);
        Ok(())
    }
}
