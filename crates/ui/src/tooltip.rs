//! What a control does, shown when the pointer rests on it, with its
//! keyboard shortcut when it has one, as macOS and GNOME do for toolbar
//! buttons: after the platform's delay, in the platform's style, centred
//! under the control, or over it where the window has no room under it.
//! Icon buttons get one from their label; [`WithTooltip`] gives one to any
//! other element.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    Anchor, AnchoredPositionMode, Animation, AnimationExt as _, App, AppContext as _, Bounds,
    BoxShadow, Context, IntoElement, ParentElement, Pixels, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, anchored, canvas, div, point,
    prelude::FluentBuilder as _, px,
};

use crate::motion as m;
use crate::theme::{ActiveTheme, Look};

/// The room between a control and its tooltip.
const GAP: Pixels = px(6.);

/// How long the pointer rests before a tooltip shows: AppKit's wait, and
/// GTK's.
fn delay(look: Look) -> Duration {
    match look {
        Look::MacOs => Duration::from_millis(1000),
        Look::Gnome => Duration::from_millis(500),
    }
}

struct Tooltip {
    label: SharedString,
    keys: Option<SharedString>,
    /// The control it is about, in the window.
    control: Bounds<Pixels>,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let tip = div().flex().items_center().gap(px(8.)).text_size(px(12.));
        let tip = match theme.look {
            // A small light card with a hairline edge and a soft shadow,
            // its shortcut quieter beside the label.
            Look::MacOs => tip
                .px(px(7.))
                .py(px(3.))
                .rounded(px(6.))
                .bg(colors.frost)
                .border(px(0.5))
                .border_color(colors.separator)
                .shadow(vec![
                    BoxShadow::new(px(0.), px(2.), colors.shadow.opacity(0.4)).blur_radius(px(6.)),
                ])
                .text_color(colors.text)
                .child(self.label.clone())
                .when_some(self.keys.clone(), |tip, keys| {
                    tip.child(div().text_color(colors.text_secondary).child(keys))
                }),
            // Adwaita's: white on a dark, rounded box in either style.
            Look::Gnome => tip
                .px(px(10.))
                .py(px(6.))
                .rounded(px(8.))
                .bg(gpui::black().opacity(0.8))
                .text_color(gpui::white())
                .child(self.label.clone())
                .when_some(self.keys.clone(), |tip, keys| {
                    tip.child(div().text_color(gpui::white().opacity(0.7)).child(keys))
                }),
        };
        // GPUI shows a tooltip at the pointer; this one moves to its
        // control, and fades in there.
        anchored()
            .position_mode(AnchoredPositionMode::Window)
            .attach(self.control, Anchor::BottomCenter)
            .anchor(Anchor::TopCenter)
            .offset(point(px(0.), GAP))
            .snap_to_window_with_margin(px(8.))
            .child(tip.with_animation("tooltip", Animation::new(m::QUICK), |tip, t| tip.opacity(t)))
    }
}

/// A tooltip on any element that can have one.
pub trait WithTooltip: StatefulInteractiveElement + ParentElement + Sized {
    /// Shows `label`, and `keys`, a key binding such as `cmd-o`, when the
    /// pointer rests on the element.
    fn with_tooltip(
        self,
        label: impl Into<SharedString>,
        keys: Option<&'static str>,
        cx: &App,
    ) -> Self {
        let label = label.into();
        // Where the element is, for the tooltip to go under it.
        let control = Rc::new(Cell::new(Bounds::default()));
        let drawn = control.clone();
        self.child(
            canvas(move |bounds, _, _| drawn.set(bounds), |_, _, _, _| {})
                .absolute()
                .size_full(),
        )
        .tooltip(move |_, cx| {
            let keys = keys.map(|keys| shortcut(keys, cx.theme().look).into());
            let (label, control) = (label.clone(), control.get());
            cx.new(|_| Tooltip {
                label,
                keys,
                control,
            })
            .into()
        })
        .tooltip_show_delay(delay(cx.theme().look))
    }
}

impl<E: StatefulInteractiveElement + ParentElement> WithTooltip for E {}

/// What a click or a key does, named with the key, as a line under a list
/// says it for the row the pointer or the keyboard is on.
pub fn key_hint(label: impl Into<SharedString>, keys: &'static str, cx: &App) -> gpui::Div {
    let theme = cx.theme();
    div()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .text_size(px(11.5))
        .text_color(theme.colors.text_secondary)
        .child(label.into())
        .child(
            div()
                .text_color(theme.colors.text_tertiary)
                .child(shortcut(keys, theme.look)),
        )
}

/// A key binding as the platform writes it: `⇧⌘C` on a Mac, `Shift+Ctrl+C`
/// on GNOME, as GTK does.
pub(crate) fn shortcut(keys: &str, look: Look) -> String {
    let mut parts: Vec<&str> = keys.split('-').collect();
    let key = parts.pop().unwrap_or_default();
    let has = |modifier: &str| parts.contains(&modifier);
    let key = match (key, look) {
        ("enter", Look::MacOs) => "↩".to_string(),
        ("enter", Look::Gnome) => "Enter".to_string(),
        ("backspace", Look::MacOs) => "⌫".to_string(),
        ("backspace", Look::Gnome) => "Backspace".to_string(),
        ("escape", Look::MacOs) => "⎋".to_string(),
        ("escape", Look::Gnome) => "Esc".to_string(),
        ("comma", _) => ",".to_string(),
        (key, _) => key.to_uppercase(),
    };
    match look {
        Look::MacOs => {
            let symbols = [("ctrl", "⌃"), ("alt", "⌥"), ("shift", "⇧"), ("cmd", "⌘")];
            let mut text: String = symbols
                .iter()
                .filter(|(modifier, _)| has(modifier))
                .map(|(_, symbol)| *symbol)
                .collect();
            text.push_str(&key);
            text
        }
        Look::Gnome => {
            let names = [
                ("shift", "Shift"),
                ("ctrl", "Ctrl"),
                ("alt", "Alt"),
                ("cmd", "Super"),
            ];
            let mut words: Vec<String> = names
                .iter()
                .filter(|(modifier, _)| has(modifier))
                .map(|(_, name)| name.to_string())
                .collect();
            words.push(key);
            words.join("+")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_read_as_each_platform_writes_them() {
        assert_eq!(shortcut("cmd-shift-c", Look::MacOs), "⇧⌘C");
        assert_eq!(shortcut("ctrl-alt-c", Look::MacOs), "⌃⌥C");
        assert_eq!(shortcut("enter", Look::MacOs), "↩");
        assert_eq!(shortcut("ctrl-shift-c", Look::Gnome), "Shift+Ctrl+C");
        assert_eq!(shortcut("ctrl-o", Look::Gnome), "Ctrl+O");
        assert_eq!(shortcut("enter", Look::Gnome), "Enter");
    }
}
