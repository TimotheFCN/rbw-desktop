//! Each account's settings (lock timeout, Touch ID, SSH confirmation), which
//! several pages show: read once for each account as it appears, written
//! back at once when changed.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{Context, Entity, EventEmitter, Subscription, Task};
use rbw_desktop_core::{AccountId, AccountSettings, Error};

use crate::accounts::Accounts;
use crate::backend;

/// How long after failing to read an account's settings they are read
/// again, and how many times in a row.
const RETRY_AFTER: Duration = Duration::from_secs(3);
const RETRIES: u32 = 5;

pub struct Store {
    entries: HashMap<AccountId, Entry>,
    /// Counts the entries made, so that an answer for an account that went
    /// and came back goes nowhere.
    made: u64,
    /// The reads that failed in a row.
    failures: u32,
    retry: Task<()>,
    _accounts: Subscription,
}

/// One account's settings.
struct Entry {
    /// Which entry this is, of [`Store::made`].
    id: u64,
    /// `None` while they are on the way.
    settings: Option<AccountSettings>,
    /// The settings as last read or saved: what a failed save puts back.
    saved: Option<AccountSettings>,
    /// Whether a save is on the way.
    saving: bool,
    /// Whether they changed while a save was on the way.
    save_again: bool,
}

/// A change could not be saved; the setting is back as it was.
pub struct SaveFailed(pub Error);

impl EventEmitter<SaveFailed> for Store {}

impl Store {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let accounts = Accounts::global(cx);
        let mut this = Self {
            entries: HashMap::new(),
            made: 0,
            failures: 0,
            retry: Task::ready(()),
            _accounts: cx.observe(&accounts, |this, accounts, cx| this.follow(&accounts, cx)),
        };
        this.follow(&accounts, cx);
        this
    }

    pub fn get(&self, account: &AccountId) -> Option<&AccountSettings> {
        self.entries.get(account)?.settings.as_ref()
    }

    /// Reads the settings of accounts that appeared, forgets those that went.
    fn follow(&mut self, accounts: &Entity<Accounts>, cx: &mut Context<Self>) {
        let ids: Vec<AccountId> = accounts
            .read(cx)
            .list()
            .iter()
            .map(|account| account.id.clone())
            .collect();
        let before = self.entries.len();
        self.entries.retain(|id, _| ids.contains(id));
        if self.entries.len() != before {
            cx.notify();
        }
        for id in ids {
            if self.entries.contains_key(&id) {
                continue;
            }
            self.made += 1;
            let entry = self.made;
            self.entries.insert(
                id.clone(),
                Entry {
                    id: entry,
                    settings: None,
                    saved: None,
                    saving: false,
                    save_again: false,
                },
            );
            let requested = id.clone();
            backend::request(
                cx,
                move |backend| backend.account_settings(&requested),
                move |this, result, cx| {
                    let Some(slot) = this.entries.get_mut(&id).filter(|slot| slot.id == entry)
                    else {
                        return;
                    };
                    match result {
                        Ok(settings) => {
                            slot.saved = Some(settings.clone());
                            slot.settings = Some(settings);
                            this.failures = 0;
                            cx.notify();
                        }
                        // Read again in a moment, rather than leave the
                        // account's settings out until Settings opens again.
                        Err(error) => {
                            log::warn!("reading the settings of {id}: {error}");
                            this.entries.remove(&id);
                            this.retry_later(cx);
                        }
                    }
                },
            )
            .detach();
        }
    }

    fn retry_later(&mut self, cx: &mut Context<Self>) {
        self.failures += 1;
        if self.failures > RETRIES {
            return;
        }
        self.retry = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(RETRY_AFTER).await;
            this.update(cx, |this, cx| {
                let accounts = Accounts::global(cx);
                this.follow(&accounts, cx);
            })
            .ok();
        });
    }

    /// Changes an account's settings now, and saves them.
    pub fn update(
        &mut self,
        account: &AccountId,
        change: impl FnOnce(&mut AccountSettings),
        cx: &mut Context<Self>,
    ) {
        let Some(Entry {
            settings: Some(settings),
            ..
        }) = self.entries.get_mut(account)
        else {
            return;
        };
        let before = settings.clone();
        change(settings);
        if *settings == before {
            return;
        }
        cx.notify();
        self.save(account, cx);
    }

    /// Saves an account's settings, after the save on the way for it if
    /// there is one. A save that fails puts back the settings saved before,
    /// unless they changed again since: the newer ones are saved next.
    fn save(&mut self, account: &AccountId, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get_mut(account) else {
            return;
        };
        let Some(saving) = entry.settings.clone() else {
            return;
        };
        if entry.saving {
            entry.save_again = true;
            return;
        }
        entry.saving = true;
        let made = entry.id;
        let (id, sent) = (account.clone(), saving.clone());
        let answered = account.clone();
        backend::request(
            cx,
            move |backend| backend.set_account_settings(&id, &sent),
            move |this, result, cx| {
                // An account removed meanwhile stays removed.
                let Some(entry) = this
                    .entries
                    .get_mut(&answered)
                    .filter(|entry| entry.id == made)
                else {
                    return;
                };
                entry.saving = false;
                let again = std::mem::take(&mut entry.save_again);
                match result {
                    Ok(()) => entry.saved = Some(saving),
                    Err(error) => {
                        log::warn!("saving the settings of {answered}: {error}");
                        if !again && entry.settings.as_ref() == Some(&saving) {
                            entry.settings = entry.saved.clone();
                            cx.emit(SaveFailed(error));
                            cx.notify();
                        }
                    }
                }
                if again && entry.settings != entry.saved {
                    this.save(&answered, cx);
                }
            },
        )
        .detach();
    }
}
