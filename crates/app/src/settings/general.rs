//! General: how long each account stays unlocked without being used, when
//! a copied secret leaves the clipboard, and whether websites' icons show.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render, Styled as _,
    Subscription, Window, div, px,
};
use rbw_desktop_core::{AccountId, Preferences};
use rbw_desktop_ui::{Select, Selected, setting_row, settings_group, switch};

use super::store::Store;
use crate::accounts::{self, Accounts};
use crate::backend;
use crate::clipboard::ClearAfter;
use crate::icons::Icons;

const LOCK_AFTER: [u64; 7] = [60, 5 * 60, 15 * 60, 30 * 60, 3600, 4 * 3600, 24 * 3600];
const CLEAR_AFTER: [Option<u64>; 6] = [Some(10), Some(30), Some(60), Some(120), Some(300), None];

pub struct GeneralPage {
    store: Entity<Store>,
    /// `None` until read.
    preferences: Option<Preferences>,
    clear_after: Entity<Select>,
    /// For each account, how long it stays unlocked, with the choices it
    /// offers: the usual ones and, if it is set to something else, that.
    lock_after: HashMap<AccountId, (Entity<Select>, Vec<Duration>)>,
    _subscriptions: Vec<Subscription>,
}

impl GeneralPage {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let defaults = Preferences::default();
        let clear_after = cx.new(|cx| {
            Select::new(
                "Clear copied passwords after",
                CLEAR_AFTER.map(clear_label),
                clear_index(defaults.clipboard_clear),
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe(&clear_after, |this, _, Selected(index), cx| {
                let clear = CLEAR_AFTER[*index].map(Duration::from_secs);
                this.save(|preferences| preferences.clipboard_clear = clear, cx);
            }),
            cx.observe(&store, |this, _, cx| this.follow_store(cx)),
            cx.observe(&Accounts::global(cx), |_, _, cx| cx.notify()),
        ];
        backend::request(
            cx,
            |backend| backend.preferences(),
            |this, result, cx| match result {
                Ok(preferences) => {
                    this.clear_after.update(cx, |select, cx| {
                        select.set_selected(clear_index(preferences.clipboard_clear), cx)
                    });
                    this.preferences = Some(preferences);
                    cx.notify();
                }
                Err(error) => log::warn!("reading the preferences: {error}"),
            },
        )
        .detach();
        let mut this = Self {
            store,
            preferences: None,
            clear_after,
            lock_after: HashMap::new(),
            _subscriptions: subscriptions,
        };
        this.follow_store(cx);
        this
    }

    /// Keeps a lock timeout menu for each account whose settings are known.
    fn follow_store(&mut self, cx: &mut Context<Self>) {
        let accounts: Vec<AccountId> = Accounts::global(cx)
            .read(cx)
            .list()
            .iter()
            .map(|account| account.id.clone())
            .collect();
        self.lock_after.retain(|id, _| accounts.contains(id));
        for id in accounts {
            let Some(timeout) = self.store.read(cx).get(&id).map(|s| s.lock_timeout) else {
                continue;
            };
            let mut choices: Vec<Duration> = LOCK_AFTER
                .iter()
                .copied()
                .map(Duration::from_secs)
                .collect();
            if !choices.contains(&timeout) {
                choices.push(timeout);
            }
            let selected = choices
                .iter()
                .position(|d| *d == timeout)
                .unwrap_or_default();
            let labels: Vec<String> = choices.iter().copied().map(duration_label).collect();
            match self.lock_after.get_mut(&id) {
                Some((select, known)) => {
                    if *known != choices || select.read(cx).selected() != selected {
                        select.update(cx, |select, cx| select.set_options(labels, selected, cx));
                        *known = choices;
                    }
                }
                None => {
                    let select = cx.new(|cx| Select::new("Lock after", labels, selected, cx));
                    let account = id.clone();
                    self._subscriptions.push(cx.subscribe(
                        &select,
                        move |this, _, Selected(index), cx| {
                            let Some(timeout) = this
                                .lock_after
                                .get(&account)
                                .and_then(|(_, choices)| choices.get(*index).copied())
                            else {
                                return;
                            };
                            this.store.update(cx, |store, cx| {
                                store.update(&account, |s| s.lock_timeout = timeout, cx)
                            });
                        },
                    ));
                    self.lock_after.insert(id, (select, choices));
                }
            }
        }
        cx.notify();
    }

    /// Changes the preferences now and saves them; a failure puts them back.
    fn save(&mut self, change: impl FnOnce(&mut Preferences), cx: &mut Context<Self>) {
        let Some(preferences) = &mut self.preferences else {
            return;
        };
        let before = preferences.clone();
        change(preferences);
        let saved = preferences.clone();
        apply(&saved, cx);
        cx.notify();
        backend::request(
            cx,
            move |backend| backend.set_preferences(&saved),
            move |this, result, cx| {
                if let Err(error) = result {
                    log::warn!("saving the preferences: {error}");
                    apply(&before, cx);
                    this.clear_after.update(cx, |select, cx| {
                        select.set_selected(clear_index(before.clipboard_clear), cx)
                    });
                    this.preferences = Some(before);
                    cx.notify();
                }
            },
        )
        .detach();
    }
}

/// Makes the rest of the app follow the preferences.
fn apply(preferences: &Preferences, cx: &mut App) {
    cx.set_global(ClearAfter(preferences.clipboard_clear));
    let website_icons = preferences.website_icons;
    Icons::global(cx).update(cx, |icons, cx| icons.set_enabled(website_icons, cx));
}

fn clear_index(clear: Option<Duration>) -> usize {
    let seconds = clear.map(|clear| clear.as_secs());
    CLEAR_AFTER
        .iter()
        .position(|choice| *choice == seconds)
        // something else set by hand reads as the nearest longer choice
        .or_else(|| {
            CLEAR_AFTER
                .iter()
                .position(|choice| choice.zip(seconds).is_some_and(|(c, s)| c >= s))
        })
        .unwrap_or(CLEAR_AFTER.len() - 1)
}

fn clear_label(seconds: Option<u64>) -> String {
    seconds.map_or_else(
        || "Never".to_string(),
        |seconds| duration_label(Duration::from_secs(seconds)),
    )
}

/// A duration as people say it: "30 seconds", "1 minute", "4 hours".
fn duration_label(duration: Duration) -> String {
    let seconds = duration.as_secs();
    let (count, unit) = match seconds {
        s if s % 86_400 == 0 => (s / 86_400, "day"),
        s if s % 3600 == 0 => (s / 3600, "hour"),
        s if s % 60 == 0 => (s / 60, "minute"),
        s => (s, "second"),
    };
    if count == 1 {
        format!("1 {unit}")
    } else {
        format!("{count} {unit}s")
    }
}

impl Render for GeneralPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let accounts = Accounts::global(cx).read(cx).list().to_vec();
        let several = accounts.len() > 1;
        let lock_rows: Vec<_> = accounts
            .iter()
            .filter_map(|account| {
                let (select, _) = self.lock_after.get(&account.id)?;
                Some(if several {
                    setting_row(accounts::name(account), None, select.clone(), cx)
                } else {
                    setting_row("Lock after", None, select.clone(), cx)
                })
            })
            .collect();
        let lock_group = (!lock_rows.is_empty())
            .then(|| settings_group(several.then(|| "Lock after".into()), lock_rows, cx));
        let icons_on = self
            .preferences
            .as_ref()
            .is_some_and(|preferences| preferences.website_icons);
        let this = cx.entity().downgrade();
        let other = settings_group(
            None,
            [
                setting_row(
                    "Clear copied passwords after",
                    Some("Codes and other hidden fields too.".into()),
                    self.clear_after.clone(),
                    cx,
                ),
                setting_row(
                    "Show website icons",
                    Some("Your server fetches them for the websites of your logins.".into()),
                    switch("website-icons", icons_on, "Show website icons").on_change(
                        move |on, _, cx| {
                            this.update(cx, |this, cx| {
                                this.save(|preferences| preferences.website_icons = on, cx)
                            })
                            .ok();
                        },
                    ),
                    cx,
                ),
            ],
            cx,
        );
        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .children(lock_group)
            .child(other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_as_people_say_them() {
        assert_eq!(duration_label(Duration::from_secs(30)), "30 seconds");
        assert_eq!(duration_label(Duration::from_secs(60)), "1 minute");
        assert_eq!(duration_label(Duration::from_secs(14_400)), "4 hours");
        assert_eq!(duration_label(Duration::from_secs(86_400)), "1 day");
        assert_eq!(duration_label(Duration::from_secs(90)), "90 seconds");
        assert_eq!(clear_index(Some(Duration::from_secs(45))), 2);
        assert_eq!(clear_index(None), CLEAR_AFTER.len() - 1);
    }
}
