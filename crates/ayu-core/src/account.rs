use std::collections::BTreeMap;

use crate::auth::{AuthState, MtpAuthorizationBlob};
use crate::error::CoreError;
use crate::ids::{AccountId, UserId};
use crate::models::GhostModeConfig;

pub const MAX_ACCOUNTS_FREE: usize = 100;
pub const MAX_ACCOUNTS_PREMIUM: usize = 200;
pub const TEST_DC_SHIFT: u64 = 0x0100_0000_0000_0000;

/// Aggregated unread counter across all authorized accounts.
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq)]
pub struct UnreadBadgeSummary {
    pub total_unread: u32,
    pub all_muted: bool,
}

/// Lifecycle and network readiness state for a single account.
#[repr(u8)]
#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, Hash)]
pub enum AccountLifecycleState {
    #[default]
    IdleBackground = 0,
    ActiveForeground = 1,
    Connecting = 2,
    Unauthorized = 3,
}

/// Encapsulated per-account controller managing identity, authorization, Ghost Mode, and RAM policy.
#[derive(Clone, Debug)]
pub struct AccountController {
    account_id: AccountId,
    index: i32,
    user_id: UserId,
    main_dc_id: i32,
    state: AccountLifecycleState,
    auth_state: AuthState,
    ghost_config: GhostModeConfig,
    unread_count: u32,
    unread_muted: bool,
    is_test_dc: bool,
    is_premium: bool,
    has_separate_window: bool,
    online_marked: bool,
    pending_login_code: Option<Box<str>>,
    auth_blob: Option<MtpAuthorizationBlob>,
}

impl AccountController {
    #[must_use]
    pub fn new(account_id: AccountId, user_id: UserId, is_test_dc: bool) -> Self {
        Self::with_index(account_id, 0, user_id, is_test_dc)
    }

    #[must_use]
    pub fn with_index(
        account_id: AccountId,
        index: i32,
        user_id: UserId,
        is_test_dc: bool,
    ) -> Self {
        let authorized = user_id.0 != 0;
        Self {
            account_id,
            index,
            user_id,
            main_dc_id: 2,
            state: if authorized {
                AccountLifecycleState::IdleBackground
            } else {
                AccountLifecycleState::Unauthorized
            },
            auth_state: if authorized {
                AuthState::Authorized
            } else {
                AuthState::Unauthorized
            },
            ghost_config: GhostModeConfig::default(),
            unread_count: 0,
            unread_muted: true,
            is_test_dc,
            is_premium: false,
            has_separate_window: false,
            online_marked: false,
            pending_login_code: None,
            auth_blob: None,
        }
    }

    #[must_use]
    pub const fn account_id(&self) -> AccountId {
        self.account_id
    }

    #[must_use]
    pub const fn index(&self) -> i32 {
        self.index
    }

    pub fn set_index(&mut self, index: i32) {
        self.index = index;
    }

    #[must_use]
    pub const fn user_id(&self) -> UserId {
        self.user_id
    }

    #[must_use]
    pub const fn main_dc_id(&self) -> i32 {
        self.main_dc_id
    }

    #[must_use]
    pub const fn is_test_dc(&self) -> bool {
        self.is_test_dc
    }

    #[must_use]
    pub const fn is_premium(&self) -> bool {
        self.is_premium
    }

    #[must_use]
    pub const fn state(&self) -> AccountLifecycleState {
        self.state
    }

    #[must_use]
    pub const fn auth_state(&self) -> AuthState {
        self.auth_state
    }

    #[must_use]
    pub const fn session_unique_id(&self) -> u64 {
        if self.user_id.0 == 0 {
            0
        } else if self.is_test_dc {
            self.user_id.0 | TEST_DC_SHIFT
        } else {
            self.user_id.0
        }
    }

    #[must_use]
    pub const fn is_authorized(&self) -> bool {
        matches!(self.auth_state, AuthState::Authorized) && self.user_id.0 != 0
    }

    #[must_use]
    pub const fn has_separate_window(&self) -> bool {
        self.has_separate_window
    }

    pub fn set_separate_window(&mut self, has_window: bool) {
        self.has_separate_window = has_window;
    }

    #[must_use]
    pub const fn ghost_config(&self) -> GhostModeConfig {
        self.ghost_config
    }

    pub fn set_ghost_config(&mut self, config: GhostModeConfig) {
        self.ghost_config = config;
    }

    pub fn set_state(&mut self, state: AccountLifecycleState) {
        self.state = state;
    }

    #[must_use]
    pub const fn unread_count(&self) -> u32 {
        self.unread_count
    }

    #[must_use]
    pub const fn unread_muted(&self) -> bool {
        self.unread_muted
    }

    pub fn set_unread_count(&mut self, count: u32) {
        self.unread_count = count;
    }

    pub fn set_unread_state(&mut self, unread_count: u32, muted: bool) {
        self.unread_count = unread_count;
        self.unread_muted = muted;
    }

