//! The app's window. It shows one screen at a time, chosen from the accounts
//! on this computer and their state: adding a first account, signing in,
//! the lock screen or the vault. It asks the backend for the accounts every
//! couple of seconds, so that locking and signing in or out done elsewhere,
//! by the command line or the lock timeout, show here too.

use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, AppContext as _, Context, Entity, FocusHandle, Focusable as _,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Subscription,
    Task, Window, actions, div, point, prelude::FluentBuilder as _, px, size,
};
use rbw_desktop_core::{Account, AccountId, Error, LockState};
use rbw_desktop_ui::{
    ActiveTheme as _, ConfirmDialog, ConfirmEvent, Confirmation, Look, SheetSlot, Theme, ToastSlot,
    chrome, motion,
};

use crate::accounts::{self, Request};
use crate::backend;
use crate::lock::LockScreen;
use crate::sign_in::SignIn;
use crate::vault::VaultView;

actions!(
    window,
    [
        /// Starts adding another account.
        AddAccount
    ]
);

const CONTEXT: &str = "MainWindow";
/// How often the accounts are asked whether they locked, unlocked or synced.
const POLL: Duration = Duration::from_secs(2);

pub fn open(cx: &mut App) {
    let options = chrome::window_options(
        "Vault",
        size(px(880.), px(580.)),
        size(px(680.), px(440.)),
        cx,
    );
    // gpui-base's Root around the view gives the window Tab and Shift-Tab.
    let window = gpui::open_window(options, cx, |window, cx| {
        cx.new(|cx| MainWindow::new(window, cx))
    });
    if let Err(error) = window {
        log::error!("opening the window: {error:#}");
    }
}

enum Screen {
    /// Until the accounts are known.
    Loading,
    /// The accounts can't be read, such as when rbw is missing.
    Unavailable(Error),
    Adding(Entity<SignIn>),
    SignIn(Entity<SignIn>),
    Lock(Entity<LockScreen>),
    Vault(Entity<VaultView>),
}

impl Screen {
    fn view(&self) -> Option<AnyView> {
        match self {
            Screen::Loading | Screen::Unavailable(_) => None,
            Screen::Adding(view) | Screen::SignIn(view) => Some(view.clone().into()),
            Screen::Lock(view) => Some(view.clone().into()),
            Screen::Vault(view) => Some(view.clone().into()),
        }
    }

    /// Whether the screen is the one an account in `state` calls for.
    fn suits(&self, state: LockState) -> bool {
        matches!(
            (self, state),
            (Screen::SignIn(_), LockState::SignedOut)
                | (Screen::Lock(_), LockState::Locked)
                | (Screen::Vault(_), LockState::Unlocked)
        )
    }
}

pub struct MainWindow {
    accounts: Vec<Account>,
    /// The account shown, or the one to go back to while adding another.
    current: Option<AccountId>,
    screen: Screen,
    /// Names the screen shown, so that each new one plays its entrance.
    generation: usize,
    /// The screen being replaced, drawn under the new one while it fades in.
    leaving: Option<AnyView>,
    alert: SheetSlot,
    /// Where the keyboard was before the alert, to go back there after.
    alert_return: Option<FocusHandle>,
    toast: ToastSlot,
    /// Whether the next accounts to arrive replace even the sign-in that
    /// just finished.
    show_next: bool,
    _screen_events: Option<Subscription>,
    refresh_task: Task<()>,
    leave_task: Task<()>,
    _poll: Task<()>,
}

