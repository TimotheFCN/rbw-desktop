//! The lock screen: who the account is, its master password, Touch ID where
//! it can unlock, and the other accounts.

use gpui::base::input::{InputEvent, InputState};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    IntoElement, ParentElement as _, Render, Styled as _, Task, Window, div, px,
};
use rbw_desktop_core::{Account, AccountId, Error, Secret};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Opening, app_icon, button, field_error, motion,
    text_field,
};

use crate::accounts::{self, AccountMenu, Request};
use crate::backend;
use crate::window::plain_screen;

pub struct LockScreen {
    account: Account,
    accounts: Vec<Account>,
    password: Entity<InputState>,
    problem: Option<&'static str>,
    /// Counts wrong passwords, each of which shakes the field.
    wrong: usize,
    busy: bool,
    menu_open: bool,
    menu_focus: FocusHandle,
    task: Task<()>,
}

impl EventEmitter<Request> for LockScreen {}

impl LockScreen {
    pub fn new(
        account: Account,
        accounts: Vec<Account>,
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
                InputEvent::PressEnter { .. } => this.unlock(window, cx),
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
        Self {
            account,
            accounts,
            password,
            problem: None,
            wrong: 0,
            busy: false,
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
        cx: &mut Context<Self>,
    ) {
        if account != self.account || accounts != self.accounts {
            self.account = account;
            self.accounts = accounts;
            cx.notify();
        }
    }

    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = Secret::new(self.password.read(cx).value().to_string());
        if self.busy || password.is_empty() {
            return;
        }
        let id = self.account.id.clone();
        self.run(move |backend| backend.unlock(&id, &password), window, cx);
    }

    fn unlock_with_touch_id(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let id = self.account.id.clone();
        self.run(move |backend| backend.unlock_with_touch_id(&id), window, cx);
    }

    fn run(
        &mut self,
        call: impl FnOnce(&dyn rbw_desktop_core::Backend) -> rbw_desktop_core::Result<()>
        + Send
        + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.problem = None;
        cx.notify();
        self.task = backend::request_in(window, cx, call, |this, result, window, cx| {
            match result {
                // The spinner keeps going until the vault replaces this screen.
                Ok(()) => cx.emit(Request::Refresh),
                Err(error) => {
                    this.busy = false;
                    this.fail(&error, window, cx);
                }
            }
            cx.notify();
        });
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
            cx.emit(Request::Refresh);
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
        let several = self.accounts.len() > 1;

        // With other accounts to go to, the name opens the account menu.
        let who = if several {
            div()
                .relative()
                .text_color(dim)
                .child(
                    button("account")
                        .label(name.clone())
                        .trailing_icon(IconName::ChevronsUpDown)
                        .style(ButtonStyle::Flat)
                        .size(px(24.))
                        .accessibility_label(format!("{name}, switch account"))
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
                .into_any_element()
        } else {
            div().text_color(dim).child(name).into_any_element()
        };

        let field = text_field("password", &self.password, "Master password").trailing(
            button("unlock")
                .icon(IconName::ArrowRight)
                .style(ButtonStyle::Primary)
                .size(px(28.))
                .busy(self.busy)
                .accessibility_label("Unlock")
                .on_click(cx.listener(|this, _, window, cx| this.unlock(window, cx))),
        );

        let touch_id = self.account.touch_id_ready.then(|| {
            button("touch-id")
                .label("Unlock with Touch ID")
                .icon(IconName::TouchId)
                .style(ButtonStyle::Link)
                .on_click(cx.listener(|this, _, window, cx| this.unlock_with_touch_id(window, cx)))
        });

        let content = div()
            .flex()
            .flex_col()
            .items_center()
            .w(px(270.))
            .child(app_icon(cx))
            .child(
                div()
                    .mt(px(16.))
                    .text_size(px(20.))
                    .font_weight(FontWeight::BOLD)
                    .child("Welcome back"),
            )
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
        plain_screen("lock", content, window, cx)
    }
}
