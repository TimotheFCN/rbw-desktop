//! SSH Agent: rbw's agent serves the vault's SSH keys to `ssh` and `git`.
//! The page says whether it is set up and whether `ssh` uses it, with one
//! action to set it up or remove it, the keys it serves, and whether each
//! use asks first. Only one account's agent can be set up at a time.

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Render,
    SharedString, Styled as _, Subscription, Task, Window, div, prelude::FluentBuilder as _, px,
};
use rbw_desktop_core::{
    Account, AccountId, Error, ItemId, LockState, SshAgentStatus, SshKeySummary, SshService,
    SshUses,
};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, Confirmation, IconName, Select, Selected, button, card,
    copied_badge, field_error, field_row, icon, setting_row, setting_row_with, settings_group,
    switch,
};

use super::note;
use super::store::Store;
use crate::accounts::{self, Accounts};
use crate::alerts;
use crate::backend::{self, Load};
use crate::clipboard;

/// What was copied from the page.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Copied {
    Line,
    Key(ItemId),
}

/// What the page is doing.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Busy {
    SettingUp,
    Removing,
}

pub struct SshPage {
    store: Entity<Store>,
    /// The account whose agent is shown.
    account: Option<AccountId>,
    /// Chooses the account, when there are several.
    chooser: Entity<Select>,
    /// The accounts the chooser lists, in its order.
    choices: Vec<AccountId>,
    status: Load<SshAgentStatus>,
    busy: Option<Busy>,
    problem: Option<&'static str>,
    /// What was just copied: the line to paste, or a key.
    copied: Option<Copied>,
    /// Whether the account shown was chosen by the person, rather than
    /// guessed: a guess follows the account the agent is set up for.
    chosen: bool,
    /// Whether Settings shows this page, which only then asks the agent.
    shown: bool,
    load_task: Task<()>,
    task: Task<()>,
    copied_timer: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl SshPage {
    pub fn new(store: Entity<Store>, _: &mut Window, cx: &mut Context<Self>) -> Self {
        let chooser = cx.new(|cx| Select::new("Account", Vec::<SharedString>::new(), 0, cx));
        let accounts = Accounts::global(cx);
        let subscriptions = vec![
            cx.subscribe(&chooser, |this, _, Selected(index), cx| {
                if let Some(id) = this.choices.get(*index).cloned() {
                    this.chosen = true;
                    this.show(id, cx);
                }
            }),
            // Unlocking shows the keys, locking hides them.
            cx.observe(&accounts, |this, _, cx| {
                this.follow_accounts(cx);
                if this.shown {
                    this.refresh(cx);
                }
            }),
            cx.observe(&store, |_, _, cx| cx.notify()),
        ];
        let mut this = Self {
            store,
            account: None,
            chooser,
            choices: vec![],
            status: Load::Loading,
            busy: None,
            problem: None,
            copied: None,
            chosen: false,
            shown: false,
            load_task: Task::ready(()),
            task: Task::ready(()),
            copied_timer: Task::ready(()),
            _subscriptions: subscriptions,
        };
        this.follow_accounts(cx);
        this
    }

    /// Settings shows the page, or another: the page asks the agent only
    /// while it shows.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.shown = shown;
        if shown {
            self.refresh(cx);
        }
    }

    /// Lists the accounts in the chooser, and picks one to show if none is:
    /// an unlocked one, whose keys can be listed, or the first.
    fn follow_accounts(&mut self, cx: &mut Context<Self>) {
        let accounts: Vec<Account> = Accounts::global(cx)
            .read(cx)
            .list()
            .iter()
            .filter(|account| account.state != LockState::SignedOut)
            .cloned()
            .collect();
        if self
            .account
            .as_ref()
            .is_none_or(|id| !accounts.iter().any(|a| a.id == *id))
        {
            self.account = accounts
                .iter()
                .find(|account| account.state == LockState::Unlocked)
                .or_else(|| accounts.first())
                .map(|account| account.id.clone());
            self.chosen = false;
        }
        self.choices = accounts.iter().map(|account| account.id.clone()).collect();
        let selected = self
            .account
            .as_ref()
            .and_then(|id| self.choices.iter().position(|choice| choice == id))
            .unwrap_or_default();
        let labels: Vec<String> = accounts.iter().map(accounts::name).collect();
        self.chooser
            .update(cx, |chooser, cx| chooser.set_options(labels, selected, cx));
    }

    fn show(&mut self, account: AccountId, cx: &mut Context<Self>) {
        if self.account.as_ref() != Some(&account) {
            self.account = Some(account);
            self.status = Load::Loading;
            self.problem = None;
            self.follow_accounts(cx);
            self.refresh(cx);
        }
    }

