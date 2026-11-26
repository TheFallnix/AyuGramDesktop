use std::collections::{HashMap, HashSet};

use regex::{Regex, RegexBuilder};

use crate::ids::{AccountId, DialogId, MsgId};
use crate::models::{RegexFilterGlobalExclusion, RegexFilterRule};

pub const MAX_FILTER_EVAL_CACHE_ENTRIES: usize = 4096;

struct CompiledPattern {
    id: Box<[u8]>,
    regex: Regex,
    reversed: bool,
}

impl CompiledPattern {
    fn matches(&self, text: &str) -> bool {
        let matched = self.regex.is_match(text);
        if self.reversed {
            !matched
        } else {
            matched
        }
    }
}

/// Coordinates compiled regex filters, per-dialog exclusions, shadow bans, hidden messages,
/// and per-story temporary ghost mode state with bounded RAM usage.
#[derive(Default)]
pub struct AyuFilterEngine {
    shared_patterns: Vec<CompiledPattern>,
    dialog_patterns: HashMap<DialogId, Vec<CompiledPattern>>,
    exclusions_by_dialog: HashMap<DialogId, HashSet<Box<[u8]>>>,
    shadow_banned_peers: HashSet<i64>,
    showing_filtered_dialogs: HashSet<DialogId>,
    eval_cache: HashMap<(DialogId, MsgId), bool>,
    dialogs_with_filtered: HashSet<DialogId>,
    hidden_messages: HashMap<DialogId, HashSet<MsgId>>,
    story_ghost_armed: HashSet<AccountId>,
    filters_enabled: bool,
    filters_in_chats: bool,
    hide_from_blocked: bool,
}

impl AyuFilterEngine {
    #[must_use]
    pub fn new() -> Self {
        Self {
            filters_enabled: true,
            filters_in_chats: true,
            hide_from_blocked: false,
            ..Self::default()
        }
    }

    pub fn set_filters_enabled(
        &mut self,
        enabled: bool,
        enabled_in_chats: bool,
        hide_from_blocked: bool,
    ) {
        self.filters_enabled = enabled;
        self.filters_in_chats = enabled_in_chats;
        self.hide_from_blocked = hide_from_blocked;
    }

    #[must_use]
    pub const fn filters_enabled(&self) -> bool {
        self.filters_enabled
    }

    #[must_use]
    pub const fn filters_in_chats(&self) -> bool {
        self.filters_in_chats
    }

    #[must_use]
    pub const fn hide_from_blocked(&self) -> bool {
        self.hide_from_blocked
    }

    pub fn validate_pattern(pattern: &str) -> Result<(), String> {
        RegexBuilder::new(pattern)
            .multi_line(true)
            .build()
            .map(|_| ())
            .map_err(|err| err.to_string())
    }

    pub fn set_shadow_banned(&mut self, peer_dialog_id: i64, banned: bool) {
        if banned {
            self.shadow_banned_peers.insert(peer_dialog_id);
        } else {
            self.shadow_banned_peers.remove(&peer_dialog_id);
        }
    }

    #[must_use]
    pub fn is_shadow_banned(&self, peer_dialog_id: i64) -> bool {
        self.shadow_banned_peers.contains(&peer_dialog_id)
    }

    pub fn toggle_filtered_shown(&mut self, dialog_id: DialogId) -> bool {
        if !self.showing_filtered_dialogs.remove(&dialog_id) {
            self.showing_filtered_dialogs.insert(dialog_id);
            true
        } else {
            false
        }
    }

    #[must_use]
    pub fn filtered_messages_shown(&self, dialog_id: DialogId) -> bool {
        self.showing_filtered_dialogs.contains(&dialog_id)
    }

    pub fn hide_message(&mut self, dialog_id: DialogId, msg_id: MsgId) {
        self.hidden_messages
            .entry(dialog_id)
            .or_default()
            .insert(msg_id);
    }

    #[must_use]
    pub fn is_message_hidden(&self, dialog_id: DialogId, msg_id: MsgId) -> bool {
        self.hidden_messages
            .get(&dialog_id)
            .is_some_and(|set| set.contains(&msg_id))
    }

