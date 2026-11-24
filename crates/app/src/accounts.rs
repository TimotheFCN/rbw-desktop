//! The accounts on this computer, as the screens offer them: the menu that
//! switches between them, adds one or signs out of one, and the requests
//! screens make of the window to do so.

use gpui::{App, Context, EventEmitter, FocusHandle, ParentElement as _, Window};
use rbw_desktop_core::{Account, AccountId, LockState};
use rbw_desktop_ui::{IconName, Opening, Popover, avatar, menu_item, menu_separator, popover};

/// What a screen asks of the window that shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Show this account, in whatever state it is.
    Show(AccountId),
    AddAccount,
    /// Ask, then sign out of this account and remove it from this computer.
    SignOut(AccountId),
    /// An account may have changed state, such as after unlocking: look
    /// again now rather than at the next poll.
    Refresh,
    /// Leave adding an account for the account shown before.
    CancelAdd,
    /// Signing in to this account succeeded: show it once its new state is
    /// known.
    SignedIn(AccountId),
}

pub fn name(account: &Account) -> String {
    account
        .email
        .clone()
        .unwrap_or_else(|| account.id.to_string())
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
    accounts: &[Account],
    current: &AccountId,
    opening: Opening,
    cx: &mut Context<V>,
) -> Popover {
    let this = cx.entity().downgrade();
    let mut menu = popover(
        "account-menu",
        view.menu_open(),
        view.menu_focus(),
        "Accounts",
    )
    .opening(opening)
    .on_dismiss(move |window, cx| {
        this.update(cx, |this, cx| close(this, window, cx)).ok();
    });
    for account in accounts {
        let name = name(account);
        menu = menu.child(
            menu_item(
                gpui::ElementId::Name(format!("account-{}", account.id.profile()).into()),
                name.clone(),
            )
            .leading(avatar(&name, cx))
            .detail(state(account))
            .checked(account.id == *current)
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
