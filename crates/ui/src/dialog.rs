//! The alert that asks before something that can't be undone, such as
//! deleting an item or signing out.
//!
//! Present a [`ConfirmDialog`] in a [`SheetSlot::alert`](crate::SheetSlot::alert)
//! and dismiss it on its [`ConfirmEvent`]. Return confirms and Escape cancels.

use gpui::base::FocusTrapElement as _;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Render, SharedString, StatefulInteractiveElement as _,
    Styled, Window, actions, div, px, relative,
};

use crate::button::{ButtonStyle, button};
use crate::theme::ActiveTheme;

actions!(confirm_dialog, [Confirm, Cancel]);

const CONTEXT: &str = "ConfirmDialog";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Cancel, Some(CONTEXT)),
        KeyBinding::new("enter", Confirm, Some(CONTEXT)),
    ]);
}

/// What to ask. The message says what will happen, in a sentence or two.
pub struct Confirmation {
    pub title: SharedString,
    pub message: SharedString,
    /// The verb on the confirming button, such as "Delete".
    pub confirm: SharedString,
    /// Whether confirming removes something, which colours the button red.
    pub destructive: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmEvent {
    Confirmed,
    Cancelled,
}

pub struct ConfirmDialog {
    confirmation: Confirmation,
    focus: FocusHandle,
}

impl EventEmitter<ConfirmEvent> for ConfirmDialog {}

impl ConfirmDialog {
    pub fn new(confirmation: Confirmation, cx: &mut Context<Self>) -> Self {
        Self {
            confirmation,
            focus: cx.focus_handle(),
        }
    }

    fn confirm(&mut self, _: &Confirm, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ConfirmEvent::Confirmed);
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ConfirmEvent::Cancelled);
    }
}

impl Focusable for ConfirmDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for ConfirmDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let dim = theme.colors.text_secondary;
        let confirmation = &self.confirmation;
        let style = if confirmation.destructive {
            ButtonStyle::Destructive
        } else {
            ButtonStyle::Primary
        };
        div()
            .id("confirm-dialog")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .role(gpui::Role::AlertDialog)
            .aria_label(confirmation.title.clone())
            .flex()
            .flex_col()
            .items_center()
            .text_center()
            .line_height(relative(1.35))
            .child(
                div()
                    .text_size(px(14.))
                    .font_weight(FontWeight::BOLD)
                    .child(confirmation.title.clone()),
            )
            .child(
                div()
                    .mt(px(6.))
                    .text_size(theme.metrics.small_text_size)
                    .text_color(dim)
                    .child(confirmation.message.clone()),
            )
            .child(
                div()
                    .mt(px(16.))
                    .w_full()
                    .flex()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .child(button("cancel").label("Cancel").wide().on_click(
                                cx.listener(|this, _, window, cx| this.cancel(&Cancel, window, cx)),
                            )),
                    )
                    .child(
                        div().flex_1().child(
                            button("confirm")
                                .label(confirmation.confirm.clone())
                                .style(style)
                                .wide()
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.confirm(&Confirm, window, cx)
                                })),
                        ),
                    ),
            )
            .focus_trap("confirm-dialog", &self.focus)
    }
}