    pub fn apply_mtp_authorization(&mut self, blob: MtpAuthorizationBlob) {
        if blob.user_id.0 != 0 {
            self.user_id = blob.user_id;
            self.auth_state = AuthState::Authorized;
            if self.state == AccountLifecycleState::Unauthorized {
                self.state = AccountLifecycleState::IdleBackground;
            }
        }
        if blob.main_dc_id > 0 {
            self.main_dc_id = blob.main_dc_id;
        }
        self.auth_blob = Some(blob);
    }

    #[must_use]
    pub fn export_mtp_authorization(&self) -> MtpAuthorizationBlob {
        match &self.auth_blob {
            Some(blob) => {
                let mut cloned = blob.clone();
                cloned.user_id = self.user_id;
                cloned.main_dc_id = self.main_dc_id;
                cloned
            }
            None => MtpAuthorizationBlob {
                user_id: self.user_id,
                main_dc_id: self.main_dc_id,
                ..MtpAuthorizationBlob::default()
            },
        }
    }

    pub fn complete_authorization(&mut self, user_id: UserId, main_dc_id: i32, is_premium: bool) {
        self.user_id = user_id;
        if main_dc_id > 0 {
            self.main_dc_id = main_dc_id;
        }
        self.is_premium = is_premium;
        self.auth_state = if user_id.0 != 0 {
            AuthState::Authorized
        } else {
            AuthState::Unauthorized
        };
        if self.state == AccountLifecycleState::Unauthorized && user_id.0 != 0 {
            self.state = AccountLifecycleState::IdleBackground;
        }
        self.pending_login_code = None;
        if let Some(ref mut blob) = self.auth_blob {
            blob.user_id = user_id;
            blob.main_dc_id = self.main_dc_id;
        }
    }

    pub fn begin_logout(&mut self) {
        self.auth_state = AuthState::LoggingOut;
    }

    pub fn finish_logout(&mut self) {
        self.user_id = UserId(0);
        self.is_premium = false;
        self.unread_count = 0;
        self.unread_muted = true;
        self.online_marked = false;
        self.pending_login_code = None;
        self.auth_state = AuthState::Unauthorized;
        self.state = AccountLifecycleState::Unauthorized;
        self.auth_blob = None;
    }

    /// Moves all active DC keys into `keys_to_destroy` and resets session state.
    pub fn force_logout(&mut self) {
        if let Some(ref mut blob) = self.auth_blob {
            for mut key in blob.keys.drain(..) {
                key.to_destroy = true;
                blob.keys_to_destroy.push(key);
            }
            blob.user_id = UserId(0);
        }
        self.user_id = UserId(0);
        self.is_premium = false;
        self.unread_count = 0;
        self.unread_muted = true;
        self.online_marked = false;
        self.pending_login_code = None;
        self.auth_state = AuthState::Unauthorized;
        self.state = AccountLifecycleState::Unauthorized;
    }

    pub fn push_login_code(&mut self, code: &str) {
        self.pending_login_code = Some(code.into());
        if self.auth_state == AuthState::Unauthorized || self.auth_state == AuthState::WaitingForPhone {
            self.auth_state = AuthState::WaitingForCode;
        }
    }

    pub fn take_login_code(&mut self) -> Option<Box<str>> {
        self.pending_login_code.take()
    }

    pub fn mark_online(&mut self) {
        self.online_marked = true;
    }

    pub fn consume_online_mark(&mut self) -> bool {
        std::mem::replace(&mut self.online_marked, false)
    }

    #[must_use]
    pub const fn auth_blob(&self) -> Option<&MtpAuthorizationBlob> {
        self.auth_blob.as_ref()
    }
}

/// Multi-account orchestrator coordinating active foreground focus, authorization, and RAM eviction.
#[derive(Debug, Default)]
pub struct MultiAccountOrchestrator {
    accounts: BTreeMap<AccountId, AccountController>,
    active_account: Option<AccountId>,
    last_active_account: Option<AccountId>,
}

