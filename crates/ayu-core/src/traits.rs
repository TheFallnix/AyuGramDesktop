use crate::auth::{AuthState, MtpAuthorizationBlob};
use crate::error::CoreError;
use crate::ids::{AccountId, DialogId, MsgId, TopicId, UserId};
use crate::models::{
    AyuDeletedMessage, AyuEditedRevision, CompactDialog, CompactMessage,
    RegexFilterGlobalExclusion, RegexFilterRule,
};

/// Repository abstraction for reading and writing paginated chat history slices on disk.
pub trait MessageRepository: Send + Sync {
    fn upsert_message(&self, account_id: AccountId, message: &CompactMessage)
        -> Result<(), CoreError>;

    fn upsert_batch(
        &self,
        account_id: AccountId,
        messages: &[CompactMessage],
    ) -> Result<usize, CoreError>;

    fn load_slice_around(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        center_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError>;

    fn load_older_than(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        before_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError>;

    fn load_newer_than(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        after_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError>;

    fn delete_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        msg_ids: &[MsgId],
    ) -> Result<usize, CoreError>;

    fn delete_messages_up_to(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        max_msg_id: MsgId,
    ) -> Result<usize, CoreError>;

    fn search_dialog_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError>;

    fn dialog_message_count(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
    ) -> Result<usize, CoreError>;
}

impl<T: MessageRepository + ?Sized> MessageRepository for std::sync::Arc<T> {
    fn upsert_message(
        &self,
        account_id: AccountId,
        message: &CompactMessage,
    ) -> Result<(), CoreError> {
        self.as_ref().upsert_message(account_id, message)
    }

    fn upsert_batch(
        &self,
        account_id: AccountId,
        messages: &[CompactMessage],
    ) -> Result<usize, CoreError> {
        self.as_ref().upsert_batch(account_id, messages)
    }

    fn load_slice_around(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        center_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        self.as_ref()
            .load_slice_around(account_id, dialog_id, center_msg_id, limit)
    }

    fn load_older_than(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        before_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        self.as_ref()
            .load_older_than(account_id, dialog_id, before_msg_id, limit)
    }

    fn load_newer_than(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        after_msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        self.as_ref()
            .load_newer_than(account_id, dialog_id, after_msg_id, limit)
    }

    fn delete_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        msg_ids: &[MsgId],
    ) -> Result<usize, CoreError> {
        self.as_ref()
            .delete_messages(account_id, dialog_id, msg_ids)
    }

    fn delete_messages_up_to(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        max_msg_id: MsgId,
    ) -> Result<usize, CoreError> {
        self.as_ref()
            .delete_messages_up_to(account_id, dialog_id, max_msg_id)
    }

    fn search_dialog_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CompactMessage>, CoreError> {
        self.as_ref()
            .search_dialog_messages(account_id, dialog_id, query, limit)
    }

    fn dialog_message_count(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
    ) -> Result<usize, CoreError> {
        self.as_ref().dialog_message_count(account_id, dialog_id)
    }
}

/// Repository abstraction for virtualized dialog list rows.
pub trait DialogRepository: Send + Sync {
    fn upsert_dialog(&self, account_id: AccountId, dialog: &CompactDialog)
        -> Result<(), CoreError>;

    fn upsert_dialog_batch(
        &self,
        account_id: AccountId,
        dialogs: &[CompactDialog],
    ) -> Result<usize, CoreError>;

    fn update_dialog_read_state(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        read_inbox_max_id: MsgId,
        unread_count: u32,
    ) -> Result<bool, CoreError>;

    fn remove_dialog(&self, account_id: AccountId, dialog_id: DialogId)
        -> Result<bool, CoreError>;

