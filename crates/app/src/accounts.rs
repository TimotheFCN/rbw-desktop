//! The accounts on this computer: the list every window shows, asked of
//! the backend every couple of seconds while a window is open, so that
//! locking and signing in or out done anywhere, by the command line or the
//! lock timeout too, shows everywhere at once; and the account menu the
//! screens share.

use std::time::{Duration, Instant};

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Global, ParentElement as _,
    Task, Window,
};
use rbw_desktop_core::{Account, AccountId, Error, LockState};
use rbw_desktop_ui::{
    Confirmation, IconName, Opening, Popover, avatar, menu_item, menu_separator, popover,
};

use crate::alerts;
use crate::backend::{self, Load};
use crate::clipboard;

/// How often the accounts are asked whether they locked, unlocked or synced
/// while a window shows them.
const POLL: Duration = Duration::from_secs(2);
/// How often while no window shows, such as while the app waits in the menu
/// bar or its windows are minimised: often enough to clear the clipboard
/// soon after a lock, and to keep the menu bar item's Lock up to date. A
/// window that opens asks at once.
const POLL_HIDDEN: Duration = Duration::from_secs(30);

/// The accounts on this computer and their state, shared by every window.
pub struct Accounts {
    list: Load<Vec<Account>>,
    /// The account chosen last in the main window, which the menu bar panel
    /// follows too.
    chosen: Option<AccountId>,
    refresh_task: Task<()>,
    /// Whether the accounts are being asked for. One call at a time: rbw
    /// may take longer than the poll, and a call is never dropped for
    /// another, which would leave its process running.
    in_flight: bool,
    /// Whether something asked again while a call was on the way, which
    /// may have read the accounts before what changed them: its answer
    /// gives way to one more call.
    pending_refresh: bool,
    /// When the last answer came.
    completed_at: Instant,
    _poll: Task<()>,
}

/// Each time the accounts have been asked again, changed or not.
pub struct Refreshed;

impl EventEmitter<Refreshed> for Accounts {}

struct Shared(Entity<Accounts>);

impl Global for Shared {}

/// Starts asking for the accounts. Call once the backend is set up.
pub fn init(cx: &mut App) {
    let accounts = cx.new(|cx| {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                let asked = this.update(cx, |this: &mut Accounts, cx| {
                    if this.in_flight {
                        return;
                    }
                    // A secret on the clipboard goes soon after its account
                    // locks, even while no window shows.
                    let every = if any_window_shows(cx) || clipboard::has_pending_secret(cx) {
                        POLL
                    } else {
                        POLL_HIDDEN
                    };
                    if this.completed_at.elapsed() >= every {
                        this.refresh(cx);
                    }
                });
                if asked.is_err() {
                    break;
                }
            }
        });
        let mut this = Accounts {
            list: Load::Loading,
            chosen: None,
            refresh_task: Task::ready(()),
            in_flight: false,
            pending_refresh: false,
            completed_at: Instant::now(),
            _poll: poll,
        };
        this.refresh(cx);
        this
    });
    cx.set_global(Shared(accounts));
}

impl Accounts {
    pub fn global(cx: &App) -> Entity<Self> {
        cx.global::<Shared>().0.clone()
    }

    pub fn load(&self) -> &Load<Vec<Account>> {
        &self.list
    }

    /// Every account, or none until they are known.
    pub fn list(&self) -> &[Account] {
        self.list.ready().map(Vec::as_slice).unwrap_or_default()
    }

    pub fn get(&self, id: &AccountId) -> Option<&Account> {
        self.list().iter().find(|account| account.id == *id)
    }

    /// The account the windows show: the one chosen last if it still
    /// exists, else the first unlocked one, else the first.
    pub fn current(&self) -> Option<&Account> {
        current(self.list(), self.chosen.as_ref())
    }

    /// The account the windows show from now on, which may not be listed
    /// yet, such as one just added.
    pub fn choose(&mut self, id: Option<AccountId>, cx: &mut Context<Self>) {
        if self.chosen != id {
            self.chosen = id;
            cx.notify();
        }
    }

