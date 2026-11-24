//! Asking before something that can't be undone, and saying briefly that
//! something happened, in whichever window asked. Each window view keeps
//! its [`Alerts`] and draws them as its last child; any view in the window
//! reaches them through [`confirm`] and [`toast`].

use std::collections::HashMap;

use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, Global, IntoElement,
    ParentElement as _, Render, Styled as _, Subscription, WeakEntity, Window, WindowId, div, px,
};
use rbw_desktop_ui::{ConfirmDialog, ConfirmEvent, Confirmation, SheetSlot, ToastSlot};

pub struct Alerts {
    slot: SheetSlot,
    /// Where the keyboard was before the alert, to go back there after.
    focus_return: Option<FocusHandle>,
    toast: ToastSlot,
    _dialog_events: Option<Subscription>,
}

/// Each window's alerts.
#[derive(Default)]
struct ByWindow(HashMap<WindowId, WeakEntity<Alerts>>);

impl Global for ByWindow {}

impl Alerts {
    /// The alerts of `window`, for its view to keep and draw.
    pub fn new(window: &Window, cx: &mut App) -> Entity<Self> {
        let alerts = cx.new(|_| Self {
            slot: SheetSlot::alert(),
            focus_return: None,
            toast: ToastSlot::default(),
            _dialog_events: None,
        });
        let by_window = &mut cx.default_global::<ByWindow>().0;
        by_window.retain(|_, alerts| alerts.upgrade().is_some());
        by_window.insert(window.window_handle().window_id(), alerts.downgrade());
        alerts
    }

    fn of(window: &Window, cx: &App) -> Option<Entity<Self>> {
        cx.try_global::<ByWindow>()?
            .0
            .get(&window.window_handle().window_id())?
            .upgrade()
    }

    /// Takes the alert away, such as with the screen it was about.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.slot.dismiss();
        cx.notify();
    }

    fn confirm(
        &mut self,
        confirmation: Confirmation,
        window: &mut Window,
        cx: &mut Context<Self>,
        then: impl FnOnce(&mut Window, &mut App) + 'static,
    ) {
        let dialog = cx.new(|cx| ConfirmDialog::new(confirmation, cx));
        let mut then = Some(then);
        self._dialog_events =
            Some(
                cx.subscribe_in(&dialog, window, move |this, _, event, window, cx| {
                    this.slot.dismiss();
                    if let Some(focus) = this.focus_return.take() {
                        window.focus(&focus, cx);
                    }
                    if *event == ConfirmEvent::Confirmed
                        && let Some(then) = then.take()
                    {
                        window.defer(cx, then);
                    }
                    cx.notify();
                }),
            );
        self.focus_return = window.focused(cx);
        window.focus(&dialog.read(cx).focus_handle(cx), cx);
        self.slot.present(dialog);
        cx.notify();
    }
}

impl Render for Alerts {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let alert = self.slot.render(px(300.), window, cx);
        let toast = self.toast.render(window, cx);
        // Over the whole window, without taking a click unless an alert
        // shows.
        div().absolute().inset_0().children(alert).children(toast)
    }
}

/// Asks before doing something that can't be undone, from any view in a
/// window; `then` runs once confirmed, after the alert has started to
/// leave. The alert opens once the view that asks is done updating.
pub fn confirm(
    confirmation: Confirmation,
    window: &mut Window,
    cx: &mut App,
    then: impl FnOnce(&mut Window, &mut App) + 'static,
) {
    window.defer(cx, move |window, cx| {
        if let Some(alerts) = Alerts::of(window, cx) {
            alerts.update(cx, |alerts, cx| {
                alerts.confirm(confirmation, window, cx, then)
            });
        }
    });
}

/// Says briefly at the bottom of the window that something happened, such
/// as a failure in the background, from any view in a window.
pub fn toast(message: &'static str, window: &mut Window, cx: &mut App) {
    if let Some(alerts) = Alerts::of(window, cx) {
        alerts.update(cx, |alerts, cx| {
            alerts
                .toast
                .show(message, |alerts: &mut Alerts| &mut alerts.toast, cx)
        });
    }
}
