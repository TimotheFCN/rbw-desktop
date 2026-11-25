//! The app's window. It shows one screen at a time, chosen from the accounts
//! on this computer and their state: adding a first account, signing in,
//! the lock screen or the vault. It follows the shared [`Accounts`], so that
//! locking and signing in or out done elsewhere, in Settings, by the command
//! line or the lock timeout, show here too.

use std::time::Duration;

use gpui::{
    AnyElement, AnyView, AnyWindowHandle, App, AppContext as _, Bounds, Context, DisplayId, Entity,
    FocusHandle, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Render, Size,
    Styled as _, Subscription, Task, Window, WindowBounds, WindowOptions, actions, div, point,
    prelude::FluentBuilder as _, px, size,
};
use rbw_desktop_core::{
    APP_ID, APP_NAME, Account, AccountId, Error, ItemId, LockState, REQUIRED_RBW, WindowPlace,
};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, Theme, app_icon, button, chrome, motion, screen_title,
};

use crate::CloseWindow;
use crate::accounts::{self, Accounts, Refreshed, Request};
use crate::alerts::{self, Alerts};
use crate::backend;
use crate::command_line::OtherRbwNotice;
use crate::lock::LockScreen;
use crate::preferences::{self, Prefs};
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

/// Where rbw-ng is, for the screen that says it is missing.
const RBW_NG: &str = "https://github.com/TimotheFCN/rbw-ng";

/// The main window's size the first time, and the smallest it gets.
const SIZE: Size<Pixels> = size(px(880.), px(580.));
const MIN_SIZE: Size<Pixels> = size(px(680.), px(440.));
/// How long the window rests after a move or a resize before where it is
/// is saved.
const SAVE_PLACE_AFTER: Duration = Duration::from_millis(500);

/// Opens the main window, where it was last if that is still on a
/// display. `missing` says why there is no backend, such as rbw not being
/// installed; the window then says so and offers to look again.
pub fn open(missing: Option<Error>, cx: &mut App) -> Option<(AnyWindowHandle, Entity<MainWindow>)> {
    let mut options = WindowOptions {
        app_id: Some(APP_ID.into()),
        ..chrome::window_options("Vault", SIZE, MIN_SIZE, cx)
    };
    let saved = backend::ready(cx)
        .then(|| Prefs::global(cx).read(cx).get().window.clone())
        .flatten();
    if let Some((bounds, display)) = saved.and_then(|place| restored(place, cx)) {
        options.window_bounds = Some(bounds);
        options.display_id = display;
    }
    crate::dock::show();
    // gpui-base's Root around the view gives the window Tab and Shift-Tab.
    let window = gpui::open_window(options, cx, |window, cx| {
        cx.new(|cx| MainWindow::new(missing, window, cx))
    });
    match window {
        Ok((handle, main)) => Some((handle, main)),
        Err(error) => {
            log::error!("opening the window: {error:#}");
            None
        }
    }
}

/// Where a window saved at `place` opens: there, at least its smallest
/// size, unless no display shows it any more, as after one was unplugged.
fn restored(place: WindowPlace, cx: &App) -> Option<(WindowBounds, Option<DisplayId>)> {
    let bounds = Bounds::new(
        point(px(place.x), px(place.y)),
        size(
            px(place.width).max(MIN_SIZE.width),
            px(place.height).max(MIN_SIZE.height),
        ),
    );
    let (shown, display) = if cfg!(target_os = "macos") {
        // The place counts from its display's top-left corner: that
        // display's if it is still plugged in, else the primary one's,
        // where the window then opens.
        let saved = place.display.as_deref().and_then(|saved| {
            cx.displays()
                .into_iter()
                .find(|display| display.uuid().is_ok_and(|uuid| uuid.to_string() == saved))
        });
        let display = saved.or_else(|| cx.primary_display());
        let shown = display
            .as_ref()
            .is_some_and(|display| display.bounds().intersects(&bounds));
        (shown, display.map(|display| display.id()))
    } else {
        // The place counts on the whole screen, as the displays' bounds do.
        let shown = cx
            .displays()
            .iter()
            .any(|display| display.bounds().intersects(&bounds));
        (shown, None)
    };
    let bounds = if place.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    };
    shown.then_some((bounds, display))
}

/// Where a window is, on `display`, to open there again; full screen, it
/// opens as it was before.
fn place(bounds: WindowBounds, display: Option<String>) -> WindowPlace {
    let (rect, maximized) = match bounds {
        WindowBounds::Windowed(rect) | WindowBounds::Fullscreen(rect) => (rect, false),
        WindowBounds::Maximized(rect) => (rect, true),
    };
    WindowPlace {
        x: rect.origin.x.into(),
        y: rect.origin.y.into(),
        width: rect.size.width.into(),
        height: rect.size.height.into(),
        maximized,
        display,
    }
}