    /// Asks for the accounts again now, such as after unlocking one. While
    /// a call is on the way, one more follows it, whose answer is the one
    /// shown.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.in_flight {
            self.pending_refresh = true;
            return;
        }
        self.in_flight = true;
        self.refresh_task = backend::request(
            cx,
            |backend| backend.accounts(),
            |this, result, cx| {
                this.in_flight = false;
                this.completed_at = Instant::now();
                if std::mem::take(&mut this.pending_refresh) {
                    this.refresh(cx);
                    return;
                }
                match result {
                    Ok(accounts) => this.settle(accounts, cx),
                    // A poll failing once leaves the accounts as they were;
                    // only a list that was never read says why.
                    Err(error) => {
                        if this.list.ready().is_none() {
                            this.list = Load::Failed(error);
                            cx.notify();
                        }
                    }
                }
                cx.emit(Refreshed);
            },
        );
    }

    /// Asks for the accounts unless an answer is already on the way, for
    /// a window that wants them fresh without anything having changed.
    pub fn refresh_if_idle(&mut self, cx: &mut Context<Self>) {
        if !self.in_flight {
            self.refresh(cx);
        }
    }

    fn settle(&mut self, accounts: Vec<Account>, cx: &mut Context<Self>) {
        if self.list.ready() == Some(&accounts) {
            return;
        }
        // A secret copied from an account that locked or signed out goes
        // with it, and once none is unlocked, any secret.
        let closed = closed(self.list(), &accounts);
        for account in &closed {
            clipboard::account_closed(account, cx);
        }
        let none_unlocked = !accounts
            .iter()
            .any(|account| account.state == LockState::Unlocked);
        if !closed.is_empty() && none_unlocked {
            clipboard::all_closed(cx);
        }
        crate::vault_cache::follow_accounts(self.list(), &accounts, cx);
        self.list = Load::Ready(accounts);
        cx.notify();
    }
}

/// The accounts unlocked `before` that aren't any more `now`: locked,
/// signed out or removed.
fn closed(before: &[Account], now: &[Account]) -> Vec<AccountId> {
    before
        .iter()
        .filter(|before| {
            before.state == LockState::Unlocked
                && !now
                    .iter()
                    .any(|now| now.id == before.id && now.state == LockState::Unlocked)
        })
        .map(|account| account.id.clone())
        .collect()
}

/// Whether any of the app's windows is on screen.
fn any_window_shows(cx: &mut App) -> bool {
    cx.windows().into_iter().any(|handle| {
        handle
            .update(cx, |_, window, _| window.is_visible())
            .unwrap_or(false)
    })
}

/// The account the windows show: `chosen` if it is listed, else the first
/// unlocked one, else the first.
fn current<'a>(accounts: &'a [Account], chosen: Option<&AccountId>) -> Option<&'a Account> {
    chosen
        .and_then(|id| accounts.iter().find(|account| account.id == *id))
        .or_else(|| {
            accounts
                .iter()
                .find(|account| account.state == LockState::Unlocked)
        })
        .or_else(|| accounts.first())
}

/// [`Accounts::refresh`] from anywhere.
pub fn refresh(cx: &mut App) {
    Accounts::global(cx).update(cx, |accounts, cx| accounts.refresh(cx));
}

/// [`Accounts::refresh_if_idle`] from anywhere.
pub fn refresh_if_idle(cx: &mut App) {
    Accounts::global(cx).update(cx, |accounts, cx| accounts.refresh_if_idle(cx));
}

/// The accounts being signed out of, so that asking twice signs out once.
#[derive(Default)]
struct SigningOut(std::collections::HashSet<AccountId>);

impl Global for SigningOut {}

/// Asks, then signs out of the account and removes it from this computer.
/// `failed` hears of a failure, to say so in the window that asked.
pub fn sign_out(
    account: &Account,
    window: &mut Window,
    cx: &mut App,
    failed: impl FnOnce(Error, &mut Window, &mut App) + 'static,
) {
    let confirmation = Confirmation {
        title: format!("Sign out of {}?", name(account)).into(),
        message: "Its vault is removed from this computer, for the rbw command line too. \
                  You can sign in again at any time."
            .into(),
        confirm: "Sign Out".into(),
        destructive: true,
    };
    let id = account.id.clone();
    if cx.default_global::<SigningOut>().0.contains(&id) {
        return;
    }
    alerts::confirm(confirmation, window, cx, move |window, cx| {
        if !cx.default_global::<SigningOut>().0.insert(id.clone()) {
            return;
        }
        let removing = id.clone();
        let removed = backend::call(cx, move |backend| backend.remove_account(&removing));
        // The window that asked hears of a failure if it is still open.
        let asked = window.window_handle();
        cx.spawn(async move |cx| {
            let result = removed.await;
            cx.update(|cx| {
                cx.default_global::<SigningOut>().0.remove(&id);
                if let Err(error) = result {
                    asked
                        .update(cx, |_, window, cx| failed(error, window, cx))
                        .ok();
                }
                refresh(cx);
            });
        })
        .detach();
    });
}

/// Whether the account is being signed out of.
pub fn signing_out(id: &AccountId, cx: &App) -> bool {
    cx.try_global::<SigningOut>()
        .is_some_and(|signing_out| signing_out.0.contains(id))
}

/// What a screen asks of the window that shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Show this account, in whatever state it is.
    Show(AccountId),
    AddAccount,
    /// Ask, then sign out of this account and remove it from this computer.
    SignOut(AccountId),
    /// Leave adding an account for the account shown before.
    CancelAdd,
    /// Signing in to this account succeeded: show it once its new state is
    /// known.
    SignedIn(AccountId),
    /// The person locked this account in the window.
    Locked(AccountId),
}

pub fn name(account: &Account) -> String {
    account
        .email
        .clone()
        .unwrap_or_else(|| account.id.to_string())
}

