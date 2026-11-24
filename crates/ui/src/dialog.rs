//! The alert that asks before something that can't be undone, such as
//! deleting an item or signing out.
//!
//! Present a [`ConfirmDialog`] in a [`SheetSlot::alert`](crate::SheetSlot::alert),
//! focus it, and dismiss it on its [`ConfirmEvent`]. Its default button has
//! the keyboard, so Return presses it: the confirming one, or Cancel when
//! confirming removes something, as macOS and GNOME do. Escape cancels.

use gpui::base::FocusTrapElement as _;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Render, SharedString, StatefulInteractiveElement as _,
    Styled, Window, actions, div, px, relative,
};

use crate::button::{ButtonStyle, button};
use crate::theme::ActiveTheme;

actions!(confirm_dialog, [Cancel]);

const CONTEXT: &str = "ConfirmDialog";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new("escape", Cancel, Some(CONTEXT))]);
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
    cancel_focus: FocusHandle,
    confirm_focus: FocusHandle,
}

impl EventEmitter<ConfirmEvent> for ConfirmDialog {}

impl ConfirmDialog {
    pub fn new(confirmation: Confirmation, cx: &mut Context<Self>) -> Self {
        Self {
            confirmation,
            focus: cx.focus_handle(),
            cancel_focus: cx.focus_handle(),
            confirm_focus: cx.focus_handle(),
        }
    }

    fn cancel(&mut self, _: &Cancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(ConfirmEvent::Cancelled);
    }
}

/// The default button.
impl Focusable for ConfirmDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        if self.confirmation.destructive {
            self.cancel_focus.clone()
        } else {
            self.confirm_focus.clone()
        }
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
                        div().flex_1().child(
                            button("cancel")
                                .label("Cancel")
                                .wide()
                                .track_focus(&self.cancel_focus)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.cancel(&Cancel, window, cx)
                                })),
                        ),
                    )
                    .child(
                        div().flex_1().child(
                            button("confirm")
                                .label(confirmation.confirm.clone())
                                .style(style)
                                .wide()
                                .track_focus(&self.confirm_focus)
                                .on_click(
                                    cx.listener(|_, _, _, cx| cx.emit(ConfirmEvent::Confirmed)),
                                ),
                        ),
                    ),
            )
            .focus_trap("confirm-dialog", &self.focus)
    }
}
