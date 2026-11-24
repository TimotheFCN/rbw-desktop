//! A short message floating at the bottom of a window, for something that went
//! wrong in the background, like a copy that failed.
//!
//! A window keeps a [`ToastSlot`], calls [`ToastSlot::show`] and renders the
//! slot as one of its last children. The message leaves on its own.

use std::time::Duration;

use gpui::{
    AnyElement, App, BoxShadow, Context, IntoElement, ParentElement, SharedString, Styled, Task,
    Window, div, px,
};

use crate::motion as m;
use crate::theme::ActiveTheme;

const VISIBLE_FOR: Duration = Duration::from_secs(4);

#[derive(Default)]
pub struct ToastSlot {
    message: Option<SharedString>,
    open: bool,
    _hide: Option<Task<()>>,
}

impl ToastSlot {
    /// Shows `message`, replacing any message on screen. `slot` finds this
    /// slot in the window's view again when the message is due to leave.
    pub fn show<V: 'static>(
        &mut self,
        message: impl Into<SharedString>,
        slot: fn(&mut V) -> &mut ToastSlot,
        cx: &mut Context<V>,
    ) {
        self.message = Some(message.into());
        self.open = true;
        // Replacing the task cancels the previous message's timer.
        self._hide = Some(cx.spawn(async move |view, cx| {
            cx.background_executor().timer(VISIBLE_FOR).await;
            view.update(cx, |view, cx| {
                slot(view).open = false;
                cx.notify();
            })
            .ok();
        }));
        cx.notify();
    }

    pub fn render(&mut self, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let presence = m::presence("toast", self.open, window, cx);
        if !presence.should_render() {
            self.message = None;
            return None;
        }
        let message = self.message.clone()?;
        let p = presence.progress;
        let colors = &cx.theme().colors;
        Some(
            div()
                .absolute()
                .bottom(px(16.) + px(10.) * p)
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(
                    div()
                        .px(px(16.))
                        .py(px(9.))
                        .rounded_full()
                        .bg(colors.text)
                        .text_color(colors.window)
                        .opacity(p)
                        .shadow(vec![
                            BoxShadow::new(px(0.), px(8.), colors.shadow).blur_radius(px(24.)),
                        ])
                        .child(message),
                )
                .into_any_element(),
        )
    }
}
