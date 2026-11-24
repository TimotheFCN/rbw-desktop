//! Push buttons: pill buttons with a label, round icon buttons, the
//! accent-filled primary button, the red destructive one and link-like text
//! buttons.

use gpui::base::Button as BaseButton;
use gpui::{
    App, BoxShadow, ClickEvent, ElementId, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Pixels, RenderOnce, SharedString, StatefulInteractiveElement,
    Styled, Window, div, point, prelude::FluentBuilder as _, px,
};

use crate::controls::spinner;
use crate::focus::control_ring;
use crate::icon::{IconName, icon};
use crate::theme::{ActiveTheme, Look};
use crate::tooltip::WithTooltip as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    /// A raised button on macOS, a filled one on GNOME.
    Normal,
    /// The default action: filled with the accent colour.
    Primary,
    /// No fill until hovered, in the colour of the text around it; GNOME's
    /// flat header-bar buttons.
    Flat,
    /// An action that removes something, filled red.
    Destructive,
    /// Accent-coloured text without a shape, for a secondary way to do
    /// something, like unlocking with Touch ID.
    Link,
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    style: ButtonStyle,
    label: Option<SharedString>,
    icon: Option<IconName>,
    trailing_icon: Option<IconName>,
    accessibility_label: Option<SharedString>,
    shortcut: Option<&'static str>,
    size: Option<Pixels>,
    disabled: bool,
    busy: bool,
    wide: bool,
    focus: Option<FocusHandle>,
    on_click: Option<ClickHandler>,
    on_hover: Option<HoverHandler>,
}

type HoverHandler = Box<dyn Fn(&bool, &mut Window, &mut App)>;

pub fn button(id: impl Into<ElementId>) -> Button {
    Button {
        id: id.into(),
        style: ButtonStyle::Normal,
        label: None,
        icon: None,
        trailing_icon: None,
        accessibility_label: None,
        shortcut: None,
        size: None,
        disabled: false,
        busy: false,
        wide: false,
        focus: None,
        on_click: None,
        on_hover: None,
    }
}