    /// Asks the agent's state again.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else {
            self.status = Load::Failed(Error::AccountNotFound);
            cx.notify();
            return;
        };
        self.load_task = backend::request(
            cx,
            move |backend| backend.ssh_agent(&account),
            |this, result, cx| {
                // Until someone chooses, show the account whose agent it is.
                if let Ok(SshAgentStatus {
                    service: SshService::InstalledForOther(other),
                    ..
                }) = &result
                    && !this.chosen
                    && this.choices.contains(other)
                {
                    let other = other.clone();
                    this.show(other, cx);
                    return;
                }
                if this.status.ready().is_none() || result.is_ok() {
                    this.status = result.into();
                }
                cx.notify();
            },
        );
    }

    fn set_up(&mut self, cx: &mut Context<Self>) {
        let Some(account) = self.account.clone() else {
            return;
        };
        // Another account's agent goes first: only one can be set up.
        let other = match self.status.ready().map(|status| &status.service) {
            Some(SshService::InstalledForOther(other)) => Some(other.clone()),
            _ => None,
        };
        self.run(
            Busy::SettingUp,
            move |backend| {
                if let Some(other) = other {
                    backend.uninstall_ssh_agent(&other)?;
                }
                backend.install_ssh_agent(&account)
            },
            cx,
        );
    }

    fn remove(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let confirmation = Confirmation {
            title: "Remove the SSH agent?".into(),
            // launchd stops the agent's process, which holds the unlocked
            // vault.
            message: if cfg!(target_os = "macos") {
                "ssh and git stop using the keys in your vault, and the vault locks. You \
                 can set it up again at any time."
            } else {
                "ssh and git stop using the keys in your vault. You can set it up again at \
                 any time."
            }
            .into(),
            confirm: "Remove".into(),
            destructive: true,
        };
        let this = cx.entity().downgrade();
        alerts::confirm(confirmation, window, cx, move |_, cx| {
            this.update(cx, |this, cx| {
                let Some(account) = this.account.clone() else {
                    return;
                };
                this.run(
                    Busy::Removing,
                    move |backend| backend.uninstall_ssh_agent(&account),
                    cx,
                );
            })
            .ok();
        });
    }

    fn run(
        &mut self,
        busy: Busy,
        call: impl FnOnce(&dyn rbw_desktop_core::Backend) -> rbw_desktop_core::Result<SshAgentStatus>
        + Send
        + 'static,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(busy);
        self.problem = None;
        cx.notify();
        self.task = backend::request(cx, call, |this, result, cx| {
            this.busy = None;
            match result {
                Ok(status) => this.status = Load::Ready(status),
                Err(error) => {
                    this.problem = Some(match error {
                        Error::SshAgentUnsupported => {
                            "This system can't run the SSH agent as a service."
                        }
                        Error::SshAgentInstalledForOther(_) => {
                            "Another account's SSH agent is set up. Remove it first."
                        }
                        ref error => describe(error),
                    });
                    this.refresh(cx);
                }
            }
            cx.notify();
        });
    }

    fn copy(&mut self, what: Copied, text: &str, cx: &mut Context<Self>) {
        clipboard::copy_text(text, cx);
        self.copied = Some(what);
        self.copied_timer = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(rbw_desktop_ui::motion::CONFIRMATION_HOLD)
                .await;
            this.update(cx, |this, cx| {
                this.copied = None;
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    fn render_status(&self, status: &SshAgentStatus, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let (on, off, warn) = (colors.success, colors.text_secondary, colors.danger);
        let dot = |color| div().size(px(8.)).rounded_full().bg(color);
        let value = |color, text: SharedString| {
            div()
                .flex()
                .items_center()
                .gap(px(7.))
                .text_color(colors.text_secondary)
                .child(dot(color))
                .child(text)
        };
        let accounts = Accounts::global(cx).read(cx);
        let state = match &status.service {
            SshService::Installed if status.running => value(on, "On".into()),
            SshService::Installed => value(on, "On, starts when needed".into()),
            SshService::NotInstalled => value(off, "Not set up".into()),
            SshService::InstalledForOther(other) => {
                let owner = accounts
                    .get(other)
                    .map_or_else(|| other.to_string(), accounts::name);
                value(off, format!("Set up for {owner}").into())
            }
            SshService::Unsupported => value(off, "Not available here".into()),
        };
        let mut rows = vec![setting_row("SSH agent", None, state, cx)];
        if status.service == SshService::Installed {
            let uses = match &status.ssh_uses {
                SshUses::Rbw => value(on, "Yes".into()),
                SshUses::Other(_) => value(warn, "No, another agent".into()),
                SshUses::None => value(warn, "No agent".into()),
                SshUses::Unknown => value(off, "Can’t tell".into()),
            };
            rows.push(setting_row("Used by ssh", None, uses, cx));
        }
        settings_group(None, rows, cx)
    }

    /// The one thing to do next: set up, remove, or move the agent here.
    fn render_action(&self, status: &SshAgentStatus, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (label, style, busy) = match status.service {
            SshService::Unsupported => return None,
            SshService::NotInstalled => ("Set Up", ButtonStyle::Primary, Busy::SettingUp),
            SshService::InstalledForOther(_) => (
                "Use This Account Instead",
                ButtonStyle::Primary,
                Busy::SettingUp,
            ),
            SshService::Installed => ("Remove…", ButtonStyle::Normal, Busy::Removing),
        };
        let theme = cx.theme();
        let (dim, small) = (theme.colors.text_secondary, theme.metrics.small_text_size);
        let progress = match self.busy {
            Some(Busy::SettingUp) => Some("Setting up…"),
            Some(Busy::Removing) => Some("Removing…"),
            None => None,
        };
        let removing = busy == Busy::Removing;
        Some(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(4.))
                .child(div().flex_1().min_w_0().map(|d| match self.problem {
                    Some(problem) => d.child(field_error(problem, cx)),
                    None => d.text_size(small).text_color(dim).children(progress),
                }))
                .child(
                    button("ssh-action")
                        .label(label)
                        .style(style)
                        .busy(self.busy == Some(busy))
                        .disabled(self.busy.is_some())
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if removing {
                                this.remove(window, cx);
                            } else {
                                this.set_up(cx);
                            }
                        })),
                )
                .into_any_element(),
        )
    }

    /// What to do when `ssh` uses another agent than the one set up.
    fn render_help(&self, status: &SshAgentStatus, cx: &mut Context<Self>) -> Option<AnyElement> {
        if status.service != SshService::Installed
            || !matches!(status.ssh_uses, SshUses::Other(_) | SshUses::None)
        {
            return None;
        }
        let socket = status.socket.display();
        let (advice, line): (&str, SharedString) = if cfg!(target_os = "macos") {
            (
                "ssh still uses another agent. Add this line at the top of the file \
                 .ssh/config in your home folder:",
                format!("IdentityAgent \"{socket}\"").into(),
            )
        } else {
            (
                "ssh still uses another agent. Log out and back in, so that your session \
                 picks this one up. If ssh still doesn’t use it in a terminal, add this line \
                 to your shell’s startup file, such as .bashrc:",
                format!("export SSH_AUTH_SOCK={}", shell_word(&socket.to_string())).into(),
            )
        };
        let theme = cx.theme();
        let (warn, mono) = (theme.colors.danger, theme.mono_font_family.clone());
        let copy = line.clone();
        let row = field_row(
            "ssh-line",
            "Line to add",
            div()
                .font_family(mono)
                .text_size(px(12.))
                .truncate()
                .child(line),
        )
        .copied(self.copied == Some(Copied::Line))
        .action(icon(IconName::Copy))
        .on_click(cx.listener(move |this, _, _, cx| this.copy(Copied::Line, &copy, cx)));
        Some(
            div()
                .flex()
                .flex_col()
                .gap(px(10.))
                .child(
                    div()
                        .flex()
                        .gap(px(8.))
                        .px(px(4.))
                        .child(
                            div()
                                .pt(px(2.))
                                .text_color(warn)
                                .child(icon(IconName::Warning)),
                        )
                        .child(note(advice, cx).px(px(0.)).flex_1().min_w_0()),
                )
                .child(card().row(row))
                .into_any_element(),
        )
    }

    fn render_keys(&self, status: &SshAgentStatus, cx: &mut Context<Self>) -> AnyElement {
        let rows: Vec<AnyElement> = match &status.keys {
            None => vec![note_row("Unlock the vault to see its keys.", cx)],
            Some(keys) if keys.is_empty() => vec![note_row("This vault has no SSH keys.", cx)],
            Some(keys) => keys.iter().map(|key| self.render_key(key, cx)).collect(),
        };
        settings_group(Some("Keys".into()), rows, cx)
    }

    /// A key: its name, kind and fingerprint, and a button that copies its
    /// public key, which is what servers and websites ask for.
    fn render_key(&self, key: &SshKeySummary, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (dim, mono) = (theme.colors.text_secondary, theme.mono_font_family.clone());
        let subtitle = div()
            .flex()
            .gap(px(6.))
            .min_w_0()
            .when(!key.key_type.is_empty(), |d| {
                d.child(div().flex_none().child(key.key_type.clone()))
            })
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(mono)
                    .text_size(px(11.))
                    .text_color(dim)
                    .child(key.fingerprint.clone()),
            );
        let copied = Copied::Key(key.item.clone());
        let control = if self.copied.as_ref() == Some(&copied) {
            copied_badge(cx).into_any_element()
        } else {
            let public_key = key.public_key.clone();
            button(gpui::ElementId::Name(format!("copy-{}", key.item.0).into()))
                .icon(IconName::Copy)
                .style(ButtonStyle::Flat)
                .size(px(26.))
                .accessibility_label(format!("Copy the public key of {}", key.name))
                .disabled(public_key.is_empty())
                .on_click(
                    cx.listener(move |this, _, _, cx| this.copy(copied.clone(), &public_key, cx)),
                )
                .into_any_element()
        };
        setting_row_with(key.name.clone(), Some(subtitle), control, cx)
    }
}

