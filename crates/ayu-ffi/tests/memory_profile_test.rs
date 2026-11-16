use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use ayu_ffi::{create_ayu_engine, FfiDialogRow, FfiMessageRow, FfiRegexFilter};

#[test]
fn multi_account_scroll_and_media_cache_stay_strictly_bounded() -> Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let db_path = std::env::temp_dir().join(format!("ayu_profile_test_{nanos}.db"));
    let db_str = db_path.to_string_lossy().into_owned();

    // 16 MiB hard media cache ceiling across 3 simultaneous accounts.
    let engine = create_ayu_engine(&db_str, 16)?;
    engine.register_account(1, 101, false)?;
    engine.register_account(2, 102, false)?;
    engine.register_account(3, 103, false)?;

    // Seed 5,000 messages into SQLite WAL across 3 accounts.
    for msg_id in 1..=5_000i32 {
        let account_id = ((msg_id - 1) % 3 + 1) as u64;
        let dialog_id = (account_id as i64) * 1_000;
        engine.upsert_message(
            account_id,
            FfiMessageRow {
                dialog_id,
                msg_id,
                sender_id: account_id,
                reply_to_msg_id: 0,
                date_unix: 1_700_000_000 + msg_id,
                edit_date_unix: 0,
                views: 0,
                flags: 1,
                grouped_id: 0,
                author_name: "BenchmarkUser".to_owned(),
                text: format!("Message body payload #{msg_id} with fixed test content"),
                media_kind: 0,
                media_path: String::new(),
            },
        )?;
    }

    // Simulate scrolling through all 5,000 messages with a 100-message viewport window.
    for account_id in 1..=3u64 {
        engine.activate_account(account_id)?;
        let dialog_id = (account_id as i64) * 1_000;
        for center in (50..=5_000).step_by(100) {
            let rows = engine.load_viewport_messages(account_id, dialog_id, center, 100)?;
            assert!(rows.len() <= 100);
            let stats = engine.get_memory_stats();
            assert!(
                stats.cached_window_messages <= 200,
                "resident viewport exceeded bound: {}",
                stats.cached_window_messages
            );
        }
    }

    // Push 50 MiB of decoded media blobs (100 x 512 KiB) through the 16 MiB LRU cache.
    let blob = vec![0xABu8; 512 * 1024];
    for key in 1..=100u64 {
        engine.put_media_cache(key, &blob)?;
        let stats = engine.get_memory_stats();
        assert!(
            stats.media_cache_bytes <= 16 * 1024 * 1024,
            "media cache exceeded 16 MiB cap: {}",
            stats.media_cache_bytes
        );
    }

    // Switching accounts + evicting inactive caches drops inactive windows and halves media cache.
    engine.activate_account(1)?;
    let _freed = engine.evict_inactive_caches();
    let final_stats = engine.get_memory_stats();
    assert_eq!(final_stats.active_accounts, 3);
    assert!(final_stats.resident_windows <= 1);
    assert!(final_stats.cached_window_messages <= 100);
    assert!(final_stats.media_cache_bytes <= 8 * 1024 * 1024);

    drop(engine);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    Ok(())
}

