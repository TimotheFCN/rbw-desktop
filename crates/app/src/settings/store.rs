//! Each account's settings (lock timeout, Touch ID, SSH confirmation), which
//! several pages show: read once for each account as it appears, written
//! back at once when changed.

use std::collections::HashMap;

use gpui::{Context, Entity, EventEmitter, Subscription};
use rbw_desktop_core::{AccountId, AccountSettings, Error};

use crate::accounts::Accounts;
use crate::backend;

pub struct Store {
    /// `None` while an account's settings are on the way.
    settings: HashMap<AccountId, Option<AccountSettings>>,
    _accounts: Subscription,
}

/// A change could not be saved; the setting is back as it was.
pub struct SaveFailed(pub Error);

impl EventEmitter<SaveFailed> for Store {}

impl Store {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let accounts = Accounts::global(cx);
        let mut this = Self {
            settings: HashMap::new(),
            _accounts: cx.observe(&accounts, |this, accounts, cx| this.follow(&accounts, cx)),
        };
        this.follow(&accounts, cx);
        this
    }

    pub fn get(&self, account: &AccountId) -> Option<&AccountSettings> {
        self.settings.get(account)?.as_ref()
    }

    /// Reads the settings of accounts that appeared, forgets those that went.
    fn follow(&mut self, accounts: &Entity<Accounts>, cx: &mut Context<Self>) {
        let ids: Vec<AccountId> = accounts
            .read(cx)
            .list()
            .iter()
            .map(|account| account.id.clone())
            .collect();
        self.settings.retain(|id, _| ids.contains(id));
        for id in ids {
            if self.settings.contains_key(&id) {
                continue;
            }
            self.settings.insert(id.clone(), None);
            let requested = id.clone();
            backend::request(
                cx,
                move |backend| backend.account_settings(&requested),
                move |this, result, cx| match result {
                    Ok(settings) => {
                        if let Some(slot) = this.settings.get_mut(&id) {
                            *slot = Some(settings);
                            cx.notify();
                        }
                    }
                    Err(error) => log::warn!("reading the settings of {id}: {error}"),
                },
            )
            .detach();
        }
    }

    /// Changes an account's settings now, and saves them.
    pub fn update(
        &mut self,
        account: &AccountId,
        change: impl FnOnce(&mut AccountSettings),
        cx: &mut Context<Self>,
    ) {
        let Some(Some(settings)) = self.settings.get_mut(account) else {
            return;
        };
        let before = settings.clone();
        change(settings);
        if *settings == before {
            return;
        }
        let (id, saved) = (account.clone(), settings.clone());
        let restored = account.clone();
        cx.notify();
        backend::request(
            cx,
            move |backend| backend.set_account_settings(&id, &saved),
            move |this, result, cx| {
                if let Err(error) = result {
                    this.settings.insert(restored, Some(before));
                    cx.emit(SaveFailed(error));
                    cx.notify();
                }
            },
        )
        .detach();
    }
}