    fn load_dialog_slice(
        &self,
        account_id: AccountId,
        folder_id: i32,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<CompactDialog>, CoreError>;

    fn search_dialogs(
        &self,
        account_id: AccountId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CompactDialog>, CoreError>;

    fn dialog_count(&self, account_id: AccountId, folder_id: i32) -> Result<usize, CoreError>;
}

impl<T: DialogRepository + ?Sized> DialogRepository for std::sync::Arc<T> {
    fn upsert_dialog(
        &self,
        account_id: AccountId,
        dialog: &CompactDialog,
    ) -> Result<(), CoreError> {
        self.as_ref().upsert_dialog(account_id, dialog)
    }

    fn upsert_dialog_batch(
        &self,
        account_id: AccountId,
        dialogs: &[CompactDialog],
    ) -> Result<usize, CoreError> {
        self.as_ref().upsert_dialog_batch(account_id, dialogs)
    }

    fn update_dialog_read_state(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        read_inbox_max_id: MsgId,
        unread_count: u32,
    ) -> Result<bool, CoreError> {
        self.as_ref()
            .update_dialog_read_state(account_id, dialog_id, read_inbox_max_id, unread_count)
    }

    fn remove_dialog(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
    ) -> Result<bool, CoreError> {
        self.as_ref().remove_dialog(account_id, dialog_id)
    }

    fn load_dialog_slice(
        &self,
        account_id: AccountId,
        folder_id: i32,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<CompactDialog>, CoreError> {
        self.as_ref()
            .load_dialog_slice(account_id, folder_id, offset, limit)
    }

    fn search_dialogs(
        &self,
        account_id: AccountId,
        query: &str,
        limit: usize,
    ) -> Result<Vec<CompactDialog>, CoreError> {
        self.as_ref().search_dialogs(account_id, query, limit)
    }

    fn dialog_count(&self, account_id: AccountId, folder_id: i32) -> Result<usize, CoreError> {
        self.as_ref().dialog_count(account_id, folder_id)
    }
}

/// Repository abstraction for AyuGram audit history (deleted/edited messages & filters).
pub trait AyuAuditRepository: Send + Sync {
    fn record_deleted_message(
        &self,
        account_id: AccountId,
        entry: &AyuDeletedMessage,
    ) -> Result<(), CoreError>;

    fn load_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
        limit: usize,
    ) -> Result<Vec<AyuDeletedMessage>, CoreError>;

    fn has_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
    ) -> Result<bool, CoreError>;

    fn record_edited_revision(
        &self,
        account_id: AccountId,
        entry: &AyuEditedRevision,
    ) -> Result<(), CoreError>;

    fn load_edited_revisions(
        &self,
        user_id: UserId,
        dialog_id: DialogId,
        msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<AyuEditedRevision>, CoreError>;

    fn has_edited_revisions(
        &self,
        user_id: UserId,
        dialog_id: DialogId,
        msg_id: MsgId,
    ) -> Result<bool, CoreError>;

    fn save_regex_filter(&self, rule: &RegexFilterRule) -> Result<(), CoreError>;

    fn load_regex_filters(&self) -> Result<Vec<RegexFilterRule>, CoreError>;

    #[allow(clippy::too_many_arguments)]
    fn query_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
        min_msg_id: MsgId,
        max_msg_id: MsgId,
        limit: usize,
        search_query: &str,
    ) -> Result<Vec<AyuDeletedMessage>, CoreError>;

    fn remove_deleted_message(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        msg_id: MsgId,
    ) -> Result<usize, CoreError>;

    fn clear_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
    ) -> Result<usize, CoreError>;

    fn query_edited_revisions(
        &self,
        user_id: UserId,
        dialog_id: DialogId,
        msg_id: MsgId,
        min_id: i64,
        max_id: i64,
        limit: usize,
    ) -> Result<Vec<AyuEditedRevision>, CoreError>;

    fn delete_regex_filter(&self, filter_id: &[u8]) -> Result<bool, CoreError>;

    fn delete_all_regex_filters(&self) -> Result<usize, CoreError>;

    fn save_regex_exclusion(
        &self,
        exclusion: &RegexFilterGlobalExclusion,
    ) -> Result<(), CoreError>;

    fn load_regex_exclusions(&self) -> Result<Vec<RegexFilterGlobalExclusion>, CoreError>;

    fn delete_regex_exclusion(
        &self,
        dialog_id: DialogId,
        filter_id: &[u8],
    ) -> Result<bool, CoreError>;

    fn delete_exclusions_by_filter_id(&self, filter_id: &[u8]) -> Result<usize, CoreError>;