#[test]
fn multi_account_auth_lifecycle_and_1000_switches_no_leak() -> Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let db_path = std::env::temp_dir().join(format!("ayu_auth_profile_{nanos}.db"));
    let db_str = db_path.to_string_lossy().into_owned();

    let engine = create_ayu_engine(&db_str, 8)?;
    for idx in 0..4u64 {
        engine.register_account(idx, 0, idx == 3)?;
        let mut blob = Vec::new();
        blob.extend_from_slice(&u64::MAX.to_be_bytes());
        blob.extend_from_slice(&(10_000 + idx).to_be_bytes());
        blob.extend_from_slice(&2i32.to_be_bytes());
        blob.extend_from_slice(&1i32.to_be_bytes());
        blob.extend_from_slice(&2i32.to_be_bytes());
        blob.extend_from_slice(&[(idx as u8).wrapping_add(1); 256]);
        blob.extend_from_slice(&0i32.to_be_bytes());

        let snap = engine.import_mtp_authorization(idx, &blob)?;
        assert_eq!(snap.user_id, 10_000 + idx);
        engine.complete_authorization(idx, 10_000 + idx, 2, idx % 2 == 0)?;
        engine.set_account_unread_state(idx, (idx as u32) + 1, idx == 0)?;
    }

    assert_eq!(engine.authed_accounts_count(), 4);
    assert_eq!(engine.max_accounts_limit(), 103);

    let badge = engine.compute_unread_badge();
    assert_eq!(badge.total_unread, 10);
    assert!(!badge.all_muted);

    let order = [10_002u64, 10_000u64];
    let ordered = engine.ordered_account_ids(&order);
    assert_eq!(ordered[0], 2);
    assert_eq!(ordered[1], 0);

    for step in 0..1_000u64 {
        let acc = step % 4;
        engine.activate_account(acc)?;
        engine.upsert_message(
            acc,
            FfiMessageRow {
                dialog_id: 900 + acc as i64,
                msg_id: (step as i32) + 1,
                sender_id: 10_000 + acc,
                reply_to_msg_id: 0,
                date_unix: 1_700_000_000,
                edit_date_unix: 0,
                views: 0,
                flags: 1,
                grouped_id: 0,
                author_name: "Switcher".to_owned(),
                text: "Switch cycle payload".to_owned(),
                media_kind: 0,
                media_path: String::new(),
            },
        )?;
        let _ = engine.load_viewport_messages(acc, 900 + acc as i64, (step as i32) + 1, 50)?;
        let stats = engine.get_memory_stats();
        assert!(stats.resident_windows <= 1);
        assert!(stats.cached_window_messages <= 50);
    }

    engine.force_logout(1)?;
    let exported_after_logout = engine.export_mtp_authorization(1)?;
    assert!(!exported_after_logout.is_empty());
    assert_eq!(engine.authed_accounts_count(), 3);
    assert_eq!(engine.redundant_account_ids(), vec![1]);

    drop(engine);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    Ok(())
}

#[test]
fn stage3_dialogs_history_and_scoped_media_cache_budget_test() -> Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let db_path = std::env::temp_dir().join(format!("ayu_stage3_profile_{nanos}.db"));
    let db_str = db_path.to_string_lossy().into_owned();

    let engine = create_ayu_engine(&db_str, 8)?;
    engine.register_account(0, 2001, false)?;
    engine.register_account(1, 2002, false)?;

    let mut dialogs = Vec::with_capacity(400);
    for i in 1..=400i64 {
        dialogs.push(FfiDialogRow {
            dialog_id: i,
            peer_id: i as u64,
            folder_id: 0,
            title: format!("Chat #{i}"),
            draft_text: String::new(),
            top_message_id: 100,
            read_inbox_max_id: 50,
            read_outbox_max_id: 50,
            last_message_date: 1_700_000_000 + i as i32,
            unread_count: 3,
            unread_mentions_count: 0,
            flags: 0,
        });
    }
    assert_eq!(engine.upsert_dialog_batch(0, &dialogs)?, 400);
    assert!(engine.update_dialog_read_state(0, 400, 100, 0)?);

    let page0 = engine.load_dialog_slice(0, 0, 0, 50)?;
    assert_eq!(page0.len(), 50);
    let page1 = engine.load_dialog_slice(0, 0, 50, 50)?;
    assert_eq!(page1.len(), 50);
    assert_eq!(engine.get_memory_stats().cached_dialog_rows, 50);
    assert!(engine.search_dialogs(0, "Chat #399", 10)?.len() == 1);

    let mut msgs = Vec::with_capacity(300);
    for m in 1..=300i32 {
        msgs.push(FfiMessageRow {
            dialog_id: 400,
            msg_id: m,
            sender_id: 2001,
            reply_to_msg_id: 0,
            date_unix: 1_700_000_000 + m,
            edit_date_unix: 0,
            views: 0,
            flags: 1,
            grouped_id: 0,
            author_name: "Tester".to_owned(),
            text: format!("Payload line #{m}"),
            media_kind: 0,
            media_path: String::new(),
        });
    }
    assert_eq!(engine.upsert_message_batch(0, &msgs)?, 300);
    let initial = engine.load_viewport_messages(0, 400, 150, 60)?;
    assert_eq!(initial.len(), 60);
    let scrolled_older = engine.scroll_viewport_older(0, 400, 20)?;
    assert_eq!(scrolled_older.len(), 60);
    let scrolled_newer = engine.scroll_viewport_newer(0, 400, 20)?;
    assert_eq!(scrolled_newer.len(), 60);
    assert_eq!(engine.search_dialog_messages(0, 400, "Payload line #250", 10)?.len(), 1);

    assert_eq!(engine.delete_messages(0, 400, &[150, 151])?, 2);
    assert_eq!(engine.clear_dialog_history_up_to(0, 400, 100)?, 100);
    engine.unload_dialog_viewport(0, 400);
    assert_eq!(engine.get_memory_stats().resident_windows, 0);

    // Scoped media cache test: heavy_only keeps Avatar(1) and drops DecodedPhoto(3).
    engine.put_scoped_media_cache(0, 1, 77, &[0xAAu8; 4096])?;
    engine.put_scoped_media_cache(0, 3, 88, &[0xBBu8; 65_536])?;
    assert_eq!(engine.evict_account_media_cache(0, true), 65_536);
    assert_eq!(engine.get_scoped_media_cache(0, 1, 77).len(), 4096);
    assert!(engine.get_scoped_media_cache(0, 3, 88).is_empty());

    assert!(engine.remove_dialog(0, 1)?);

    drop(engine);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    Ok(())
}

