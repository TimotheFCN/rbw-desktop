//! The lock screen: who the account is, its master password, Touch ID where
//! it can unlock, and the other accounts. Where Touch ID can unlock, the
//! screen asks for it by itself when it shows in the window in use
//! ([`AutoUnlock`]), and its button is the main one.

use gpui::base::input::{InputEvent, InputState};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement as _, Render, Styled as _, Task, Window, div, px,
};
use rbw_desktop_core::{Account, AccountId, Error, LockState, Secret};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Opening, app_icon, button, chrome, field_error,
    motion, screen_title, text_field,
};

use crate::accounts::{self, AccountMenu, Request};
use crate::backend;
use crate::touch_id::AutoUnlock;

/// An unlock under way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Unlocking {
    Password,
    TouchId,
}

pub struct LockScreen {
    account: Account,
    accounts: Vec<Account>,
    password: Entity<InputState>,
    problem: Option<&'static str>,
    /// Counts wrong passwords, each of which shakes the field.
    wrong: usize,
    busy: Option<Unlocking>,
    /// A password submitted while Touch ID's prompt was up, tried once
    /// that ends without unlocking.
    password_waits: bool,
    auto: AutoUnlock,
    /// Whether the password unlocked the account, and the screen waits for
    /// the vault to replace it.
    unlocked: bool,
    menu_open: bool,
    menu_focus: FocusHandle,
    task: Task<()>,
}

impl EventEmitter<Request> for LockScreen {}

impl LockScreen {
    /// `locked_here` says the person locked the account in this window,
    /// after which Touch ID waits for the window to come forward again.
    pub fn new(
        account: Account,
        accounts: Vec<Account>,
        locked_here: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let password = cx.new(|cx| {
            InputState::new(window, cx)
                .masked(true)
                .placeholder("Master password")
        });
        cx.subscribe_in(
            &password,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.submit(window, cx),
                InputEvent::Change => {
                    if this.problem.take().is_some() {
                        cx.notify();
                    }
                }
                InputEvent::Focus | InputEvent::Blur => {}
            },
        )
        .detach();
        password.update(cx, |password, cx| password.focus(window, cx));
        cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() {
                this.auto.activated();
                this.start_touch_id_if_wanted(window, cx);
            } else {
                this.auto.deactivated();
            }
        })
        .detach();
        // Once the screen is in the window.
        cx.defer_in(window, |this, window, cx| {
            this.start_touch_id_if_wanted(window, cx)
        });
        Self {
            account,
            accounts,
            password,
            problem: None,
            wrong: 0,
            busy: None,
            password_waits: false,
            auto: AutoUnlock::new(locked_here),
            unlocked: false,
            menu_open: false,
            menu_focus: cx.focus_handle(),
            task: Task::ready(()),
        }
    }

    pub fn account(&self) -> &AccountId {
        &self.account.id
    }

    pub fn set_account(
        &mut self,
        account: Account,
        accounts: Vec<Account>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Unlocked, then locked again before the vault showed, as by the
        // lock timeout or an agent that restarted: try again.
        if self.unlocked && account.state == LockState::Locked {
            self.unlocked = false;
            self.busy = None;
            cx.notify();
        }
        // Another account's screen starts empty: what was typed was for
        // the one before.
        if account.id != self.account.id {
            self.password
                .update(cx, |password, cx| password.set_value("", window, cx));
            self.problem = None;
            self.password_waits = false;
        }
        if account != self.account || accounts != self.accounts {
            self.account = account;
            self.accounts = accounts;
            cx.notify();
        }
        // Touch ID may have become ready.
        self.start_touch_id_if_wanted(window, cx);
    }

    /// Return in the field: the password typed, or Touch ID when there is
    /// none and it can unlock.
    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.password.read(cx).value().is_empty() && self.account.touch_id_ready {
            self.unlock_with_touch_id(window, cx);
        } else {
            self.unlock(window, cx);
        }
    }

    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = Secret::new(self.password.read(cx).value().to_string());
        if password.is_empty() {
            return;
        }
        match self.busy {
            Some(Unlocking::Password) => return,
            // Kept, and tried if Touch ID doesn't unlock.
            Some(Unlocking::TouchId) => {
                self.password_waits = true;
                cx.notify();
                return;
            }
            None => {}
        }
        let id = self.account.id.clone();
        self.run(
            Unlocking::Password,
            move |backend| backend.unlock(&id, &password),
            window,
            cx,
        );
    }

    fn unlock_with_touch_id(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy.is_some() {
            return;
        }
        let id = self.account.id.clone();
        self.run(
            Unlocking::TouchId,
            move |backend| backend.unlock_with_touch_id(&id),
            window,
            cx,
        );
    }

    /// Asks for Touch ID if the screen shows in the window in use and
    /// nothing holds it back.
    fn start_touch_id_if_wanted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.unlocked
            && self
                .auto
                .should_start(self.account.touch_id_ready, window.is_window_active())
        {
            self.unlock_with_touch_id(window, cx);
        }
    }

    fn run(
        &mut self,
        unlocking: Unlocking,
        call: impl FnOnce(&dyn rbw_desktop_core::Backend) -> rbw_desktop_core::Result<()>
        + Send
        + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = Some(unlocking);
        self.auto.started(unlocking == Unlocking::TouchId);
        self.problem = None;
        cx.notify();
        // Touch ID waits on the person, then on the server.
        self.task = match unlocking {
            Unlocking::TouchId => backend::request_network_in(window, cx, call, Self::answered),
            Unlocking::Password => backend::request_in(window, cx, call, Self::answered),
        };
    }

    fn answered(
        &mut self,
        result: rbw_desktop_core::Result<()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            // The spinner keeps going until the vault replaces this screen.
            Ok(()) => {
                self.unlocked = true;
                self.password_waits = false;
                self.password
                    .update(cx, |password, cx| password.set_value("", window, cx));
                crate::accounts::refresh(cx);
            }
            Err(error) => {
                self.busy = None;
                self.auto.ended();
                if std::mem::take(&mut self.password_waits) {
                    self.unlock(window, cx);
                } else {
                    self.fail(&error, window, cx);
                }
            }
        }
        cx.notify();
    }

    fn fail(&mut self, error: &Error, window: &mut Window, cx: &mut Context<Self>) {
        self.problem = match error {
            Error::TouchIdCanceled => None,
            Error::TouchIdUnavailable => Some("Touch ID can't unlock now. Use your password."),
            error => Some(backend::describe(error)),
        };
        if *error == Error::WrongPassword {
            self.wrong += 1;
            self.password.update(cx, |password, cx| {
                password.focus(window, cx);
                password.select_all(window, cx);
            });
        }
        // Another program may have signed the account out or unlocked it.
        if matches!(error, Error::NotSignedIn | Error::AccountNotFound) {
            crate::accounts::refresh(cx);
        }
    }
}