    fn delete_all_regex_exclusions(&self) -> Result<usize, CoreError>;
}

impl<T: AyuAuditRepository + ?Sized> AyuAuditRepository for std::sync::Arc<T> {
    fn record_deleted_message(
        &self,
        account_id: AccountId,
        entry: &AyuDeletedMessage,
    ) -> Result<(), CoreError> {
        self.as_ref().record_deleted_message(account_id, entry)
    }

    fn load_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
        limit: usize,
    ) -> Result<Vec<AyuDeletedMessage>, CoreError> {
        self.as_ref()
            .load_deleted_messages(account_id, dialog_id, topic_id, limit)
    }

    fn has_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
    ) -> Result<bool, CoreError> {
        self.as_ref()
            .has_deleted_messages(account_id, dialog_id, topic_id)
    }

    fn record_edited_revision(
        &self,
        account_id: AccountId,
        entry: &AyuEditedRevision,
    ) -> Result<(), CoreError> {
        self.as_ref().record_edited_revision(account_id, entry)
    }

    fn load_edited_revisions(
        &self,
        user_id: UserId,
        dialog_id: DialogId,
        msg_id: MsgId,
        limit: usize,
    ) -> Result<Vec<AyuEditedRevision>, CoreError> {
        self.as_ref()
            .load_edited_revisions(user_id, dialog_id, msg_id, limit)
    }

    fn has_edited_revisions(
        &self,
        user_id: UserId,
        dialog_id: DialogId,
        msg_id: MsgId,
    ) -> Result<bool, CoreError> {
        self.as_ref()
            .has_edited_revisions(user_id, dialog_id, msg_id)
    }

    fn save_regex_filter(&self, rule: &RegexFilterRule) -> Result<(), CoreError> {
        self.as_ref().save_regex_filter(rule)
    }

    fn load_regex_filters(&self) -> Result<Vec<RegexFilterRule>, CoreError> {
        self.as_ref().load_regex_filters()
    }

    fn query_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
        min_msg_id: MsgId,
        max_msg_id: MsgId,
        limit: usize,
        search_query: &str,
    ) -> Result<Vec<AyuDeletedMessage>, CoreError> {
        self.as_ref().query_deleted_messages(
            account_id,
            dialog_id,
            topic_id,
            min_msg_id,
            max_msg_id,
            limit,
            search_query,
        )
    }

    fn remove_deleted_message(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        msg_id: MsgId,
    ) -> Result<usize, CoreError> {
        self.as_ref()
            .remove_deleted_message(account_id, dialog_id, msg_id)
    }

    fn clear_deleted_messages(
        &self,
        account_id: AccountId,
        dialog_id: DialogId,
        topic_id: TopicId,
    ) -> Result<usize, CoreError> {
        self.as_ref()
            .clear_deleted_messages(account_id, dialog_id, topic_id)
    }

    fn query_edited_revisions(
        &self,
        user_id: UserId,
        dialog_id: DialogId,
        msg_id: MsgId,
        min_id: i64,
        max_id: i64,
        limit: usize,
    ) -> Result<Vec<AyuEditedRevision>, CoreError> {
        self.as_ref()
            .query_edited_revisions(user_id, dialog_id, msg_id, min_id, max_id, limit)
    }

    fn delete_regex_filter(&self, filter_id: &[u8]) -> Result<bool, CoreError> {
        self.as_ref().delete_regex_filter(filter_id)
    }

    fn delete_all_regex_filters(&self) -> Result<usize, CoreError> {
        self.as_ref().delete_all_regex_filters()
    }

    fn save_regex_exclusion(
        &self,
        exclusion: &RegexFilterGlobalExclusion,
    ) -> Result<(), CoreError> {
        self.as_ref().save_regex_exclusion(exclusion)
    }

    fn load_regex_exclusions(&self) -> Result<Vec<RegexFilterGlobalExclusion>, CoreError> {
        self.as_ref().load_regex_exclusions()
    }

    fn delete_regex_exclusion(
        &self,
        dialog_id: DialogId,
        filter_id: &[u8],
    ) -> Result<bool, CoreError> {
        self.as_ref().delete_regex_exclusion(dialog_id, filter_id)
    }

    fn delete_exclusions_by_filter_id(&self, filter_id: &[u8]) -> Result<usize, CoreError> {
        self.as_ref().delete_exclusions_by_filter_id(filter_id)
    }

    fn delete_all_regex_exclusions(&self) -> Result<usize, CoreError> {
        self.as_ref().delete_all_regex_exclusions()
    }
}