#[test]
fn stage4_mtproto_sync_messaging_and_file_transfer_test() -> Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let db_path = std::env::temp_dir().join(format!("ayu_stage4_profile_{nanos}.db"));
    let db_str = db_path.to_string_lossy().into_owned();

    {
        let engine = create_ayu_engine(&db_str, 8)?;
        engine.register_account(0, 3001, false)?;
        engine.init_sync_state(0, 100, 5, 1_700_000_000, 10)?;

        let st = engine.get_sync_state(0)?;
        assert_eq!(st.pts, 100);
        assert_eq!(st.qts, 5);
        assert_eq!(st.seq, 10);

        let in_msg = FfiMessageRow {
            dialog_id: 700,
            msg_id: 500,
            sender_id: 4001,
            reply_to_msg_id: 0,
            date_unix: 1_700_000_010,
            edit_date_unix: 0,
            views: 0,
            flags: 0,
            grouped_id: 0,
            author_name: "Bob".to_owned(),
            text: "Incoming in-order".to_owned(),
            media_kind: 0,
            media_path: String::new(),
        };
        let out_ok = engine.receive_incoming_message(0, 0, 101, 1, in_msg.clone())?;
        assert!(out_ok.applied);
        assert!(!out_ok.gap_detected);
        assert_eq!(out_ok.current_pts, 101);

        let out_dup = engine.receive_incoming_message(0, 0, 101, 1, in_msg)?;
        assert!(out_dup.duplicate);
        assert!(!out_dup.applied);

        let gap_msg = FfiMessageRow {
            dialog_id: 700,
            msg_id: 505,
            sender_id: 4001,
            reply_to_msg_id: 0,
            date_unix: 1_700_000_020,
            edit_date_unix: 0,
            views: 0,
            flags: 0,
            grouped_id: 0,
            author_name: "Bob".to_owned(),
            text: "Gap skipped".to_owned(),
            media_kind: 0,
            media_path: String::new(),
        };
        let out_gap = engine.receive_incoming_message(0, 0, 105, 1, gap_msg)?;
        assert!(out_gap.gap_detected);
        assert_eq!(out_gap.current_pts, 101);

        let chan_out = engine.apply_pts_update(0, 999_001, 12, 1)?;
        assert!(chan_out.applied);
        assert_eq!(chan_out.current_pts, 12);

        let local_id = engine.queue_outgoing_message(
            0,
            123_456_789,
            FfiMessageRow {
                dialog_id: 700,
                msg_id: 0,
                sender_id: 3001,
                reply_to_msg_id: 500,
                date_unix: 1_700_000_030,
                edit_date_unix: 0,
                views: 0,
                flags: 1,
                grouped_id: 0,
                author_name: "Me".to_owned(),
                text: "Pending outbound text".to_owned(),
                media_kind: 0,
                media_path: String::new(),
            },
        )?;
        assert!(local_id < 0);

        let vp_before = engine.load_viewport_messages(0, 700, 500, 50)?;
        assert_eq!(vp_before.len(), 2);
        assert_eq!(vp_before[0].msg_id, local_id);
        assert_eq!(vp_before[1].msg_id, 500);

        assert!(engine.confirm_outgoing_message(0, 123_456_789, 501, 1_700_000_035, 102, 1)?);
        let vp_after = engine.load_viewport_messages(0, 700, 501, 50)?;
        assert_eq!(vp_after.len(), 2);
        assert!(vp_after.iter().all(|m| m.msg_id > 0));
        assert!(vp_after
            .iter()
            .any(|m| m.msg_id == 501 && m.text == "Pending outbound text"));

        let dl_id = engine.enqueue_file_download(0, 2, 77_001, 512 * 1024, 5)?;
        let ul_id = engine.enqueue_file_upload(0, 2, 77_002, 40 * 1024 * 1024)?;

        let p_dl = engine.advance_file_transfer(dl_id, 128 * 1024)?;
        assert_eq!(p_dl.part_size, 128 * 1024);
        assert_eq!(p_dl.parts_completed, 1);
        assert_eq!(p_dl.parts_total, 4);

        let p_ul = engine.advance_file_transfer(ul_id, 128 * 1024)?;
        assert!(p_ul.is_big_file);
        assert!(engine.active_transfer_inflight_bytes() <= 512 * 1024);

        assert!(engine.cancel_file_transfer(ul_id));
        for _ in 0..3 {
            let _ = engine.advance_file_transfer(dl_id, 128 * 1024)?;
        }
        assert_eq!(engine.active_transfer_inflight_bytes(), 0);
    }

    // Reopen SQLite WAL storage and confirm sync state survived process restart.
    {
        let reopened = create_ayu_engine(&db_str, 8)?;
        let restored = reopened.get_sync_state(0)?;
        assert_eq!(restored.pts, 102);
        assert_eq!(restored.qts, 5);
        assert_eq!(restored.seq, 10);
    }

    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    Ok(())
}

