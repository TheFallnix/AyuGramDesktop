pub mod account;
pub mod auth;
pub mod error;
pub mod filters;
pub mod ids;
pub mod models;
pub mod traits;
pub mod window;

pub use account::{
    AccountController, AccountLifecycleState, MultiAccountOrchestrator, UnreadBadgeSummary,
    MAX_ACCOUNTS_FREE, MAX_ACCOUNTS_PREMIUM, TEST_DC_SHIFT,
};
pub use auth::{AuthState, DcAuthKeyEntry, MtpAuthorizationBlob};
pub use error::CoreError;
pub use filters::{AyuFilterEngine, MAX_FILTER_EVAL_CACHE_ENTRIES};
pub use ids::{AccountId, DialogId, MsgId, PeerId, PeerKind, TopicId, UserId};
pub use models::{
    AyuDeletedMessage, AyuEditedRevision, CompactDialog, CompactMessage, DialogFlags, EntityKind,
    GhostModeConfig, GhostModeFlags, MediaCacheKind, MediaDescriptor, MediaKind, MessageEntitySpan,
    MessageFlags, RegexFilterGlobalExclusion, RegexFilterRule, SendWithoutSoundOption,
};
pub use traits::{
    AuthStateRepository, AyuAuditRepository, DialogRepository, MediaCacheBackend,
    MessageRepository, NetworkTransport, SyncStateRepository, SyncStateSnapshot,
};
pub use window::{VirtualizedDialogWindow, VirtualizedHistoryWindow};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    use smallvec::SmallVec;

    use super::*;

    struct InMemoryMockRepo {
        rows: Mutex<BTreeMap<(AccountId, DialogId, i32), CompactMessage>>,
    }

    impl InMemoryMockRepo {
        fn seeded(account_id: AccountId, dialog_id: DialogId, count: i32) -> Self {
            let mut map = BTreeMap::new();
            let shared_author: Arc<str> = Arc::from("Alice");
            for i in 1..=count {
                map.insert(
                    (account_id, dialog_id, i),
                    CompactMessage {
                        dialog_id,
                        sender_id: PeerId::from_user(UserId(42)),
                        grouped_id: 0,
                        text: format!("Message #{i}").into_boxed_str(),
                        author_name: Some(Arc::clone(&shared_author)),
                        media: None,
                        entities: SmallVec::new(),
                        id: MsgId(i),
                        reply_to_msg_id: MsgId(0),
                        date_unix: 1_700_000_000 + i,
                        edit_date_unix: 0,
                        views: 0,
                        flags: MessageFlags::empty(),
                    },
                );
            }
            Self {
                rows: Mutex::new(map),
            }
        }
    }

    impl MessageRepository for InMemoryMockRepo {
        fn upsert_message(
            &self,
            account_id: AccountId,
            message: &CompactMessage,
        ) -> Result<(), CoreError> {
            let mut guard = self
                .rows
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            guard.insert((account_id, message.dialog_id, message.id.0), message.clone());
            Ok(())
        }

        fn upsert_batch(
            &self,
            account_id: AccountId,
            messages: &[CompactMessage],
        ) -> Result<usize, CoreError> {
            let mut guard = self
                .rows
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            for msg in messages {
                guard.insert((account_id, msg.dialog_id, msg.id.0), msg.clone());
            }
            Ok(messages.len())
        }

        fn load_slice_around(
            &self,
            account_id: AccountId,
            dialog_id: DialogId,
            center_msg_id: MsgId,
            limit: usize,
        ) -> Result<Vec<CompactMessage>, CoreError> {
            let guard = self
                .rows
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            let all: Vec<CompactMessage> = guard
                .range((account_id, dialog_id, i32::MIN)..=(account_id, dialog_id, i32::MAX))
                .map(|(_, v)| v.clone())
                .collect();
            let pos = all.partition_point(|m| m.id.0 < center_msg_id.0);
            let half = limit / 2;
            let start = pos.saturating_sub(half);
            let end = (start + limit).min(all.len());
            Ok(all[start..end].to_vec())
        }

        fn load_older_than(
            &self,
            account_id: AccountId,
            dialog_id: DialogId,
            before_msg_id: MsgId,
            limit: usize,
        ) -> Result<Vec<CompactMessage>, CoreError> {
            let guard = self
                .rows
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            let mut slice: Vec<CompactMessage> = guard
                .range((account_id, dialog_id, i32::MIN)..(account_id, dialog_id, before_msg_id.0))
                .rev()
                .take(limit)
                .map(|(_, v)| v.clone())
                .collect();
            slice.reverse();
            Ok(slice)
        }

        fn load_newer_than(
            &self,
            account_id: AccountId,
            dialog_id: DialogId,
            after_msg_id: MsgId,
            limit: usize,
        ) -> Result<Vec<CompactMessage>, CoreError> {
            let guard = self
                .rows
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            let start = after_msg_id.0.saturating_add(1);
            Ok(guard
                .range((account_id, dialog_id, start)..=(account_id, dialog_id, i32::MAX))
                .take(limit)
                .map(|(_, v)| v.clone())
                .collect())
        }

        fn delete_messages(
            &self,
            account_id: AccountId,
            dialog_id: DialogId,
            msg_ids: &[MsgId],
        ) -> Result<usize, CoreError> {
            let mut guard = self
                .rows
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            let mut count = 0;
            for id in msg_ids {
                if guard.remove(&(account_id, dialog_id, id.0)).is_some() {
                    count += 1;
                }
            }
            Ok(count)
        }

        fn delete_messages_up_to(
            &self,
            account_id: AccountId,
            dialog_id: DialogId,
            max_msg_id: MsgId,
        ) -> Result<usize, CoreError> {
            let mut guard = self
                .rows
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            let before = guard.len();
            guard.retain(|(acc, dlg, id), _| {
                !(*acc == account_id && *dlg == dialog_id && *id <= max_msg_id.0)
            });
            Ok(before.saturating_sub(guard.len()))
        }

        fn search_dialog_messages(
            &self,
            account_id: AccountId,
            dialog_id: DialogId,
            query: &str,
            limit: usize,
        ) -> Result<Vec<CompactMessage>, CoreError> {
            let guard = self
                .rows
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            Ok(guard
                .range((account_id, dialog_id, i32::MIN)..=(account_id, dialog_id, i32::MAX))
                .filter(|(_, m)| m.text.contains(query))
                .take(limit)
                .map(|(_, m)| m.clone())
                .collect())
        }

        fn dialog_message_count(
            &self,
            account_id: AccountId,
            dialog_id: DialogId,
        ) -> Result<usize, CoreError> {
            let guard = self
                .rows
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            Ok(guard
                .range((account_id, dialog_id, i32::MIN)..=(account_id, dialog_id, i32::MAX))
                .count())
        }
    }

    struct InMemoryMockDialogRepo {
        dialogs: Mutex<Vec<CompactDialog>>,
    }

    impl InMemoryMockDialogRepo {
        fn seeded(count: usize) -> Self {
            let mut rows = Vec::with_capacity(count);
            for i in 0..count {
                rows.push(CompactDialog {
                    dialog_id: DialogId((i + 1) as i64),
                    peer_id: PeerId::from_user(UserId((i + 1) as u64)),
                    title: format!("Dialog #{i}").into_boxed_str(),
                    draft_text: None,
                    top_message_id: MsgId((i + 1) as i32),
                    read_inbox_max_id: MsgId(0),
                    read_outbox_max_id: MsgId(0),
                    last_message_date: 1_700_000_000 + (count - i) as i32,
                    unread_count: (i % 5) as u32,
                    unread_mentions_count: 0,
                    folder_id: 0,
                    flags: DialogFlags::empty(),
                });
            }
            Self {
                dialogs: Mutex::new(rows),
            }
        }
    }

    impl DialogRepository for InMemoryMockDialogRepo {
        fn upsert_dialog(
            &self,
            _account_id: AccountId,
            dialog: &CompactDialog,
        ) -> Result<(), CoreError> {
            let mut guard = self
                .dialogs
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            if let Some(existing) = guard.iter_mut().find(|d| d.dialog_id == dialog.dialog_id) {
                *existing = dialog.clone();
            } else {
                guard.push(dialog.clone());
            }
            Ok(())
        }

        fn upsert_dialog_batch(
            &self,
            account_id: AccountId,
            dialogs: &[CompactDialog],
        ) -> Result<usize, CoreError> {
            for dialog in dialogs {
                self.upsert_dialog(account_id, dialog)?;
            }
            Ok(dialogs.len())
        }

        fn update_dialog_read_state(
            &self,
            _account_id: AccountId,
            dialog_id: DialogId,
            read_inbox_max_id: MsgId,
            unread_count: u32,
        ) -> Result<bool, CoreError> {
            let mut guard = self
                .dialogs
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            if let Some(d) = guard.iter_mut().find(|d| d.dialog_id == dialog_id) {
                d.read_inbox_max_id = read_inbox_max_id;
                d.unread_count = unread_count;
                return Ok(true);
            }
            Ok(false)
        }

        fn remove_dialog(
            &self,
            _account_id: AccountId,
            dialog_id: DialogId,
        ) -> Result<bool, CoreError> {
            let mut guard = self
                .dialogs
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            let before = guard.len();
            guard.retain(|d| d.dialog_id != dialog_id);
            Ok(guard.len() < before)
        }

        fn load_dialog_slice(
            &self,
            _account_id: AccountId,
            folder_id: i32,
            offset: usize,
            limit: usize,
        ) -> Result<Vec<CompactDialog>, CoreError> {
            let guard = self
                .dialogs
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            Ok(guard
                .iter()
                .filter(|d| d.folder_id == folder_id)
                .skip(offset)
                .take(limit)
                .cloned()
                .collect())
        }

        fn search_dialogs(
            &self,
            _account_id: AccountId,
            query: &str,
            limit: usize,
        ) -> Result<Vec<CompactDialog>, CoreError> {
            let guard = self
                .dialogs
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            Ok(guard
                .iter()
                .filter(|d| d.title.contains(query))
                .take(limit)
                .cloned()
                .collect())
        }

        fn dialog_count(&self, _account_id: AccountId, folder_id: i32) -> Result<usize, CoreError> {
            let guard = self
                .dialogs
                .lock()
                .map_err(|_| CoreError::Storage("lock poisoned".to_owned()))?;
            Ok(guard.iter().filter(|d| d.folder_id == folder_id).count())
        }
    }

    #[test]
    fn compact_message_layout_and_alignment() {
        assert!(std::mem::size_of::<CompactMessage>() <= 136);
        assert_eq!(std::mem::size_of::<MessageEntitySpan>(), 16);
        assert_eq!(std::mem::size_of::<PeerId>(), 8);
        assert_eq!(std::mem::size_of::<MsgId>(), 4);
    }

    #[test]
    fn peer_id_encoding_matches_tdesktop() {
        let user_peer = PeerId::from_user(UserId(123_456));
        assert_eq!(user_peer.kind(), PeerKind::User);
        assert_eq!(user_peer.bare_id(), 123_456);

        let channel_peer = PeerId::from_channel(999_888);
        assert_eq!(channel_peer.kind(), PeerKind::Channel);
        assert_eq!(channel_peer.bare_id(), 999_888);
    }

    #[test]
    fn virtualized_window_caps_ram_across_10000_scrolls() -> Result<(), CoreError> {
        let acc = AccountId(1);
        let dlg = DialogId(100);
        let repo = InMemoryMockRepo::seeded(acc, dlg, 2_000);
        let mut window = VirtualizedHistoryWindow::new(repo, acc, dlg, 100)?;

        window.jump_to(MsgId(1_000))?;
        assert_eq!(window.len(), 100);

        for step in 0..5_000 {
            if step % 2 == 0 {
                window.scroll_up(15)?;
            } else {
                window.scroll_down(15)?;
            }
            assert!(window.len() <= 100);
        }

        window.clear_ram();
        assert_eq!(window.len(), 0);
        Ok(())
    }

    #[test]
    fn multi_account_orchestrator_transitions_and_ghost_rules() -> Result<(), CoreError> {
        let mut orchestrator = MultiAccountOrchestrator::new();
        orchestrator.register_account(AccountId(1), UserId(1001), false)?;
        orchestrator.register_account(AccountId(2), UserId(1002), false)?;

        assert_eq!(orchestrator.active_account(), Some(AccountId(1)));
        assert_eq!(
            orchestrator.get(AccountId(1)).map(AccountController::state),
            Some(AccountLifecycleState::ActiveForeground)
        );
        assert_eq!(
            orchestrator.get(AccountId(2)).map(AccountController::state),
            Some(AccountLifecycleState::IdleBackground)
        );

        orchestrator.activate_account(AccountId(2))?;
        assert_eq!(orchestrator.active_account(), Some(AccountId(2)));
        assert_eq!(
            orchestrator.get(AccountId(1)).map(AccountController::state),
            Some(AccountLifecycleState::IdleBackground)
        );

        let ghost = GhostModeConfig::new(
            GhostModeFlags::GHOST_MODE_ACTIVE | GhostModeFlags::SEND_OFFLINE_AFTER_ONLINE,
            SendWithoutSoundOption::InGhostMode,
        );
        orchestrator.get_mut(AccountId(2))?.set_ghost_config(ghost);
        let active_ghost = orchestrator
            .get(AccountId(2))
            .map(AccountController::ghost_config)
            .ok_or(CoreError::AccountNotFound(2))?;
        assert!(!active_ghost.should_send_read_messages());
        assert!(!active_ghost.should_send_online_packets());
        assert!(active_ghost.should_send_without_sound());
        assert!(active_ghost.should_send_offline_after_online());

        Ok(())
    }

    #[test]
    fn mtp_authorization_blob_roundtrip_wide_and_legacy() -> Result<(), CoreError> {
        let mut blob = MtpAuthorizationBlob {
            user_id: UserId(7_000_000_001),
            main_dc_id: 2,
            ..MtpAuthorizationBlob::default()
        };
        blob.keys.push(DcAuthKeyEntry::new(2, [0xAAu8; 256], false, false));
        blob.keys.push(DcAuthKeyEntry::new(4, [0xBBu8; 256], false, false));
        blob.keys_to_destroy
            .push(DcAuthKeyEntry::new(1, [0xCCu8; 256], false, true));

        let encoded = blob.encode_tdesktop();
        let decoded = MtpAuthorizationBlob::decode_tdesktop(&encoded)?;
        assert_eq!(decoded, blob);

        // Legacy 32-bit format: i32 user_id, i32 main_dc_id, 1 key, 0 destroy keys.
        let mut legacy = Vec::new();
        legacy.extend_from_slice(&42_000i32.to_be_bytes());
        legacy.extend_from_slice(&4i32.to_be_bytes());
        legacy.extend_from_slice(&1i32.to_be_bytes());
        legacy.extend_from_slice(&4i32.to_be_bytes());
        legacy.extend_from_slice(&[0x55u8; 256]);
        legacy.extend_from_slice(&0i32.to_be_bytes());

        let decoded_legacy = MtpAuthorizationBlob::decode_tdesktop(&legacy)?;
        assert_eq!(decoded_legacy.user_id, UserId(42_000));
        assert_eq!(decoded_legacy.main_dc_id, 4);
        assert_eq!(decoded_legacy.keys.len(), 1);
        assert_eq!(decoded_legacy.keys[0].key_bytes.as_ref(), &[0x55u8; 256]);
        Ok(())
    }

    #[test]
    fn multi_account_ordering_limits_unread_and_force_logout() -> Result<(), CoreError> {
        let mut orchestrator = MultiAccountOrchestrator::new();
        orchestrator.register_account(AccountId(1), UserId(101), false)?;
        orchestrator.register_account(AccountId(2), UserId(102), true)?;
        orchestrator.register_account(AccountId(3), UserId(0), false)?;

        orchestrator
            .get_mut(AccountId(1))?
            .complete_authorization(UserId(101), 2, true);
        orchestrator
            .get_mut(AccountId(1))?
            .set_unread_state(5, true);
        orchestrator
            .get_mut(AccountId(2))?
            .set_unread_state(3, false);

        assert_eq!(orchestrator.authed_accounts_count(), 2);
        assert_eq!(orchestrator.max_accounts(), MAX_ACCOUNTS_FREE + 2);

        let badge = orchestrator.compute_unread_badge();
        assert_eq!(badge.total_unread, 8);
        assert!(!badge.all_muted);

        let uid2 = orchestrator
            .get(AccountId(2))
            .map(AccountController::session_unique_id)
            .ok_or(CoreError::AccountNotFound(2))?;
        let ordered = orchestrator.ordered_account_ids(&[uid2, 101]);
        assert_eq!(ordered, vec![AccountId(2), AccountId(1), AccountId(3)]);

        let redundant = orchestrator.redundant_account_ids();
        assert_eq!(redundant, vec![AccountId(3)]);

        // Force logout moves active keys into keys_to_destroy and resets authorization.
        let mut blob = MtpAuthorizationBlob {
            user_id: UserId(101),
            main_dc_id: 2,
            ..MtpAuthorizationBlob::default()
        };
        blob.keys.push(DcAuthKeyEntry::new(2, [0x77u8; 256], false, false));
        let ctrl1 = orchestrator.get_mut(AccountId(1))?;
        ctrl1.apply_mtp_authorization(blob);
        ctrl1.force_logout();
        assert!(!ctrl1.is_authorized());
        let exported = ctrl1.export_mtp_authorization();
        assert!(exported.keys.is_empty());
        assert_eq!(exported.keys_to_destroy.len(), 1);
        Ok(())
    }

    #[test]
    fn virtualized_dialog_window_paginates_within_ram_cap() -> Result<(), CoreError> {
        let repo = InMemoryMockDialogRepo::seeded(500);
        let mut window = VirtualizedDialogWindow::new(repo, AccountId(1), 0, 50)?;
        window.load_page(0)?;
        assert_eq!(window.len(), 50);
        assert!(window.estimated_ram_bytes() > 0);

        window.scroll_down(25)?;
        assert_eq!(window.offset(), 25);
        assert!(window.len() <= 50);

        window.scroll_up(25)?;
        assert_eq!(window.offset(), 0);
        assert!(window.len() <= 50);

        window.clear_ram();
        assert_eq!(window.len(), 0);
        Ok(())
    }

    #[test]
    fn filter_engine_evaluates_regex_exclusions_shadow_ban_and_hidden_state() {
        let mut engine = AyuFilterEngine::new();
        let dlg_chat = DialogId(-10_042);
        let dlg_excluded = DialogId(-10_099);

        let rules = vec![
            RegexFilterRule {
                id: Box::from(*b"shared-spam"),
                pattern: r"\bcrypto\s+giveaway\b".into(),
                dialog_id: None,
                enabled: true,
                reversed: false,
                case_insensitive: true,
            },
            RegexFilterRule {
                id: Box::from(*b"dialog-only-rev"),
                pattern: r"\bverified\b".into(),
                dialog_id: Some(dlg_chat),
                enabled: true,
                reversed: true,
                case_insensitive: false,
            },
            RegexFilterRule {
                id: Box::from(*b"broken"),
                pattern: "[unclosed".into(),
                dialog_id: None,
                enabled: true,
                reversed: false,
                case_insensitive: false,
            },
        ];
        let exclusions = vec![RegexFilterGlobalExclusion {
            dialog_id: dlg_excluded,
            filter_id: Box::from(*b"shared-spam"),
        }];

        assert_eq!(engine.rebuild(&rules, &exclusions), 2);

        // Shared case-insensitive filter blocks incoming messages in dlg_chat.
        assert!(engine.evaluate_message(dlg_chat, MsgId(1), 500, false, "Free CRYPTO Giveaway!"));
        assert!(engine.has_filtered_messages(dlg_chat));

        // Outgoing messages are never hidden by content filters.
        assert!(!engine.evaluate_message(dlg_chat, MsgId(2), 1, true, "Free CRYPTO Giveaway!"));

        // Reversed dialog rule blocks messages that do NOT contain "verified".
        assert!(engine.evaluate_message(dlg_chat, MsgId(3), 500, false, "unverified promo"));
        assert!(!engine.evaluate_message(dlg_chat, MsgId(4), 500, false, "verified update"));

        // Global exclusion spares dlg_excluded from "shared-spam".
        assert!(!engine.evaluate_message(dlg_excluded, MsgId(10), 500, false, "Crypto Giveaway"));

        // Shadow-banned sender in group triggers filtering without evaluating regex.
        engine.set_shadow_banned(777, true);
        assert!(engine.is_shadow_banned(777));
        assert!(engine.evaluate_message(dlg_excluded, MsgId(11), 777, false, "verified clean"));

        // Toggling show-filtered reveals messages until toggled back.
        assert!(engine.toggle_filtered_shown(dlg_excluded));
        assert!(!engine.evaluate_message(dlg_excluded, MsgId(12), 777, false, "Crypto Giveaway"));
        assert!(!engine.toggle_filtered_shown(dlg_excluded));

        // Hidden messages always stay hidden even when showing filtered messages.
        engine.hide_message(dlg_excluded, MsgId(13));
        assert!(engine.is_message_hidden(dlg_excluded, MsgId(13)));
        assert!(engine.evaluate_message(dlg_excluded, MsgId(13), 500, false, "anything"));

        // One-shot story ghost auto-disable arming/consumption.
        engine.arm_disable_ghost_on_story_close(AccountId(5));
        assert!(engine.consume_disable_ghost_on_story_close(AccountId(5)));
        assert!(!engine.consume_disable_ghost_on_story_close(AccountId(5)));
    }
}
