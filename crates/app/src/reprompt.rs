//! The master password again, for an item that asks for it before its
//! secrets are shown or changed (Bitwarden's "master password re-prompt").
//! It looks and behaves like the lock screen's field: Return or the arrow
//! unlocks, a wrong password shakes the field and says so.

use gpui::base::input::{InputEvent, InputState};
use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, IntoElement, ParentElement as _, Render,
    Styled as _, Task, Window, div, px,
};
use rbw_desktop_core::{AccountId, Error, ItemId, Secret};
use rbw_desktop_ui::{ButtonStyle, IconName, button, field_error, motion, text_field};

use crate::backend;

/// The item is unlocked: ask for it again.
pub struct Unlocked;

pub struct Reprompt {
    account: AccountId,
    item: ItemId,
    password: Entity<InputState>,
    problem: Option<&'static str>,
    /// Counts wrong passwords, each of which shakes the field.
    wrong: usize,
    busy: bool,
    task: Task<()>,
}

impl EventEmitter<Unlocked> for Reprompt {}

impl Reprompt {
    pub fn new(
        account: AccountId,
        item: ItemId,
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
        Self {
            account,
            item,
            password,
            problem: None,
            wrong: 0,
            busy: false,
            task: Task::ready(()),
        }
    }

    pub fn item(&self) -> &ItemId {
        &self.item
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.password
            .update(cx, |password, cx| password.focus(window, cx));
    }

    /// Clears any typed password and stops an in-flight unlock, as when the
    /// prompt is dismissed or the account locks.
    pub fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.task = Task::ready(());
        self.busy = false;
        self.problem = None;
        self.password
            .update(cx, |password, cx| password.set_value("", window, cx));
    }

    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let password = Secret::new(self.password.read(cx).value().to_string());
        if self.busy || password.is_empty() {
            return;
        }
        self.busy = true;
        self.problem = None;
        cx.notify();
        let (account, item) = (self.account.clone(), self.item.clone());
        self.task = backend::request_in(
            window,
            cx,
            move |backend| backend.unlock_item(&account, &item, &password),
            |this, result, window, cx| {
                this.busy = false;
                match result {
                    Ok(()) => {
                        this.password
                            .update(cx, |password, cx| password.set_value("", window, cx));
                        cx.emit(Unlocked);
                    }
                    Err(error) => {
                        this.problem = Some(backend::describe(&error));
                        if error == Error::WrongPassword {
                            this.wrong += 1;
                            this.password.update(cx, |password, cx| {
                                password.focus(window, cx);
                                password.select_all(window, cx);
                            });
                        }
                    }
                }
                cx.notify();
            },
        );
    }
}

impl Render for Reprompt {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shake = motion::shake(
            gpui::ElementId::Name(format!("reprompt-{}", self.item.0).into()),
            self.wrong,
            window,
            cx,
        );
        let field = text_field("reprompt-password", &self.password, "Master password").trailing(
            button("reprompt-unlock")
                .icon(IconName::ArrowRight)
                .style(ButtonStyle::Primary)
                .size(px(28.))
                .busy(self.busy)
                .accessibility_label("Unlock Item")
                .on_click(cx.listener(|this, _, window, cx| this.unlock(window, cx))),
        );
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(div().relative().left(shake).child(field))
            // The message has a line of its own, so that it moves nothing
            // when it appears.
            .child(
                div()
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(self.problem.map(|problem| field_error(problem, cx))),
            )
    }
}