#[test]
fn stage5_ayu_deleted_edited_filters_and_ghost_state_test() -> Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let db_path = std::env::temp_dir().join(format!("ayu_stage5_profile_{nanos}.db"));
    let db_str = db_path.to_string_lossy().into_owned();

    let engine = create_ayu_engine(&db_str, 8)?;
    engine.register_account(0, 5001, false)?;

    // Record deleted messages, search & paginate, then remove/clear.
    for msg_id in 1..=4i32 {
        engine.record_deleted_message(
            0,
            FfiMessageRow {
                dialog_id: -100_800,
                msg_id,
                sender_id: 9001,
                reply_to_msg_id: 0,
                date_unix: 1_700_000_000 + msg_id,
                edit_date_unix: 0,
                views: 0,
                flags: 0,
                grouped_id: 0,
                author_name: "Sender".to_owned(),
                text: if msg_id == 3 {
                    "Secret giveaway invite".to_owned()
                } else {
                    format!("Normal deleted #{msg_id}")
                },
                media_kind: 0,
                media_path: String::new(),
            },
        )?;
    }

    let searched = engine.query_deleted_messages(0, -100_800, 0, 0, 0, 10, "giveaway")?;
    assert_eq!(searched.len(), 1);
    assert_eq!(searched[0].msg_id, 3);

    assert_eq!(engine.remove_deleted_message(0, -100_800, 3)?, 1);
    assert_eq!(engine.clear_deleted_messages(0, -100_800, 0)?, 3);
    assert!(!engine.has_deleted_messages(0, -100_800));

    // Edit revisions pagination.
    for rev in 1..=3i32 {
        engine.record_edited_message(
            0,
            FfiMessageRow {
                dialog_id: -100_800,
                msg_id: 77,
                sender_id: 9001,
                reply_to_msg_id: 0,
                date_unix: 1_700_000_100,
                edit_date_unix: 1_700_000_100 + rev,
                views: 0,
                flags: 0,
                grouped_id: 0,
                author_name: "Editor".to_owned(),
                text: format!("Revision #{rev}"),
                media_kind: 0,
                media_path: String::new(),
            },
        )?;
    }
    let edits = engine.query_edited_messages(0, -100_800, 77, 0, 0, 2)?;
    assert_eq!(edits.len(), 2);
    assert_eq!(edits[0].text, "Revision #3");

    // Regex filter, global exclusion, shadow-ban, hide, and story ghost arming.
    engine.upsert_regex_filter(FfiRegexFilter {
        id: b"flt-global".to_vec(),
        pattern: r"casino".to_owned(),
        enabled: true,
        reversed: false,
        case_insensitive: true,
        has_dialog_id: false,
        dialog_id: 0,
    })?;
    assert_eq!(engine.list_regex_filters()?.len(), 1);
    engine.add_regex_exclusion(-100_801, b"flt-global")?;
    assert_eq!(engine.rebuild_filters_cache()?, 1);

    engine.set_filters_enabled(true, true, true);
    assert!(engine.evaluate_message_filter(-100_800, 1, 9001, false, "Best CASINO offer"));
    assert!(engine.has_filtered_messages(-100_800));
    assert!(!engine.evaluate_message_filter(-100_801, 2, 9001, false, "Best CASINO offer"));

    engine.set_shadow_banned(9002, true);
    assert!(engine.is_shadow_banned(9002));
    assert!(engine.evaluate_message_filter(-100_801, 3, 9002, false, "Clean text"));

    assert!(engine.toggle_filtered_messages_shown(-100_801));
    assert!(!engine.evaluate_message_filter(-100_801, 4, 9002, false, "Clean text"));

    engine.hide_message(-100_801, 5);
    assert!(engine.is_message_hidden(-100_801, 5));
    assert!(engine.evaluate_message_filter(-100_801, 5, 9001, false, "Visible text"));

    engine.arm_disable_ghost_on_story_close(0);
    assert!(engine.consume_disable_ghost_on_story_close(0));
    assert!(!engine.consume_disable_ghost_on_story_close(0));

    assert!(engine.remove_regex_exclusion(-100_801, b"flt-global")?);
    assert!(engine.delete_regex_filter(b"flt-global")?);
    assert_eq!(engine.clear_regex_exclusions()?, 0);
    assert_eq!(engine.clear_regex_filters()?, 0);

    drop(engine);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    Ok(())
}

