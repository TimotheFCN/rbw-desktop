//! General: how long each account stays unlocked without being used, when
//! a copied secret leaves the clipboard, whether websites' icons show, the
//! menu bar item, and the command line tool.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    Styled as _, Subscription, Task, Window, div, px,
};
use rbw_desktop_core::{APP_NAME, AccountId, CommandLineTool, Error, ToolState};
use rbw_desktop_ui::{Select, Selected, field_error, setting_row, settings_group, switch};

use super::store::Store;
use crate::accounts::{self, Accounts};
use crate::backend;
use crate::command_line;
use crate::preferences::{self, Prefs};
use crate::tray::{self, MenuBar};

const LOCK_AFTER: [u64; 7] = [60, 5 * 60, 15 * 60, 30 * 60, 3600, 4 * 3600, 24 * 3600];
const CLEAR_AFTER: [Option<u64>; 6] = [Some(10), Some(30), Some(60), Some(120), Some(300), None];

pub struct GeneralPage {
    store: Entity<Store>,
    clear_after: Entity<Select>,
    /// For each account, how long it stays unlocked, with the choices it
    /// offers: the usual ones and, if it is set to something else, that.
    lock_after: HashMap<AccountId, (Entity<Select>, Vec<Duration>)>,
    /// The command line tool; `None` until known, or when the app has no
    /// rbw of its own to link.
    tool: Option<CommandLineTool>,
    /// Whether the tool is being turned on or off.
    tool_busy: Option<bool>,
    tool_problem: Option<&'static str>,
    tool_task: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl GeneralPage {
    pub fn new(store: Entity<Store>, cx: &mut Context<Self>) -> Self {
        let prefs = Prefs::global(cx);
        let clear = prefs.read(cx).get().clipboard_clear;
        let clear_after = cx.new(|cx| {
            Select::new(
                "Clear copied passwords after",
                CLEAR_AFTER.map(clear_label),
                clear_index(clear),
                cx,
            )
        });
        let subscriptions = vec![
            cx.subscribe(&clear_after, |_, _, Selected(index), cx| {
                let clear = CLEAR_AFTER[*index].map(Duration::from_secs);
                preferences::update(cx, move |preferences| preferences.clipboard_clear = clear);
            }),
            cx.observe(&prefs, |this, prefs, cx| {
                let index = clear_index(prefs.read(cx).get().clipboard_clear);
                this.clear_after
                    .update(cx, |select, cx| select.set_selected(index, cx));
                cx.notify();
            }),
            cx.observe(&store, |this, _, cx| this.follow_store(cx)),
            cx.observe(&Accounts::global(cx), |_, _, cx| cx.notify()),
        ];
        let subscriptions = subscriptions
            .into_iter()
            .chain(
                MenuBar::global(cx).map(|menu_bar| cx.observe(&menu_bar, |_, _, cx| cx.notify())),
            )
            .collect();
        let mut this = Self {
            store,
            clear_after,
            lock_after: HashMap::new(),
            tool: None,
            tool_busy: None,
            tool_problem: None,
            tool_task: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.follow_store(cx);
        this.refresh_command_line(cx);
        this
    }

    /// Reads the command line tool again, as it may have changed in a
    /// terminal.
    pub fn refresh_command_line(&mut self, cx: &mut Context<Self>) {
        if self.tool_busy.is_some() {
            return;
        }
        self.tool_task = backend::request(
            cx,
            |backend| backend.command_line_tool(),
            |this, result, cx| {
                match result {
                    Ok(tool) => this.tool = tool,
                    Err(error) => log::warn!("reading the command line tool: {error}"),
                }
                cx.notify();
            },
        );
    }

    fn set_command_line(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.tool_busy.is_some() {
            return;
        }
        self.tool_busy = Some(on);
        self.tool_problem = None;
        cx.notify();
        self.tool_task = backend::request(
            cx,
            move |backend| backend.set_command_line_tool(on),
            |this, result, cx| {
                this.tool_busy = None;
                match result {
                    // also when the person canceled the administrator's
                    // prompt, which leaves it as it was
                    Ok(tool) => this.tool = Some(tool),
                    Err(error) => {
                        this.tool_problem = Some(match error {
                            Error::AppNotInstalled => {
                                "Move the app to the Applications folder first, then open it from there."
                            }
                            ref error => backend::describe(error),
                        });
                        this.refresh_command_line(cx);
                    }
                }
                cx.notify();
            },
        );
    }

    fn render_command_line(&self, tool: &CommandLineTool, cx: &mut Context<Self>) -> AnyElement {
        let path = command_line::shown(&tool.path);
        let subtitle = match tool.state {
            ToolState::On if !tool.on_path => {
                let folder = tool
                    .path
                    .parent()
                    .map(command_line::shown)
                    .unwrap_or_default();
                format!(
                    "Installed at {path}. Your terminal won’t find it until {folder} is on your PATH."
                )
            }
            ToolState::On => format!("Installed at {path}."),
            ToolState::Off => {
                format!("Use rbw in a terminal, with the same accounts as {APP_NAME}.")
            }
            ToolState::Taken => format!("Another program is already at {path}."),
        };
        let on = self.tool_busy.unwrap_or(tool.state == ToolState::On);
        let set = cx.listener(|this, on: &bool, _, cx| this.set_command_line(*on, cx));
        let group = settings_group(
            None,
            [setting_row(
                "Command line tool",
                Some(subtitle.into()),
                switch("command-line", on, "Command line tool")
                    .disabled(tool.state == ToolState::Taken)
                    .on_change(move |on, window, cx| set(&on, window, cx)),
                cx,
            )],
            cx,
        );
        let below = self
            .tool_problem
            .map(|problem| field_error(problem, cx).px(px(4.)));
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(group)
            .children(below)
            .into_any_element()
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
        let preferences = Prefs::global(cx).read(cx).get().clone();
        let menu_bar_note = if MenuBar::global(cx)
            .is_some_and(|menu_bar| preferences.menu_bar && menu_bar.read(cx).unseen())
        {
            "Your desktop has no system tray to show it in."
        } else if cfg!(target_os = "macos") {
            "Copy passwords you use often and make new ones from the menu bar."
        } else {
            "Copy passwords you use often and make new ones from the tray."
        };
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
                    switch(
                        "website-icons",
                        preferences.website_icons,
                        "Show website icons",
                    )
                    .on_change(|on, _, cx| {
                        preferences::update(cx, move |preferences| preferences.website_icons = on)
                    }),
                    cx,
                ),
                setting_row(
                    tray::SETTING,
                    Some(menu_bar_note.into()),
                    switch("menu-bar", preferences.menu_bar, tray::SETTING).on_change(
                        |on, _, cx| {
                            preferences::update(cx, move |preferences| preferences.menu_bar = on)
                        },
                    ),
                    cx,
                ),
            ],
            cx,
        );
        let tool = self
            .tool
            .clone()
            .map(|tool| self.render_command_line(&tool, cx));
        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .children(lock_group)
            .child(other)
            .children(tool)
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
