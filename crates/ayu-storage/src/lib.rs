pub mod error;
pub mod media_cache;
pub mod sqlite_repo;

pub use error::StorageError;
pub use media_cache::ByteBoundedMediaCache;
pub use sqlite_repo::SqliteStorage;

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};

    use ayu_core::{
        AccountId, AuthState, AuthStateRepository, AyuAuditRepository, AyuDeletedMessage,
        AyuEditedRevision, CompactDialog, CompactMessage, DcAuthKeyEntry, DialogFlags, DialogId,
        DialogRepository, EntityKind, MediaCacheKind, MediaDescriptor, MediaKind,
        MessageEntitySpan, MessageFlags, MessageRepository, MsgId, MtpAuthorizationBlob, PeerId,
        RegexFilterGlobalExclusion, RegexFilterRule, SyncStateRepository, SyncStateSnapshot,
        TopicId, UserId, VirtualizedHistoryWindow,
    };
    use smallvec::smallvec;

    use super::*;

    fn sample_message(dialog_id: DialogId, id: i32, with_media: bool) -> CompactMessage {
        CompactMessage {
            dialog_id,
            sender_id: PeerId::from_user(UserId(500)),
            grouped_id: 0,
            text: format!("Stored msg {id}").into_boxed_str(),
            author_name: Some(Arc::from("Tester")),
            media: if with_media {
                Some(Box::new(MediaDescriptor {
                    file_id: 777,
                    access_hash: 888,
                    size_bytes: 12_345,
                    mime_type: "image/webp".into(),
                    local_path: Some("/tmp/a.webp".into()),
                    thumb_path: None,
                    width: 512,
                    height: 512,
                    duration_secs: 0,
                    kind: MediaKind::Sticker,
                    has_spoiler: false,
                }))
            } else {
                None
            },
            entities: smallvec![MessageEntitySpan {
                extra: 0,
                offset: 0,
                length: 6,
                kind: EntityKind::Bold as u8,
                flags: 0,
            }],
            id: MsgId(id),
            reply_to_msg_id: MsgId(0),
            date_unix: 1_700_000_000 + id,
            edit_date_unix: 0,
            views: 1,
            flags: MessageFlags::OUTGOING,
        }
    }

    #[test]
    fn sqlite_wal_mode_and_window_pagination() -> Result<(), Box<dyn std::error::Error>> {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let db_path = std::env::temp_dir().join(format!("ayu_storage_wal_{nanos}.db"));
        let storage = Arc::new(SqliteStorage::open(&db_path)?);
        assert_eq!(storage.journal_mode()?.to_lowercase(), "wal");

        let acc = AccountId(1);
        let dlg = DialogId(-100_123_456);
        let batch: Vec<CompactMessage> = (1..=300)
            .map(|i| sample_message(dlg, i, i % 10 == 0))
            .collect();
        let written = storage.upsert_batch(acc, &batch)?;
        assert_eq!(written, 300);

        let mut window = VirtualizedHistoryWindow::new(Arc::clone(&storage), acc, dlg, 50)?;
        window.jump_to(MsgId(150))?;
        assert_eq!(window.len(), 50);

        let older_added = window.scroll_up(20)?;
        assert_eq!(older_added, 20);
        assert_eq!(window.len(), 50);

        let newer_added = window.scroll_down(20)?;
        assert_eq!(newer_added, 20);
        assert_eq!(window.len(), 50);

        let _ = std::fs::remove_file(&db_path);
        let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
        let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
        Ok(())
    }

    #[test]
    fn sqlite_dialogs_and_ayu_audit_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let storage = SqliteStorage::open_in_memory()?;
        let acc = AccountId(7);
        let dlg = DialogId(42);

        storage.upsert_dialog(
            acc,
            &CompactDialog {
                dialog_id: dlg,
                peer_id: PeerId::from_user(UserId(42)),
                title: "Saved".into(),
                draft_text: Some("draft".into()),
                top_message_id: MsgId(12),
                read_inbox_max_id: MsgId(10),
                read_outbox_max_id: MsgId(12),
                last_message_date: 1_700_000_999,
                unread_count: 2,
                unread_mentions_count: 0,
                folder_id: 0,
                flags: DialogFlags::PINNED,
            },
        )?;
        assert_eq!(storage.dialog_count(acc, 0)?, 1);
        let rows = storage.load_dialog_slice(acc, 0, 0, 10)?;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].title.as_ref(), "Saved");

        let msg = sample_message(dlg, 99, false);
        storage.record_deleted_message(
            acc,
            &AyuDeletedMessage {
                user_id: UserId(7),
                topic_id: TopicId(0),
                entity_create_date: 1_700_001_000,
                message: msg.clone(),
            },
        )?;
        let deleted = storage.load_deleted_messages(acc, dlg, TopicId(0), 10)?;
        assert_eq!(deleted.len(), 1);
        assert!(deleted[0].message.flags.contains(MessageFlags::AYU_DELETED));

        storage.record_edited_revision(
            acc,
            &AyuEditedRevision {
                user_id: UserId(7),
                entity_create_date: 1_700_001_111,
                message: msg,
            },
        )?;
        let edits = storage.load_edited_revisions(UserId(7), dlg, MsgId(99), 10)?;
        assert_eq!(edits.len(), 1);
        assert!(edits[0].message.flags.contains(MessageFlags::AYU_EDITED));

        storage.save_regex_filter(&RegexFilterRule {
            id: Box::from(*b"flt-1"),
            pattern: "(?i)spam".into(),
            dialog_id: None,
            enabled: true,
            reversed: false,
            case_insensitive: true,
        })?;
        assert_eq!(storage.load_regex_filters()?.len(), 1);

        Ok(())
    }

    #[test]
    fn media_cache_enforces_strict_byte_limit() -> Result<(), StorageError> {
        let cache = ByteBoundedMediaCache::new(1024);
        cache.put_bytes(1, &[1u8; 400])?;
        cache.put_bytes(2, &[2u8; 400])?;
        assert_eq!(cache.current_bytes(), 800);

        // Touch key 1 so key 2 becomes least-recently-used.
        assert!(cache.get_bytes(1).is_some());

        // Inserting 400 more bytes must evict key 2 while keeping key 1.
        cache.put_bytes(3, &[3u8; 400])?;
        assert!(cache.current_bytes() <= 1024);
        assert!(cache.get_bytes(1).is_some());
        assert!(cache.get_bytes(2).is_none());
        assert!(cache.get_bytes(3).is_some());

        // Single blob larger than max_bytes must fail gracefully without panicking.
        let err = cache.put_bytes(4, &[4u8; 2048]);
        assert!(matches!(
            err,
            Err(StorageError::PayloadExceedsCacheLimit { .. })
        ));

        let freed = cache.shrink_for_background_account(400);
        assert!(freed >= 400);
        assert!(cache.current_bytes() <= 400);
        Ok(())
    }

    #[test]
    fn sqlite_account_auth_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let storage = SqliteStorage::open_in_memory()?;
        let acc = AccountId(3);
        let mut blob = MtpAuthorizationBlob {
            user_id: UserId(987_654_321),
            main_dc_id: 2,
            ..MtpAuthorizationBlob::default()
        };
        blob.keys.push(DcAuthKeyEntry::new(2, [0x11u8; 256], false, false));
        blob.keys.push(DcAuthKeyEntry::new(4, [0x22u8; 256], false, false));

        storage.save_account_auth(acc, 0, false, true, AuthState::Authorized, &blob)?;
        let loaded = storage.load_account_auth(acc)?;
        assert_eq!(loaded.as_ref(), Some(&blob));

        storage.delete_account_auth(acc)?;
        assert!(storage.load_account_auth(acc)?.is_none());
        Ok(())
    }

    #[test]
    fn sqlite_batch_search_clear_and_scoped_media_cache() -> Result<(), Box<dyn std::error::Error>> {
        let storage = SqliteStorage::open_in_memory()?;
        let acc = AccountId(4);
        let dlg1 = DialogId(10);
        let dlg2 = DialogId(20);

        let dialogs = vec![
            CompactDialog {
                dialog_id: dlg1,
                peer_id: PeerId::from_user(UserId(10)),
                title: "Rust Engineering".into(),
                draft_text: None,
                top_message_id: MsgId(5),
                read_inbox_max_id: MsgId(1),
                read_outbox_max_id: MsgId(1),
                last_message_date: 1_700_000_200,
                unread_count: 4,
                unread_mentions_count: 0,
                folder_id: 0,
                flags: DialogFlags::empty(),
            },
            CompactDialog {
                dialog_id: dlg2,
                peer_id: PeerId::from_user(UserId(20)),
                title: "100%_Match\\Test".into(),
                draft_text: None,
                top_message_id: MsgId(2),
                read_inbox_max_id: MsgId(0),
                read_outbox_max_id: MsgId(0),
                last_message_date: 1_700_000_100,
                unread_count: 2,
                unread_mentions_count: 0,
                folder_id: 0,
                flags: DialogFlags::empty(),
            },
        ];
        assert_eq!(storage.upsert_dialog_batch(acc, &dialogs)?, 2);
        assert!(storage.update_dialog_read_state(acc, dlg1, MsgId(5), 0)?);
        assert_eq!(storage.search_dialogs(acc, "100%_Match", 10)?.len(), 1);

        let msgs: Vec<CompactMessage> = (1..=10)
            .map(|id| sample_message(dlg1, id, false))
            .collect();
        storage.upsert_batch(acc, &msgs)?;
        assert_eq!(storage.dialog_message_count(acc, dlg1)?, 10);
        assert_eq!(storage.search_dialog_messages(acc, dlg1, "msg 7", 10)?.len(), 1);
        assert_eq!(storage.delete_messages_up_to(acc, dlg1, MsgId(5))?, 5);
        assert_eq!(storage.dialog_message_count(acc, dlg1)?, 5);

        assert!(!storage.has_deleted_messages(acc, dlg1, TopicId(0))?);
        storage.record_deleted_message(
            acc,
            &AyuDeletedMessage {
                user_id: UserId(4),
                topic_id: TopicId(0),
                entity_create_date: 1_700_000_500,
                message: sample_message(dlg1, 8, false),
            },
        )?;
        assert!(storage.has_deleted_messages(acc, dlg1, TopicId(0))?);

        assert!(!storage.has_edited_revisions(UserId(4), dlg1, MsgId(8))?);
        storage.record_edited_revision(
            acc,
            &AyuEditedRevision {
                user_id: UserId(4),
                entity_create_date: 1_700_000_600,
                message: sample_message(dlg1, 8, false),
            },
        )?;
        assert!(storage.has_edited_revisions(UserId(4), dlg1, MsgId(8))?);

        assert!(storage.remove_dialog(acc, dlg2)?);
        assert_eq!(storage.dialog_count(acc, 0)?, 1);

        let cache = ByteBoundedMediaCache::new(4096);
        cache.put_scoped(acc, MediaCacheKind::Avatar, 1, &[1u8; 256])?;
        cache.put_scoped(acc, MediaCacheKind::DecodedPhoto, 2, &[2u8; 1024])?;
        assert_eq!(cache.account_bytes(acc), 1280);
        assert_eq!(cache.evict_account(acc, true), 1024);
        assert!(cache.get_scoped(acc, MediaCacheKind::Avatar, 1).is_some());
        assert!(cache.get_scoped(acc, MediaCacheKind::DecodedPhoto, 2).is_none());
        assert_eq!(cache.evict_account(acc, false), 256);
        assert_eq!(cache.account_bytes(acc), 0);
        Ok(())
    }

    #[test]
    fn sqlite_sync_state_and_channel_pts_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let storage = SqliteStorage::open_in_memory()?;
        let acc = AccountId(11);
        assert!(storage.load_sync_state(acc)?.is_none());

        let snapshot = SyncStateSnapshot {
            pts: 12_345,
            qts: 77,
            date: 1_700_005_000,
            seq: 9,
        };
        storage.save_sync_state(acc, snapshot)?;
        assert_eq!(storage.load_sync_state(acc)?, Some(snapshot));

        let updated = SyncStateSnapshot {
            pts: 12_350,
            ..snapshot
        };
        storage.save_sync_state(acc, updated)?;
        assert_eq!(storage.load_sync_state(acc)?, Some(updated));

        let channel_id = 998_877_665u64;
        assert!(storage.load_channel_pts(acc, channel_id)?.is_none());
        storage.save_channel_pts(acc, channel_id, 404)?;
        assert_eq!(storage.load_channel_pts(acc, channel_id)?, Some(404));
        storage.save_channel_pts(acc, channel_id, 409)?;
        assert_eq!(storage.load_channel_pts(acc, channel_id)?, Some(409));
        Ok(())
    }

    #[test]
    fn sqlite_stage5_deleted_search_clear_and_regex_exclusions()
        -> Result<(), Box<dyn std::error::Error>>
    {
        let storage = SqliteStorage::open_in_memory()?;
        let acc = AccountId(5);
        let dlg = DialogId(-100_500);

        for msg_id in 1..=5i32 {
            let mut msg = sample_message(dlg, msg_id, false);
            msg.text = if msg_id % 2 == 0 {
                format!("Important invoice #{msg_id}").into_boxed_str()
            } else {
                format!("Casual note #{msg_id}").into_boxed_str()
            };
            storage.record_deleted_message(
                acc,
                &AyuDeletedMessage {
                    user_id: UserId(5),
                    topic_id: TopicId(10),
                    entity_create_date: 1_700_000_000 + msg_id,
                    message: msg,
                },
            )?;
        }

        let ranged = storage.query_deleted_messages(
            acc,
            dlg,
            TopicId(0),
            MsgId(1),
            MsgId(5),
            10,
            "",
        )?;
        assert_eq!(ranged.len(), 3);
        assert_eq!(ranged[0].message.id, MsgId(4));

        let searched = storage.query_deleted_messages(
            acc,
            dlg,
            TopicId(10),
            MsgId(0),
            MsgId(0),
            10,
            "invoice",
        )?;
        assert_eq!(searched.len(), 2);

        assert_eq!(storage.remove_deleted_message(acc, dlg, MsgId(2))?, 1);
        assert_eq!(
            storage.clear_deleted_messages(acc, dlg, TopicId(10))?,
            4
        );
        assert!(!storage.has_deleted_messages(acc, dlg, TopicId(0))?);

        storage.save_regex_filter(&RegexFilterRule {
            id: Box::from(*b"f1"),
            pattern: "spam".into(),
            dialog_id: None,
            enabled: true,
            reversed: false,
            case_insensitive: true,
        })?;
        storage.save_regex_exclusion(&RegexFilterGlobalExclusion {
            dialog_id: dlg,
            filter_id: Box::from(*b"f1"),
        })?;
        assert_eq!(storage.load_regex_exclusions()?.len(), 1);
        assert_eq!(storage.delete_exclusions_by_filter_id(b"f1")?, 1);
        assert!(storage.delete_regex_filter(b"f1")?);
        assert!(storage.load_regex_filters()?.is_empty());
        Ok(())
    }
}