    pub fn clear_dialog_hidden(&mut self, dialog_id: DialogId) {
        self.hidden_messages.remove(&dialog_id);
    }

    pub fn arm_disable_ghost_on_story_close(&mut self, account_id: AccountId) {
        self.story_ghost_armed.insert(account_id);
    }

    pub fn consume_disable_ghost_on_story_close(&mut self, account_id: AccountId) -> bool {
        self.story_ghost_armed.remove(&account_id)
    }

    pub fn rebuild(
        &mut self,
        rules: &[RegexFilterRule],
        exclusions: &[RegexFilterGlobalExclusion],
    ) -> usize {
        let mut next_shared = Vec::new();
        let mut next_by_dialog: HashMap<DialogId, Vec<CompiledPattern>> = HashMap::new();
        let mut compiled_count = 0usize;

        for rule in rules {
            if !rule.enabled || rule.pattern.is_empty() {
                continue;
            }
            let Ok(regex) = RegexBuilder::new(&rule.pattern)
                .multi_line(true)
                .case_insensitive(rule.case_insensitive)
                .build()
            else {
                continue;
            };
            compiled_count = compiled_count.saturating_add(1);
            let entry = CompiledPattern {
                id: rule.id.clone(),
                regex,
                reversed: rule.reversed,
            };
            if let Some(dialog_id) = rule.dialog_id {
                next_by_dialog.entry(dialog_id).or_default().push(entry);
            } else {
                next_shared.push(entry);
            }
        }

        let mut next_exclusions: HashMap<DialogId, HashSet<Box<[u8]>>> = HashMap::new();
        for ex in exclusions {
            next_exclusions
                .entry(ex.dialog_id)
                .or_default()
                .insert(ex.filter_id.clone());
        }

        self.shared_patterns = next_shared;
        self.dialog_patterns = next_by_dialog;
        self.exclusions_by_dialog = next_exclusions;
        self.eval_cache.clear();
        self.dialogs_with_filtered.clear();

        compiled_count
    }

    pub fn evaluate_message(
        &mut self,
        dialog_id: DialogId,
        msg_id: MsgId,
        sender_dialog_id: i64,
        is_outgoing: bool,
        text: &str,
    ) -> bool {
        if self.is_message_hidden(dialog_id, msg_id) {
            return true;
        }
        if is_outgoing
            || !self.filters_enabled
            || self.showing_filtered_dialogs.contains(&dialog_id)
        {
            return false;
        }
        if sender_dialog_id != 0
            && sender_dialog_id != dialog_id.0
            && self.shadow_banned_peers.contains(&sender_dialog_id)
        {
            self.dialogs_with_filtered.insert(dialog_id);
            return true;
        }
        if let Some(&cached) = self.eval_cache.get(&(dialog_id, msg_id)) {
            return cached;
        }
        if text.is_empty() {
            return false;
        }

        let mut matched = false;
        if let Some(patterns) = self.dialog_patterns.get(&dialog_id) {
            for pattern in patterns {
                if pattern.matches(text) {
                    matched = true;
                    break;
                }
            }
        }
        if !matched && !self.shared_patterns.is_empty() {
            let excluded = self.exclusions_by_dialog.get(&dialog_id);
            for pattern in &self.shared_patterns {
                if excluded.is_some_and(|set| set.contains(&pattern.id)) {
                    continue;
                }
                if pattern.matches(text) {
                    matched = true;
                    break;
                }
            }
        }

        if self.eval_cache.len() >= MAX_FILTER_EVAL_CACHE_ENTRIES {
            self.eval_cache.clear();
        }
        self.eval_cache.insert((dialog_id, msg_id), matched);
        if matched {
            self.dialogs_with_filtered.insert(dialog_id);
        }
        matched
    }

    pub fn invalidate_message(&mut self, dialog_id: DialogId, msg_id: MsgId) {
        self.eval_cache.remove(&(dialog_id, msg_id));
    }

    #[must_use]
    pub fn has_filtered_messages(&self, dialog_id: DialogId) -> bool {
        self.dialogs_with_filtered.contains(&dialog_id)
    }
}