/// Byte-bounded media cache interface for LRU eviction across active/background accounts.
pub trait MediaCacheBackend: Send + Sync {
    fn put(&self, key: u64, data: &[u8]) -> Result<(), CoreError>;
    fn get(&self, key: u64) -> Option<Vec<u8>>;
    fn current_bytes(&self) -> usize;
    fn max_bytes(&self) -> usize;
    fn shrink_to(&self, target_bytes: usize) -> usize;
}

/// Abstract network transport capability for decoupling domain logic from socket details.
pub trait NetworkTransport: Send + Sync {
    fn is_connected(&self, account_id: AccountId) -> bool;
    fn queued_outbound_count(&self, account_id: AccountId) -> usize;
}

/// Repository abstraction for persisting multi-account authorization metadata and DC keys.
pub trait AuthStateRepository: Send + Sync {
    fn save_account_auth(
        &self,
        account_id: AccountId,
        index: i32,
        is_test_dc: bool,
        is_premium: bool,
        auth_state: AuthState,
        blob: &MtpAuthorizationBlob,
    ) -> Result<(), CoreError>;

    fn load_account_auth(
        &self,
        account_id: AccountId,
    ) -> Result<Option<MtpAuthorizationBlob>, CoreError>;

    fn delete_account_auth(&self, account_id: AccountId) -> Result<(), CoreError>;
}

impl<T: AuthStateRepository + ?Sized> AuthStateRepository for std::sync::Arc<T> {
    fn save_account_auth(
        &self,
        account_id: AccountId,
        index: i32,
        is_test_dc: bool,
        is_premium: bool,
        auth_state: AuthState,
        blob: &MtpAuthorizationBlob,
    ) -> Result<(), CoreError> {
        self.as_ref()
            .save_account_auth(account_id, index, is_test_dc, is_premium, auth_state, blob)
    }

    fn load_account_auth(
        &self,
        account_id: AccountId,
    ) -> Result<Option<MtpAuthorizationBlob>, CoreError> {
        self.as_ref().load_account_auth(account_id)
    }

    fn delete_account_auth(&self, account_id: AccountId) -> Result<(), CoreError> {
        self.as_ref().delete_account_auth(account_id)
    }
}

/// Snapshot of MTProto update synchronization cursors (`pts`, `qts`, `date`, `seq`).
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncStateSnapshot {
    pub pts: i32,
    pub qts: i32,
    pub date: i32,
    pub seq: i32,
}

/// Repository abstraction for persisting MTProto update state (`pts`/`qts`/`date`/`seq`) per account.
pub trait SyncStateRepository: Send + Sync {
    fn save_sync_state(
        &self,
        account_id: AccountId,
        state: SyncStateSnapshot,
    ) -> Result<(), CoreError>;

    fn load_sync_state(&self, account_id: AccountId) -> Result<Option<SyncStateSnapshot>, CoreError>;

    fn save_channel_pts(
        &self,
        account_id: AccountId,
        channel_id: u64,
        pts: i32,
    ) -> Result<(), CoreError>;

    fn load_channel_pts(
        &self,
        account_id: AccountId,
        channel_id: u64,
    ) -> Result<Option<i32>, CoreError>;
}

impl<T: SyncStateRepository + ?Sized> SyncStateRepository for std::sync::Arc<T> {
    fn save_sync_state(
        &self,
        account_id: AccountId,
        state: SyncStateSnapshot,
    ) -> Result<(), CoreError> {
        self.as_ref().save_sync_state(account_id, state)
    }

    fn load_sync_state(&self, account_id: AccountId) -> Result<Option<SyncStateSnapshot>, CoreError> {
        self.as_ref().load_sync_state(account_id)
    }

    fn save_channel_pts(
        &self,
        account_id: AccountId,
        channel_id: u64,
        pts: i32,
    ) -> Result<(), CoreError> {
        self.as_ref().save_channel_pts(account_id, channel_id, pts)
    }

    fn load_channel_pts(
        &self,
        account_id: AccountId,
        channel_id: u64,
    ) -> Result<Option<i32>, CoreError> {
        self.as_ref().load_channel_pts(account_id, channel_id)
    }
}