#[test]
fn stage6_multi_account_rss_and_full_pipeline_profile_test() -> Result<()> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let db_path = std::env::temp_dir().join(format!("ayu_stage6_rss_profile_{nanos}.db"));
    let db_str = db_path.to_string_lossy().into_owned();

    let engine = create_ayu_engine(&db_str, 16)?;
    for acc in 1..=3u64 {
        engine.register_account(acc, 10_000 + acc, false)?;
        let mut blob = Vec::new();
        blob.extend_from_slice(&u64::MAX.to_be_bytes());
        blob.extend_from_slice(&(10_000 + acc).to_be_bytes());
        blob.extend_from_slice(&2i32.to_be_bytes());
        blob.extend_from_slice(&1i32.to_be_bytes());
        blob.extend_from_slice(&2i32.to_be_bytes());
        blob.extend_from_slice(&[(acc as u8).wrapping_add(1); 256]);
        blob.extend_from_slice(&0i32.to_be_bytes());
        let _ = engine.import_mtp_authorization(acc, &blob)?;
        engine.complete_authorization(acc, 10_000 + acc, 2, acc == 1)?;
        engine.init_sync_state(acc, 500, 10, 1_700_000_000, 20)?;
    }

    engine.upsert_regex_filter(FfiRegexFilter {
        id: b"rss-spam".to_vec(),
        pattern: r"promo".to_owned(),
        enabled: true,
        reversed: false,
        case_insensitive: true,
        has_dialog_id: false,
        dialog_id: 0,
    })?;
    assert_eq!(engine.rebuild_filters_cache()?, 1);

    for acc in 1..=3u64 {
        let mut dialogs = Vec::with_capacity(200);
        for d in 1..=200i64 {
            dialogs.push(FfiDialogRow {
                dialog_id: (acc as i64) * 10_000 + d,
                peer_id: d as u64,
                folder_id: 0,
                title: format!("Acc {acc} Dialog {d}"),
                draft_text: String::new(),
                top_message_id: 3_000,
                read_inbox_max_id: 2_900,
                read_outbox_max_id: 2_900,
                last_message_date: 1_700_000_000 + d as i32,
                unread_count: 1,
                unread_mentions_count: 0,
                flags: 0,
            });
        }
        assert_eq!(engine.upsert_dialog_batch(acc, &dialogs)?, 200);

        let dialog_id = (acc as i64) * 10_000 + 1;
        let mut msgs = Vec::with_capacity(3_000);
        for m in 1..=3_000i32 {
            msgs.push(FfiMessageRow {
                dialog_id,
                msg_id: m,
                sender_id: 10_000 + acc,
                reply_to_msg_id: 0,
                date_unix: 1_700_000_000 + m,
                edit_date_unix: 0,
                views: 0,
                flags: 1,
                grouped_id: 0,
                author_name: "ProfileUser".to_owned(),
                text: format!("Account {acc} history entry #{m} with steady length"),
                media_kind: 0,
                media_path: String::new(),
            });
        }
        assert_eq!(engine.upsert_message_batch(acc, &msgs)?, 3_000);

        engine.activate_account(acc)?;
        for center in (100..=3_000).step_by(200) {
            let vp = engine.load_viewport_messages(acc, dialog_id, center, 100)?;
            assert!(vp.len() <= 100);
        }
        let dl_page = engine.load_dialog_slice(acc, 0, 0, 50)?;
        assert_eq!(dl_page.len(), 50);

        let dl_id = engine.enqueue_file_download(acc, 2, acc * 100, 256 * 1024, 1)?;
        let _ = engine.advance_file_transfer(dl_id, 128 * 1024)?;
        let _ = engine.advance_file_transfer(dl_id, 128 * 1024)?;

        engine.record_deleted_message(acc, msgs[0].clone())?;
        engine.record_edited_message(acc, msgs[1].clone())?;
    }

    let media_chunk = vec![0xCDu8; 512 * 1024];
    for key in 1..=128u64 {
        engine.put_media_cache(key, &media_chunk)?;
    }

    engine.activate_account(1)?;
    let _freed = engine.evict_inactive_caches();
    let stats = engine.get_memory_stats();
    assert!(stats.resident_windows <= 1);
    assert!(stats.cached_window_messages <= 100);
    assert!(stats.media_cache_bytes <= 8 * 1024 * 1024);
    assert_eq!(engine.active_transfer_inflight_bytes(), 0);
    if cfg!(target_os = "linux") {
        assert!(stats.process_rss_bytes > 0);
        assert!(
            stats.process_rss_bytes < 200 * 1024 * 1024,
            "RSS exceeded 200 MiB target: {} MiB",
            stats.process_rss_bytes / (1024 * 1024)
        );
        println!(
            "[Stage 6 Profile] 3 accounts, 9000 msgs, 600 dialogs, 64 MiB media streamed -> RSS: {:.2} MiB, window msgs: {}, media cache: {:.2} MiB",
            stats.process_rss_bytes as f64 / (1024.0 * 1024.0),
            stats.cached_window_messages,
            stats.media_cache_bytes as f64 / (1024.0 * 1024.0),
        );
    }

    drop(engine);
    let _ = std::fs::remove_file(&db_path);
    let _ = std::fs::remove_file(db_path.with_extension("db-wal"));
    let _ = std::fs::remove_file(db_path.with_extension("db-shm"));
    Ok(())
}