impl AccountMenu for LockScreen {
    fn menu_open(&self) -> bool {
        self.menu_open
    }

    fn set_menu_open(&mut self, open: bool) {
        self.menu_open = open;
    }

    fn menu_focus(&self) -> &FocusHandle {
        &self.menu_focus
    }

    fn menu_return(&self, cx: &App) -> FocusHandle {
        self.password.read(cx).focus_handle(cx)
    }
}

impl Focusable for LockScreen {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.password.read(cx).focus_handle(cx)
    }
}

impl Render for LockScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shake = motion::shake("lock-password", self.wrong, window, cx);
        let theme = cx.theme();
        let dim = theme.colors.text_secondary;
        let name = accounts::name(&self.account);

        // The name opens the account menu: other accounts, another to add,
        // or signing out of this one.
        let who = div()
            .relative()
            .text_color(dim)
            .child(
                button("account")
                    .label(name.clone())
                    .trailing_icon(IconName::ChevronsUpDown)
                    .style(ButtonStyle::Flat)
                    .size(px(24.))
                    .accessibility_label(format!("{name}, accounts"))
                    .on_click(
                        cx.listener(|this, _, window, cx| accounts::toggle(this, window, cx)),
                    ),
            )
            .child(accounts::menu(
                self,
                &self.accounts,
                &self.account.id,
                Opening::BelowCentered,
                cx,
            ))
            .into_any_element();

        let password_busy = self.busy == Some(Unlocking::Password) || self.password_waits;
        let field = text_field("password", &self.password, "Master password").trailing(
            button("unlock")
                .icon(IconName::ArrowRight)
                .style(ButtonStyle::Primary)
                .size(px(28.))
                .busy(password_busy)
                .accessibility_label("Unlock")
                .on_click(cx.listener(|this, _, window, cx| this.unlock(window, cx))),
        );

        // The main way in where it can unlock: filled, under the field,
        // which stays there for the password.
        let touch_id = self.account.touch_id_ready.then(|| {
            button("touch-id")
                .label("Unlock with Touch ID")
                .icon(IconName::TouchId)
                .style(ButtonStyle::Primary)
                .wide()
                .busy(self.busy == Some(Unlocking::TouchId))
                .on_click(cx.listener(|this, _, window, cx| this.unlock_with_touch_id(window, cx)))
        });

        let content = div()
            .flex()
            .flex_col()
            .items_center()
            .w(px(270.))
            .child(app_icon(cx))
            .child(screen_title("Welcome back").mt(px(16.)))
            .child(div().mt(px(3.)).h(px(24.)).flex().items_center().child(who))
            .child(
                div()
                    .mt(px(16.))
                    .w_full()
                    .relative()
                    .left(shake)
                    .child(field),
            )
            // The message has a line of its own, so that it moves nothing
            // when it appears.
            .child(
                div()
                    .h(px(30.))
                    .flex()
                    .items_center()
                    .text_center()
                    .children(self.problem.map(|problem| field_error(problem, cx))),
            )
            .children(touch_id);
        chrome::plain_screen("lock", content, window, cx)
    }
}