impl MultiAccountOrchestrator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_account(
        &mut self,
        account_id: AccountId,
        user_id: UserId,
        is_test_dc: bool,
    ) -> Result<(), CoreError> {
        if self.accounts.contains_key(&account_id) {
            return Err(CoreError::AccountAlreadyExists(account_id.0));
        }
        let max_allowed = self.max_accounts();
        if self.accounts.len() >= max_allowed {
            return Err(CoreError::MaxAccountsReached { max: max_allowed });
        }
        let index = self.accounts.len() as i32;
        let mut controller = AccountController::with_index(account_id, index, user_id, is_test_dc);
        if self.active_account.is_none() {
            controller.set_state(AccountLifecycleState::ActiveForeground);
            self.active_account = Some(account_id);
        }
        self.accounts.insert(account_id, controller);
        Ok(())
    }

    pub fn activate_account(&mut self, account_id: AccountId) -> Result<(), CoreError> {
        if !self.accounts.contains_key(&account_id) {
            return Err(CoreError::AccountNotFound(account_id.0));
        }
        if self.active_account == Some(account_id) {
            return Ok(());
        }
        if let Some(prev_id) = self.active_account {
            self.last_active_account = Some(prev_id);
            if let Some(prev) = self.accounts.get_mut(&prev_id) {
                if prev.state() == AccountLifecycleState::ActiveForeground {
                    prev.set_state(if prev.is_authorized() {
                        AccountLifecycleState::IdleBackground
                    } else {
                        AccountLifecycleState::Unauthorized
                    });
                }
            }
        }
        if let Some(next) = self.accounts.get_mut(&account_id) {
            next.set_state(AccountLifecycleState::ActiveForeground);
        }
        self.active_account = Some(account_id);
        Ok(())
    }

    pub fn remove_account(&mut self, account_id: AccountId) -> Result<AccountController, CoreError> {
        let removed = self
            .accounts
            .remove(&account_id)
            .ok_or(CoreError::AccountNotFound(account_id.0))?;
        if self.last_active_account == Some(account_id) {
            self.last_active_account = None;
        }
        if self.active_account == Some(account_id) {
            self.active_account = self
                .maybe_last_or_some_authed_account()
                .or_else(|| self.accounts.keys().next().copied());
            if let Some(new_active) = self.active_account {
                if let Some(ctrl) = self.accounts.get_mut(&new_active) {
                    ctrl.set_state(AccountLifecycleState::ActiveForeground);
                }
            }
        }
        Ok(removed)
    }

    #[must_use]
    pub fn active_account(&self) -> Option<AccountId> {
        self.active_account
    }

    #[must_use]
    pub fn account_count(&self) -> usize {
        self.accounts.len()
    }

    #[must_use]
    pub fn get(&self, account_id: AccountId) -> Option<&AccountController> {
        self.accounts.get(&account_id)
    }

    pub fn get_mut(&mut self, account_id: AccountId) -> Result<&mut AccountController, CoreError> {
        self.accounts
            .get_mut(&account_id)
            .ok_or(CoreError::AccountNotFound(account_id.0))
    }

    pub fn account_ids(&self) -> impl Iterator<Item = AccountId> + '_ {
        self.accounts.keys().copied()
    }

    #[must_use]
    pub fn max_accounts(&self) -> usize {
        let boosted = self
            .accounts
            .values()
            .filter(|a| a.is_authorized() && (a.is_premium() || a.is_test_dc()))
            .count();
        (MAX_ACCOUNTS_FREE + boosted).min(MAX_ACCOUNTS_PREMIUM)
    }

    #[must_use]
    pub fn authed_accounts_count(&self) -> usize {
        self.accounts.values().filter(|a| a.is_authorized()).count()
    }

    #[must_use]
    pub fn maybe_last_or_some_authed_account(&self) -> Option<AccountId> {
        if let Some(last_id) = self.last_active_account {
            if let Some(acc) = self.accounts.get(&last_id) {
                if acc.is_authorized() {
                    return Some(last_id);
                }
            }
        }
        self.accounts
            .values()
            .find(|a| a.is_authorized())
            .map(AccountController::account_id)
    }

    #[must_use]
    pub fn ordered_account_ids(&self, order_unique_ids: &[u64]) -> Vec<AccountId> {
        let mut list: Vec<&AccountController> = self.accounts.values().collect();
        list.sort_by_key(|acc| {
            if !acc.is_authorized() {
                return usize::MAX;
            }
            let uid = acc.session_unique_id();
            order_unique_ids
                .iter()
                .position(|&x| x == uid)
                .unwrap_or(order_unique_ids.len())
        });
        list.into_iter().map(AccountController::account_id).collect()
    }

    #[must_use]
    pub fn redundant_account_ids(&self) -> Vec<AccountId> {
        let mut redundant: Vec<AccountId> = self
            .accounts
            .values()
            .filter(|acc| {
                !acc.is_authorized()
                    && !acc.has_separate_window()
                    && acc.auth_state() != AuthState::LoggingOut
            })
            .map(AccountController::account_id)
            .collect();

        if redundant.len() == self.accounts.len() && !redundant.is_empty() {
            if let Some(keep_id) = self.active_account.or_else(|| redundant.first().copied()) {
                redundant.retain(|&id| id != keep_id);
            }
        }
        redundant
    }

    #[must_use]
    pub fn compute_unread_badge(&self) -> UnreadBadgeSummary {
        let mut total_unread: u32 = 0;
        let mut all_muted = true;
        for acc in self.accounts.values() {
            if acc.is_authorized() {
                total_unread = total_unread.saturating_add(acc.unread_count());
                if acc.unread_count() > 0 && !acc.unread_muted() {
                    all_muted = false;
                }
            }
        }
        UnreadBadgeSummary {
            total_unread,
            all_muted,
        }
    }

    #[must_use]
    pub fn total_unread_count(&self) -> u64 {
        u64::from(self.compute_unread_badge().total_unread)
    }

    pub fn collect_accounts_for_offline_packet(&mut self) -> Vec<AccountId> {
        let mut out = Vec::new();
        for acc in self.accounts.values_mut() {
            if acc.is_authorized()
                && acc.ghost_config().should_send_offline_after_online()
                && acc.consume_online_mark()
            {
                out.push(acc.account_id());
            }
        }
        out
    }
}