/// Brings the main window forward and runs `then` on it, opening it first
/// if it was closed, as it is while the app waits in the menu bar.
pub fn with_main_window(
    cx: &mut App,
    then: impl FnOnce(&mut MainWindow, &mut Window, &mut Context<MainWindow>) + 'static,
) {
    for handle in cx.windows() {
        let Some(root) = handle.downcast::<gpui::base::Root>() else {
            continue;
        };
        let Some(main) = root
            .read(cx)
            .ok()
            .and_then(|root| root.view().clone().downcast::<MainWindow>().ok())
        else {
            continue;
        };
        root.update(cx, |_, window, cx| {
            window.activate_window();
            main.update(cx, |main, cx| then(main, window, cx));
        })
        .ok();
        return;
    }
    if let Some((handle, main)) = open(None, cx) {
        // Once the window has its view.
        cx.defer(move |cx| {
            handle
                .update(cx, |_, window, cx| {
                    main.update(cx, |main, cx| then(main, window, cx))
                })
                .ok();
        });
    }
}

enum Screen {
    /// Until the accounts are known.
    Loading,
    /// There is no backend, such as when rbw is missing, or the accounts
    /// can't be read.
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
    /// `None` until there is a backend to ask. Its current account is the
    /// one shown, or the one to go back to while adding another.
    accounts: Option<Entity<Accounts>>,
    screen: Screen,
    /// Names the screen shown, so that each new one plays its entrance.
    generation: usize,
    /// The screen being replaced, which fades out before the new one fades
    /// in.
    leaving: Option<AnyView>,
    alerts: Entity<Alerts>,
    /// Says when another rbw would lock the vault, once there is a backend
    /// to ask.
    other_rbw: Option<Entity<OtherRbwNotice>>,
    /// The window's own focus, so that its shortcuts work while no screen
    /// has the keyboard.
    focus: FocusHandle,
    /// Whether the next accounts to arrive replace even the sign-in that
    /// just finished.
    show_next: bool,
    /// Whether the screen saying the backend is missing is looking again.
    retrying: bool,
    /// An item to show once its account's vault shows, such as one the
    /// menu bar panel opened while the account was locked.
    reveal: Option<(AccountId, ItemId)>,
    /// The account the person just locked in this window, whose lock
    /// screen doesn't ask for Touch ID until the window comes forward
    /// again.
    locked_here: Option<AccountId>,
    _screen_events: Option<Subscription>,
    _accounts_events: Option<Subscription>,
    leave_task: Task<()>,
    retry_task: Task<()>,
    /// Saves where the window is once it rests.
    place_task: Task<()>,
}

impl MainWindow {
    fn new(missing: Option<Error>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_appearance(window, |_, window, cx| {
            Theme::follow_window_appearance(window, cx);
        })
        .detach();
        cx.observe_window_bounds(window, |this, window, cx| this.save_place_later(window, cx))
            .detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        let mut this = Self {
            accounts: None,
            screen: Screen::Loading,
            generation: 0,
            leaving: None,
            alerts: Alerts::new(window, cx),
            other_rbw: None,
            focus,
            show_next: false,
            retrying: false,
            reveal: None,
            locked_here: None,
            _screen_events: None,
            _accounts_events: None,
            leave_task: Task::ready(()),
            retry_task: Task::ready(()),
            place_task: Task::ready(()),
        };
        match missing {
            Some(error) => this.screen = Screen::Unavailable(error),
            None => this.follow_accounts(window, cx),
        }
        this
    }