/// The account's name where there is little room, as in the sidebar: the
/// part of its email before the @ when it is the only account, which can't
/// be mistaken for another.
pub fn short_name(account: &Account, accounts: &[Account]) -> String {
    let name = name(account);
    match name.split_once('@') {
        Some((local, _)) if accounts.len() <= 1 && !local.is_empty() => local.to_string(),
        _ => name,
    }
}

fn state(account: &Account) -> &'static str {
    match account.state {
        LockState::Unlocked => "Unlocked",
        LockState::Locked => "Locked",
        LockState::SignedOut => "Signed out",
    }
}

/// A screen that offers the account menu.
pub trait AccountMenu: EventEmitter<Request> + Sized + 'static {
    fn menu_open(&self) -> bool;
    fn set_menu_open(&mut self, open: bool);
    fn menu_focus(&self) -> &FocusHandle;
    /// Where the keyboard goes back to when the menu closes.
    fn menu_return(&self, cx: &App) -> FocusHandle;
}

pub fn toggle<V: AccountMenu>(view: &mut V, window: &mut Window, cx: &mut Context<V>) {
    if view.menu_open() {
        close(view, window, cx);
    } else {
        view.set_menu_open(true);
        window.focus(view.menu_focus(), cx);
        cx.notify();
    }
}

fn close<V: AccountMenu>(view: &mut V, window: &mut Window, cx: &mut Context<V>) {
    view.set_menu_open(false);
    window.focus(&view.menu_return(cx), cx);
    cx.notify();
}

/// The account menu of a screen showing `current`.
pub fn menu<V: AccountMenu>(
    view: &V,
    _accounts: &[Account],
    current: &AccountId,
    opening: Opening,
    cx: &mut Context<V>,
) -> Popover {
    let this = cx.entity().downgrade();
    let owner = cx.entity();
    let current = current.clone();
    popover(
        "account-menu",
        view.menu_open(),
        view.menu_focus(),
        "Accounts",
    )
    .opening(opening)
    .on_dismiss(move |window, cx| {
        this.update(cx, |this, cx| close(this, window, cx)).ok();
    })
    .when_shown(move |mut menu, _, cx| {
        let accounts: Vec<Account> = Accounts::global(cx).read(cx).list().to_vec();
        owner.update(cx, |_, cx| {
            for account in &accounts {
                let name = name(account);
                menu = menu.child(
                    menu_item(
                        gpui::ElementId::Name(format!("account-{}", account.id.profile()).into()),
                        name.clone(),
                    )
                    .leading(avatar(&name, cx))
                    .detail(state(account))
                    .checked(account.id == current)
                    .on_click(choose(Request::Show(account.id.clone()), cx)),
                );
            }
            menu.child(menu_separator(cx))
                .child(
                    menu_item("add-account", "Add Account…")
                        .icon(IconName::Plus)
                        .on_click(choose(Request::AddAccount, cx)),
                )
                .child(
                    menu_item("sign-out", "Sign Out…")
                        .on_click(choose(Request::SignOut(current.clone()), cx)),
                )
        })
    })
}

/// A menu item's click: closes the menu and makes the request.
fn choose<V: AccountMenu>(
    request: Request,
    cx: &mut Context<V>,
) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
    cx.listener(move |this: &mut V, _, window, cx| {
        close(this, window, cx);
        cx.emit(request.clone());
    })
}

#[cfg(test)]
mod tests {
    use rbw_desktop_core::Server;

    use super::*;

    fn account(profile: &str, state: LockState) -> Account {
        Account {
            id: AccountId::new(profile),
            email: Some(format!("{profile}@example.com")),
            server: Server::BitwardenCom,
            state,
            touch_id_ready: false,
            last_sync: None,
        }
    }

    #[test]
    fn the_current_account_is_the_chosen_then_an_unlocked_one() {
        let accounts = [
            account("home", LockState::Locked),
            account("work", LockState::Unlocked),
        ];
        let id = |account: Option<&Account>| account.map(|account| account.id.clone());
        assert_eq!(id(current(&accounts, None)), Some(accounts[1].id.clone()));
        assert_eq!(
            id(current(&accounts, Some(&accounts[0].id))),
            Some(accounts[0].id.clone())
        );
        let gone = AccountId::new("gone");
        assert_eq!(
            id(current(&accounts, Some(&gone))),
            Some(accounts[1].id.clone())
        );
        assert_eq!(
            id(current(&accounts[..1], None)),
            Some(accounts[0].id.clone())
        );
        assert!(current(&[], None).is_none());
    }

    #[test]
    fn the_only_account_goes_by_its_name() {
        let home = account("alex.martin", LockState::Unlocked);
        let work = account("alex", LockState::Locked);
        assert_eq!(
            short_name(&home, std::slice::from_ref(&home)),
            "alex.martin"
        );
        assert_eq!(
            short_name(&home, &[home.clone(), work]),
            "alex.martin@example.com"
        );
    }
}
