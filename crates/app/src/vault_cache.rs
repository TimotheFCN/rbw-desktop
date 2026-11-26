//! The lists of each unlocked account's vault, kept between the views that
//! show them, such as the main window's vault and the menu bar panel, so
//! that one opening shows what another read at once while it reads them
//! again. What an account kept goes when it locks or signs out, and is
//! marked stale when it syncs ([`crate::accounts`] tells this module).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;

use gpui::{App, AppContext as _, Context, Entity, Global};
use rbw_desktop_core::{Account, AccountId, Folder, ItemId, ItemSummary, LockState};

/// What was read of one account's vault.
#[derive(Clone, Debug, Default)]
pub struct VaultSnapshot {
    /// The account's last sync when the lists were read.
    pub last_sync: Option<SystemTime>,
    pub items: Vec<Arc<ItemSummary>>,
    pub folders: Vec<Folder>,
    /// The items used last, most recent first.
    pub recent: Vec<ItemId>,
    /// Whether the vault synced since, so that the lists are to be read
    /// again; until then they still stand in.
    pub stale: bool,
}

pub struct VaultCache {
    accounts: HashMap<AccountId, VaultSnapshot>,
}

struct Shared(Entity<VaultCache>);

impl Global for Shared {}

/// Keeps nothing yet. Call once the backend is set up, before
/// [`crate::accounts::init`].
pub fn init(cx: &mut App) {
    let cache = cx.new(|_| VaultCache {
        accounts: HashMap::new(),
    });
    cx.set_global(Shared(cache));
}

impl VaultCache {
    /// `None` until [`init`].
    pub fn global(cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<Shared>().map(|shared| shared.0.clone())
    }

    /// What was kept of the account's vault, stale or not.
    pub fn get(&self, account: &AccountId) -> Option<&VaultSnapshot> {
        self.accounts.get(account)
    }

    /// Keeps what was just read of the account's vault.
    pub fn put(&mut self, account: AccountId, snapshot: VaultSnapshot, cx: &mut Context<Self>) {
        self.accounts.insert(account, snapshot);
        cx.notify();
    }

    /// Marks what was kept of the account as to be read again.
    pub fn invalidate(&mut self, account: &AccountId, cx: &mut Context<Self>) {
        if let Some(snapshot) = self.accounts.get_mut(account)
            && !snapshot.stale
        {
            snapshot.stale = true;
            cx.notify();
        }
    }

    /// Forgets what was kept of the account, as when it locks.
    pub fn clear_account(&mut self, account: &AccountId, cx: &mut Context<Self>) {
        if self.accounts.remove(account).is_some() {
            cx.notify();
        }
    }

    /// Follows a change of the accounts: an account no longer unlocked
    /// loses what was kept, one that synced since is marked stale.
    fn follow(&mut self, accounts: &[Account], cx: &mut Context<Self>) {
        let mut changed = false;
        self.accounts.retain(|id, _| {
            let unlocked = accounts
                .iter()
                .any(|account| account.id == *id && account.state == LockState::Unlocked);
            changed |= !unlocked;
            unlocked
        });
        for account in accounts {
            if let Some(snapshot) = self.accounts.get_mut(&account.id)
                && !snapshot.stale
                && snapshot.last_sync != account.last_sync
            {
                snapshot.stale = true;
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }
}

/// [`VaultCache::get`] from anywhere: a copy of what was kept, whose items
/// are shared.
pub fn get(account: &AccountId, cx: &App) -> Option<VaultSnapshot> {
    VaultCache::global(cx)?.read(cx).get(account).cloned()
}

/// [`VaultCache::put`] from anywhere.
pub fn put(account: AccountId, snapshot: VaultSnapshot, cx: &mut App) {
    if let Some(cache) = VaultCache::global(cx) {
        cache.update(cx, |cache, cx| cache.put(account, snapshot, cx));
    }
}

/// [`VaultCache::invalidate`] from anywhere.
pub fn invalidate(account: &AccountId, cx: &mut App) {
    if let Some(cache) = VaultCache::global(cx) {
        cache.update(cx, |cache, cx| cache.invalidate(account, cx));
    }
}

/// [`VaultCache::clear_account`] from anywhere.
pub fn clear_account(account: &AccountId, cx: &mut App) {
    if let Some(cache) = VaultCache::global(cx) {
        cache.update(cx, |cache, cx| cache.clear_account(account, cx));
    }
}

/// The accounts changed from `before` to `now`, as [`crate::accounts`]
/// read them.
pub fn follow_accounts(before: &[Account], now: &[Account], cx: &mut App) {
    if before == now {
        return;
    }
    if let Some(cache) = VaultCache::global(cx) {
        cache.update(cx, |cache, cx| cache.follow(now, cx));
    }
}