impl MainWindow {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_appearance(window, |_, window, cx| {
            Theme::follow_window_appearance(window, cx);
        })
        .detach();
        let poll = cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if this
                    .update_in(cx, |this, window, cx| this.refresh(window, cx))
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut this = Self {
            accounts: vec![],
            current: None,
            screen: Screen::Loading,
            generation: 0,
            leaving: None,
            alert: SheetSlot::alert(),
            alert_return: None,
            toast: ToastSlot::default(),
            show_next: false,
            _screen_events: None,
            refresh_task: Task::ready(()),
            leave_task: Task::ready(()),
            _poll: poll,
        };
        this.refresh(window, cx);
        this
    }

    /// Asks for the accounts again and shows what they call for.
    fn refresh(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.refresh_task = backend::request_in(
            window,
            cx,
            |backend| backend.accounts(),
            |this, result, window, cx| match result {
                Ok(accounts) => {
                    this.accounts = accounts;
                    if std::mem::take(&mut this.show_next) {
                        this.show_current(window, cx);
                    } else {
                        this.settle(window, cx);
                    }
                }
                // A poll failing once leaves the screen as it is; only a
                // window that has nothing to show yet says why.
                Err(error) => {
                    if matches!(this.screen, Screen::Loading | Screen::Unavailable(_)) {
                        this.show(Screen::Unavailable(error), window, cx);
                    }
                }
            },
        );
    }

    /// Shows the screen the current account's state calls for, or keeps the
    /// one on screen up to date. Adding an account stays on screen until it
    /// is done or abandoned.
    fn settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Screen::Adding(sign_in) = &self.screen {
            let accounts = self.accounts.clone();
            sign_in.update(cx, |sign_in, cx| sign_in.set_accounts(accounts, cx));
            return;
        }
        self.show_current(window, cx);
    }

    fn show_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .current
            .as_ref()
            .and_then(|id| self.accounts.iter().find(|account| &account.id == id))
            .or_else(|| {
                self.accounts
                    .iter()
                    .find(|account| account.state == LockState::Unlocked)
            })
            .or_else(|| self.accounts.first())
            .cloned();
        let Some(account) = current else {
            self.current = None;
            self.add_account(window, cx);
            return;
        };
        self.current = Some(account.id.clone());
        let accounts = self.accounts.clone();
        if !self.screen.suits(account.state) || !self.shows_account(&account.id, cx) {
            let screen = self.screen_for(account, window, cx);
            self.show(screen, window, cx);
            return;
        }
        match &self.screen {
            Screen::Lock(lock) => {
                lock.update(cx, |lock, cx| lock.set_account(account, accounts, cx));
            }
            Screen::Vault(vault) => {
                vault.update(cx, |vault, cx| vault.set_account(account, accounts, cx));
            }
            Screen::SignIn(sign_in) => {
                sign_in.update(cx, |sign_in, cx| sign_in.set_accounts(accounts, cx));
            }
            Screen::Loading | Screen::Unavailable(_) | Screen::Adding(_) => {}
        }
    }

    fn shows_account(&self, id: &AccountId, cx: &App) -> bool {
        match &self.screen {
            Screen::SignIn(view) => view.read(cx).account() == Some(id),
            Screen::Lock(view) => view.read(cx).account() == id,
            Screen::Vault(view) => view.read(cx).account() == id,
            Screen::Loading | Screen::Unavailable(_) | Screen::Adding(_) => false,
        }
    }

    fn screen_for(
        &mut self,
        account: Account,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Screen {
        let accounts = self.accounts.clone();
        match account.state {
            LockState::SignedOut => {
                let view = cx.new(|cx| SignIn::existing(account, accounts, window, cx));
                self.listen(&view, window, cx);
                Screen::SignIn(view)
            }
            LockState::Locked => {
                let view = cx.new(|cx| LockScreen::new(account, accounts, window, cx));
                self.listen(&view, window, cx);
                Screen::Lock(view)
            }
            LockState::Unlocked => {
                let view = cx.new(|cx| VaultView::new(account, accounts, window, cx));
                self.listen(&view, window, cx);
                Screen::Vault(view)
            }
        }
    }

    fn listen<V: gpui::EventEmitter<Request>>(
        &mut self,
        view: &Entity<V>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self._screen_events = Some(cx.subscribe_in(
            view,
            window,
            |this, _, request, window, cx| this.handle(request.clone(), window, cx),
        ));
    }

    fn handle(&mut self, request: Request, window: &mut Window, cx: &mut Context<Self>) {
        match request {
            Request::Show(id) => {
                self.current = Some(id);
                self.show_current(window, cx);
                self.refresh(window, cx);
            }
            Request::AddAccount => self.add_account(window, cx),
            Request::CancelAdd => self.show_current(window, cx),
            Request::SignedIn(id) => {
                self.current = Some(id);
                self.show_next = true;
                self.refresh(window, cx);
            }
            Request::SignOut(id) => self.confirm_sign_out(id, window, cx),
            Request::Refresh => self.refresh(window, cx),
        }
    }

    fn add_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.screen, Screen::Adding(_)) {
            return;
        }
        let accounts = self.accounts.clone();
        let view = cx.new(|cx| SignIn::add(accounts, window, cx));
        self.listen(&view, window, cx);
        self.show(Screen::Adding(view), window, cx);
    }

    fn add_account_action(&mut self, _: &AddAccount, window: &mut Window, cx: &mut Context<Self>) {
        self.add_account(window, cx);
    }

    /// Replaces the screen, fading the new one in over the old.
    fn show(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<Self>) {
        let old = std::mem::replace(&mut self.screen, screen);
        self.leaving = old.view();
        self.generation += 1;
        // Nothing of a vault stays behind it once it has gone.
        self.alert.dismiss();
        self.leave_task = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(motion::STANDARD).await;
            this.update(cx, |this, cx| {
                this.leaving = None;
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    fn confirm_sign_out(&mut self, id: AccountId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(account) = self.accounts.iter().find(|account| account.id == id) else {
            return;
        };
        let confirmation = Confirmation {
            title: format!("Sign out of {}?", accounts::name(account)).into(),
            message: "Its vault is removed from this computer, for the rbw command line too. \
                      You can sign in again at any time."
                .into(),
            confirm: "Sign Out".into(),
            destructive: true,
        };
        let this = cx.entity().downgrade();
        self.confirm(confirmation, window, cx, move |window, cx| {
            this.update(cx, |this, cx| this.sign_out(id, window, cx))
                .ok();
        });
    }

    fn sign_out(&mut self, id: AccountId, window: &mut Window, cx: &mut Context<Self>) {
        let removed = id.clone();
        backend::request_in(
            window,
            cx,
            move |backend| backend.remove_account(&id),
            move |this, result, window, cx| match result {
                Ok(()) => {
                    if this.current.as_ref() == Some(&removed) {
                        this.current = None;
                    }
                    this.accounts.retain(|account| account.id != removed);
                    this.settle(window, cx);
                    this.refresh(window, cx);
                }
                Err(error) => this.toast.show(
                    backend::describe(&error),
                    |this: &mut Self| &mut this.toast,
                    cx,
                ),
            },
        )
        .detach();
    }

    /// Asks before doing something that can't be undone; `then` runs once
    /// confirmed, after the alert has started to leave.
    pub fn confirm(
        &mut self,
        confirmation: Confirmation,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        let dialog = cx.new(|cx| ConfirmDialog::new(confirmation, cx));
        let mut then = Some(then);
        cx.subscribe_in(&dialog, window, move |this, _, event, window, cx| {
            this.alert.dismiss();
            if let Some(focus) = this.alert_return.take() {
                window.focus(&focus, cx);
            }
            if *event == ConfirmEvent::Confirmed
                && let Some(then) = then.take()
            {
                window.defer(cx, then);
            }
            cx.notify();
        })
        .detach();
        self.alert_return = window.focused(cx);
        window.focus(&dialog.read(cx).focus_handle(cx), cx);
        self.alert.present(dialog);
        cx.notify();
    }

    fn render_unavailable(
        &self,
        error: &Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dim = cx.theme().colors.text_secondary;
        plain_screen(
            "unavailable",
            div()
                .max_w(px(300.))
                .text_center()
                .text_color(dim)
                .child(backend::describe(error)),
            window,
            cx,
        )
    }
}

/// The window found from anywhere in it, to ask for a confirmation.
pub fn main_window(window: &Window, cx: &App) -> Option<Entity<MainWindow>> {
    let root = window.root::<gpui::base::Root>().flatten()?;
    root.read(cx).view().clone().downcast::<MainWindow>().ok()
}

/// [`MainWindow::confirm`] from any view in the window.
pub fn confirm(
    confirmation: Confirmation,
    window: &mut Window,
    cx: &mut App,
    then: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    if let Some(main) = main_window(window, cx) {
        main.update(cx, |main, cx| main.confirm(confirmation, window, cx, then));
    }
}

/// A screen of its own in the window, such as the lock screen: the window's
/// material, a bar to drag the window by with its buttons, and `content` in
/// the middle.
pub fn plain_screen(
    id: &'static str,
    content: impl IntoElement,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = cx.theme();
    let look = theme.look;
    let fill = match look {
        Look::MacOs => theme.colors.sidebar,
        Look::Gnome => theme.colors.window.into(),
    };
    let height = theme.metrics.toolbar_height;
    let corner = chrome::corner_radius(window, cx);
    let bar = chrome::drag_region(
        (gpui::ElementId::from(id), "bar"),
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(height)
            .flex()
            .items_center()
            .px(px(8.))
            .children(chrome::traffic_lights(cx))
            .children(chrome::window_buttons(chrome::Side::Start, window, cx))
            .child(div().flex_1())
            .children(chrome::window_buttons(chrome::Side::End, window, cx)),
        window,
        cx,
    );
    div()
        .id(id)
        .relative()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        // On macOS the window's own blurred material shows.
        .when(!cfg!(target_os = "macos"), |d| d.bg(fill))
        .rounded(corner)
        .child(bar)
        .child(content)
        .into_any_element()
}

impl Render for MainWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current: Option<AnyElement> = match &self.screen {
            Screen::Loading => None,
            Screen::Unavailable(error) => {
                let error = error.clone();
                Some(self.render_unavailable(&error, window, cx))
            }
            screen => screen.view().map(|view| view.into_any_element()),
        };
        let leaving = self.leaving.clone();
        let alert = self.alert.render(px(300.), window, cx);
        let toast = self.toast.render(window, cx);
        let content = div()
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::add_account_action))
            .relative()
            .size_full()
            .children(leaving.map(|view| div().absolute().inset_0().child(view)))
            .children(current.map(|screen| {
                motion::screen_enter(
                    ("screen", self.generation),
                    div().absolute().inset_0().child(screen),
                    point(px(0.), px(6.)),
                    cx,
                )
            }))
            .children(alert)
            .children(toast);
        chrome::window_frame(content, window, cx)
    }
}