impl Button {
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// An icon. A button with an icon and no label is round, and needs an
    /// [`accessibility_label`](Self::accessibility_label).
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// An icon after the label, such as the chevrons of a button that opens
    /// a menu.
    pub fn trailing_icon(mut self, icon: IconName) -> Self {
        self.trailing_icon = Some(icon);
        self
    }

    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
        self
    }

    /// The key binding that does the same, such as `cmd-o`, which the
    /// tooltip shows beside the label.
    pub fn shortcut(mut self, keys: &'static str) -> Self {
        self.shortcut = Some(keys);
        self
    }

    pub fn style(mut self, style: ButtonStyle) -> Self {
        self.style = style;
        self
    }

    /// Overrides the look's control height, for buttons inside rows.
    pub fn size(mut self, size: Pixels) -> Self {
        self.size = Some(size);
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Shows a spinner in place of the content while the action it started
    /// is under way, and ignores clicks. The button keeps its size.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    /// Fills the width it is given, like the buttons of an alert.
    pub fn wide(mut self) -> Self {
        self.wide = true;
        self
    }

    /// Takes the keyboard with `focus`, for a button that should have it
    /// first, such as a panel's main action.
    pub fn track_focus(mut self, focus: &FocusHandle) -> Self {
        self.focus = Some(focus.clone());
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// Called with `true` when the pointer comes onto the button, `false`
    /// when it leaves.
    pub fn on_hover(mut self, handler: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_hover = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let metrics = theme.metrics;
        let macos = theme.look == Look::MacOs;
        let link = self.style == ButtonStyle::Link;
        let height = self.size.unwrap_or(metrics.control_height);
        let round = self.label.is_none();
        let radius = if round && macos {
            height / 2.
        } else {
            metrics.control_radius.min(height / 2.)
        };
        let filled = |fill: gpui::Hsla, text: gpui::Hsla| {
            (
                Some(fill),
                Some(text),
                fill.blend(text.opacity(0.08)),
                fill.blend(gpui::black().opacity(0.12)),
            )
        };
        let (fill, text, hover, pressed) = match self.style {
            ButtonStyle::Primary => filled(colors.accent, colors.on_accent),
            ButtonStyle::Destructive => filled(colors.destructive, colors.on_accent),
            ButtonStyle::Normal => (
                Some(colors.control),
                Some(colors.text),
                colors.control.blend(colors.hover),
                colors.control.blend(colors.fill),
            ),
            ButtonStyle::Flat => (None, None, colors.hover, colors.fill),
            ButtonStyle::Link => (
                None,
                Some(colors.accent_text),
                gpui::transparent_black(),
                gpui::transparent_black(),
            ),
        };
        let shadows = match (macos, self.style) {
            (false, _) | (true, ButtonStyle::Flat | ButtonStyle::Link) => vec![],
            (true, ButtonStyle::Primary | ButtonStyle::Destructive) => vec![
                inset_highlight(gpui::white().opacity(0.3)),
                BoxShadow::new(px(0.), px(2.), fill.unwrap_or(colors.accent).opacity(0.35))
                    .blur_radius(px(8.)),
            ],
            (true, ButtonStyle::Normal) => vec![
                inset_highlight(colors.control_highlight),
                BoxShadow::new(px(0.), px(1.), gpui::black().opacity(0.11)).blur_radius(px(5.)),
                BoxShadow::new(px(0.), px(0.), gpui::black().opacity(0.05)).spread_radius(px(0.5)),
            ],
        };
        let ring = control_ring(cx);
        let spinner_color = text.unwrap_or(colors.text_secondary);
        let (busy, disabled) = (self.busy, self.disabled);
        let label = self.accessibility_label.or_else(|| self.label.clone());
        // A button with only an icon says what it does when the pointer
        // rests on it, as does one with a shortcut.
        let tip = label
            .clone()
            .filter(|_| (round && !link) || self.shortcut.is_some());
        let shortcut = self.shortcut;
        let padding = match self.style {
            ButtonStyle::Link => px(2.),
            ButtonStyle::Primary | ButtonStyle::Destructive => px(18.),
            ButtonStyle::Normal | ButtonStyle::Flat => px(12.),
        };
        // The content stays in place, hidden, under the spinner, so the
        // button keeps its width.
        let content = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .when(busy, |d| d.invisible())
            .when_some(self.icon, |d, name| d.child(icon(name)))
            .when_some(self.label, |d, label| d.child(label))
            .when_some(self.trailing_icon, |d, name| d.child(icon(name)));
        let id = self.id.clone();

        BaseButton::new(self.id)
            .when_some(self.focus, |b, focus| b.track_focus(&focus))
            .disabled(disabled || busy)
            .when_some(label, |b, label| b.accessibility_label(label))
            .when_some(tip, |b, tip| b.with_tooltip(tip, shortcut, cx))
            .when_some(self.on_click, |b, handler| b.on_click(handler))
            .when_some(self.on_hover, |b, handler| b.on_hover(handler))
            .relative()
            .h(height)
            .when(round && !link, |b| b.w(height))
            .when(!round, |b| b.px(padding))
            .when(self.wide, |b| b.w_full())
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .rounded(radius)
            .when_some(text, |b, text| b.text_color(text))
            .font_weight(FontWeight::MEDIUM)
            .when_some(fill, |b, fill| b.bg(fill))
            .when(macos && self.style == ButtonStyle::Normal, |b| {
                b.border(px(0.5)).border_color(colors.control_edge)
            })
            .shadow(shadows)
            .focus_visible(ring)
            .when(!disabled && !busy, |b| {
                b.hover(move |s| s.bg(hover)).active(move |s| s.bg(pressed))
            })
            .when(link && !disabled && !busy, |b| {
                b.hover(|s| s.opacity(0.75)).active(|s| s.opacity(0.6))
            })
            .styles(|s| s.disabled(|s| s.opacity(if busy { 1. } else { 0.45 })))
            .child(content)
            .when(busy, |b| {
                b.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(spinner((id, "busy"), px(14.), spinner_color, window, cx)),
                )
            })
    }
}

fn inset_highlight(color: gpui::Hsla) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), px(1.)),
        blur_radius: px(0.),
        spread_radius: px(0.),
        inset: true,
    }
}