    fn follow_accounts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.other_rbw.is_none() {
            self.other_rbw = Some(cx.new(OtherRbwNotice::new));
        }
        // They may have changed while no window asked for them.
        accounts::refresh(cx);
        let accounts = Accounts::global(cx);
        self._accounts_events = Some(cx.subscribe_in(
            &accounts,
            window,
            |this, _, _: &Refreshed, window, cx| this.accounts_refreshed(window, cx),
        ));
        self.accounts = Some(accounts);
        self.accounts_refreshed(window, cx);
    }

    /// Saves where the window is once it has rested there, for the next
    /// launch.
    fn save_place_later(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !backend::ready(cx) {
            return;
        }
        self.place_task = cx.spawn_in(window, async move |_, cx| {
            cx.background_executor().timer(SAVE_PLACE_AFTER).await;
            cx.update(|window, cx| {
                let display = window.display(cx).and_then(|display| display.uuid().ok());
                let place = place(window.window_bounds(), display.map(|uuid| uuid.to_string()));
                preferences::update(cx, move |preferences| {
                    preferences.window = Some(place.clone())
                });
            })
            .ok();
        });
    }

    fn account_list(&self, cx: &App) -> Vec<Account> {
        self.accounts
            .as_ref()
            .map(|accounts| accounts.read(cx).list().to_vec())
            .unwrap_or_default()
    }

    fn accounts_refreshed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(accounts) = &self.accounts else {
            return;
        };
        match accounts.read(cx).load() {
            backend::Load::Loading => {}
            backend::Load::Failed(error) => {
                if matches!(self.screen, Screen::Loading | Screen::Unavailable(_)) {
                    let error = error.clone();
                    self.retrying = false;
                    self.show(Screen::Unavailable(error), window, cx);
                }
            }
            backend::Load::Ready(_) => {
                self.retrying = false;
                if std::mem::take(&mut self.show_next) {
                    self.show_current(window, cx);
                } else {
                    self.settle(window, cx);
                }
            }
        }
    }

    /// Shows the screen the current account's state calls for, or keeps the
    /// one on screen up to date. Adding an account stays on screen until it
    /// is done or abandoned.
    fn settle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Screen::Adding(sign_in) = &self.screen {
            let accounts = self.account_list(cx);
            sign_in.update(cx, |sign_in, cx| sign_in.set_accounts(accounts, cx));
            return;
        }
        self.show_current(window, cx);
    }

    /// Makes `id` the account shown from now on, in every window.
    fn choose(&self, id: Option<AccountId>, cx: &mut App) {
        if let Some(accounts) = &self.accounts {
            accounts.update(cx, |accounts, cx| accounts.choose(id, cx));
        }
    }

    fn show_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let accounts = self.account_list(cx);
        let current = self
            .accounts
            .as_ref()
            .and_then(|accounts| accounts.read(cx).current().cloned());
        let Some(account) = current else {
            self.choose(None, cx);
            self.add_account(window, cx);
            return;
        };
        self.choose(Some(account.id.clone()), cx);
        if !self.screen.suits(account.state) || !self.shows_account(&account.id, cx) {
            let screen = self.screen_for(account, accounts, window, cx);
            self.show(screen, window, cx);
            return;
        }
        match &self.screen {
            Screen::Lock(lock) => {
                lock.update(cx, |lock, cx| {
                    lock.set_account(account, accounts, window, cx)
                });
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
        accounts: Vec<Account>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Screen {
        match account.state {
            LockState::SignedOut => {
                let view = cx.new(|cx| SignIn::existing(account, accounts, window, cx));
                self.listen(&view, window, cx);
                Screen::SignIn(view)
            }
            LockState::Locked => {
                let locked_here = self
                    .locked_here
                    .take_if(|locked| *locked == account.id)
                    .is_some();
                let view = cx.new(|cx| LockScreen::new(account, accounts, locked_here, window, cx));
                self.listen(&view, window, cx);
                Screen::Lock(view)
            }
            LockState::Unlocked => {
                let reveal = self
                    .reveal
                    .take_if(|(id, _)| *id == account.id)
                    .map(|(_, item)| item);
                let view = cx.new(|cx| VaultView::new(account, accounts, window, cx));
                if let Some(item) = reveal {
                    view.update(cx, |vault, cx| vault.reveal(item, window, cx));
                }
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
            Request::Show(id) => self.show_account(id, window, cx),
            Request::AddAccount => self.add_account(window, cx),
            Request::CancelAdd => self.show_current(window, cx),
            Request::Locked(id) => self.locked_here = Some(id),
            Request::SignedIn(id) => {
                self.choose(Some(id), cx);
                self.show_next = true;
                accounts::refresh(cx);
            }
            Request::SignOut(id) => {
                let Some(account) = self.account_list(cx).into_iter().find(|a| a.id == id) else {
                    return;
                };
                accounts::sign_out(&account, window, cx, |error, window, cx| {
                    alerts::toast(backend::describe(&error), window, cx);
                });
            }
        }
    }

    /// Shows this account, in whatever state it is.
    pub fn show_account(&mut self, id: AccountId, window: &mut Window, cx: &mut Context<Self>) {
        self.choose(Some(id), cx);
        if !matches!(self.screen, Screen::Unavailable(_)) {
            self.show_current(window, cx);
        }
        accounts::refresh(cx);
    }

    /// Shows an item of an account, such as one the menu bar panel can't
    /// copy from without the master password, which it then asks for.
    pub fn open_item(
        &mut self,
        account: AccountId,
        item: ItemId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.reveal = Some((account.clone(), item));
        self.show_account(account.clone(), window, cx);
        if let Screen::Vault(vault) = &self.screen
            && vault.read(cx).account() == &account
            && let Some((_, item)) = self.reveal.take()
        {
            vault.update(cx, |vault, cx| vault.reveal(item, window, cx));
        }
    }

    pub fn add_account(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.screen, Screen::Adding(_) | Screen::Unavailable(_)) {
            return;
        }
        let accounts = self.account_list(cx);
        let view = cx.new(|cx| SignIn::add(accounts, window, cx));
        self.listen(&view, window, cx);
        self.show(Screen::Adding(view), window, cx);
    }

    fn add_account_action(&mut self, _: &AddAccount, window: &mut Window, cx: &mut Context<Self>) {
        self.add_account(window, cx);
    }

    /// Replaces the screen: the old one fades out, then the new one in.
    fn show(&mut self, screen: Screen, window: &mut Window, cx: &mut Context<Self>) {
        let old = std::mem::replace(&mut self.screen, screen);
        self.leaving = old.view();
        self.generation += 1;
        // Nothing of a vault stays behind it once it has gone.
        self.alerts.update(cx, |alerts, cx| alerts.dismiss(cx));
        self.leave_task = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(motion::SCREEN).await;
            this.update(cx, |this, cx| {
                this.leaving = None;
                cx.notify();
            })
            .ok();
        });
        cx.notify();
    }

    /// Looks again for what was missing: rbw, or the accounts.
    fn retry(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.retrying {
            return;
        }
        self.retrying = true;
        cx.notify();
        if self.accounts.is_some() {
            accounts::refresh(cx);
            return;
        }
        let connecting = cx.background_spawn(async { backend::connect() });
        self.retry_task = cx.spawn_in(window, async move |this, cx| {
            let result = connecting.await;
            this.update_in(cx, |this, window, cx| {
                match result {
                    Ok(backend) => {
                        crate::start(backend, cx);
                        this.follow_accounts(window, cx);
                    }
                    Err(error) => {
                        this.retrying = false;
                        this.screen = Screen::Unavailable(error);
                    }
                }
                cx.notify();
            })
            .ok();
        });
    }

    fn render_unavailable(
        &self,
        error: &Error,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let dim = cx.theme().colors.text_secondary;
        let (title, message) = match error {
            Error::RbwNotFound => (
                "rbw isn’t installed".to_string(),
                format!(
                    "{APP_NAME} keeps your vault with rbw-ng, the Bitwarden client it is \
                     built on. Install it, then try again."
                ),
            ),
            Error::RbwTooOld { found, required } => (
                "rbw needs updating".to_string(),
                format!(
                    "The rbw on this computer is version {found}. {APP_NAME} needs rbw-ng \
                     {required} or later."
                ),
            ),
            error => (
                "Your accounts can’t be read".to_string(),
                backend::describe(error).to_string(),
            ),
        };
        let missing = matches!(error, Error::RbwNotFound | Error::RbwTooOld { .. });
        let content = div()
            .flex()
            .flex_col()
            .items_center()
            .w(px(320.))
            .text_center()
            .child(app_icon(cx))
            .child(screen_title(title).mt(px(16.)))
            .child(div().mt(px(6.)).text_color(dim).child(message))
            .child(
                div()
                    .mt(px(20.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(px(10.))
                    .child(
                        button("retry")
                            .label("Try Again")
                            .style(ButtonStyle::Primary)
                            .busy(self.retrying)
                            .on_click(cx.listener(|this, _, window, cx| this.retry(window, cx))),
                    )
                    .when(missing, |d| {
                        d.child(
                            button("get-rbw")
                                .label(format!("Get rbw-ng {REQUIRED_RBW}"))
                                .style(ButtonStyle::Link)
                                .on_click(|_, _, cx| cx.open_url(RBW_NG)),
                        )
                    }),
            );
        chrome::plain_screen("unavailable", content, window, cx)
    }
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
        let content = div()
            .key_context(CONTEXT)
            // Screens hold the keyboard themselves; a click on an empty
            // part of one mustn't take it from them.
            .when(
                matches!(self.screen, Screen::Loading | Screen::Unavailable(_)),
                |d| d.track_focus(&self.focus),
            )
            .on_action(cx.listener(Self::add_account_action))
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .relative()
            .size_full()
            .children(leaving.map(|view| {
                motion::screen_leave(
                    ("screen-leaving", self.generation),
                    div().absolute().inset_0().child(view),
                )
            }))
            .children(current.map(|screen| {
                motion::screen_replace(
                    ("screen", self.generation),
                    div().absolute().inset_0().child(screen),
                    point(px(0.), px(6.)),
                )
            }))
            .children(self.other_rbw.clone())
            .child(self.alerts.clone());
        chrome::window_frame(content, window, cx)
    }
}