fn note_row(text: &'static str, cx: &App) -> AnyElement {
    div()
        .flex()
        .items_center()
        .min_h(px(44.))
        .px(px(13.))
        .text_color(cx.theme().colors.text_secondary)
        .child(text)
        .into_any_element()
}

/// `text` as one word for a POSIX shell: in single quotes, in which nothing
/// expands, with its own single quotes closed and reopened around them.
fn shell_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// What went wrong, for this page: rbw's own failures to set up the
/// service read better as one sentence.
fn describe(error: &Error) -> &'static str {
    match error {
        Error::Other(_) | Error::Server(_) => "The SSH agent couldn’t be set up.",
        error => backend::describe(error),
    }
}

impl Render for SshPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let several = self.choices.len() > 1;
        let chooser = several.then(|| {
            settings_group(
                None,
                [setting_row("Account", None, self.chooser.clone(), cx)],
                cx,
            )
        });
        let intro = note(
            "Lets ssh and git sign in with the SSH keys in your vault, so that they never \
             sit in files on this computer.",
            cx,
        );
        let body: Vec<AnyElement> = match &self.status {
            Load::Loading => vec![],
            Load::Failed(error) => vec![
                note(backend::describe(error), cx).into_any_element(),
                div()
                    .flex()
                    .child(
                        button("ssh-retry")
                            .label("Try Again")
                            .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
                    )
                    .into_any_element(),
            ],
            Load::Ready(status) => {
                let status = status.clone();
                let account = self.account.clone();
                let mut parts = vec![self.render_status(&status, cx)];
                parts.extend(self.render_action(&status, cx));
                if status.service == SshService::Unsupported {
                    parts.push(
                        note(
                            "rbw runs the SSH agent as a login service, which needs systemd \
                             on Linux or launchd on macOS.",
                            cx,
                        )
                        .into_any_element(),
                    );
                }
                parts.extend(self.render_help(&status, cx));
                if status.service != SshService::Unsupported {
                    parts.push(self.render_keys(&status, cx));
                    let confirm = account.and_then(|id| {
                        let confirm = self.store.read(cx).get(&id)?.ssh_confirm;
                        Some((id, confirm))
                    });
                    if let Some((id, confirm)) = confirm {
                        let store = self.store.clone();
                        parts.push(settings_group(
                            None,
                            [setting_row(
                                "Ask before each use",
                                Some(
                                    "Confirm each time a program wants to use one of your keys."
                                        .into(),
                                ),
                                switch("ssh-confirm", confirm, "Ask before each use").on_change(
                                    move |on, _, cx| {
                                        store.update(cx, |store, cx| {
                                            store.update(&id, |s| s.ssh_confirm = on, cx)
                                        });
                                    },
                                ),
                                cx,
                            )],
                            cx,
                        ));
                    }
                }
                parts
            }
        };
        div()
            .flex()
            .flex_col()
            .gap(px(18.))
            .child(intro)
            .children(chooser)
            .children(body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_pastes_as_one_word() {
        assert_eq!(
            shell_word("/run/user/1000/rbw/agent"),
            "'/run/user/1000/rbw/agent'"
        );
        assert_eq!(shell_word("/tmp/$HOME `x` \"q\""), "'/tmp/$HOME `x` \"q\"'");
        assert_eq!(shell_word("/tmp/it's"), r"'/tmp/it'\''s'");
    }
}
