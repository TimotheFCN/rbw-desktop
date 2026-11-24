//! Push buttons: pill buttons with a label, round icon buttons, and the
//! accent-filled primary button.

use gpui::base::Button as BaseButton;
use gpui::{
    App, BoxShadow, ClickEvent, ElementId, FontWeight, InteractiveElement, IntoElement,
    ParentElement, Pixels, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window,
    div, point, prelude::FluentBuilder as _, px,
};

use crate::icon::{IconName, icon};
use crate::theme::{ActiveTheme, Look};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonStyle {
    /// A raised button on macOS, a filled one on GNOME.
    Normal,
    /// The default action: filled with the accent colour.
    Primary,
    /// No fill until hovered; GNOME's flat header-bar buttons.
    Flat,
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    style: ButtonStyle,
    label: Option<SharedString>,
    icon: Option<IconName>,
    accessibility_label: Option<SharedString>,
    size: Option<Pixels>,
    disabled: bool,
    on_click: Option<ClickHandler>,
}

pub fn button(id: impl Into<ElementId>) -> Button {
    Button {
        id: id.into(),
        style: ButtonStyle::Normal,
        label: None,
        icon: None,
        accessibility_label: None,
        size: None,
        disabled: false,
        on_click: None,
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

    pub fn accessibility_label(mut self, label: impl Into<SharedString>) -> Self {
        self.accessibility_label = Some(label.into());
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

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let metrics = theme.metrics;
        let macos = theme.look == Look::MacOs;
        let height = self.size.unwrap_or(metrics.control_height);
        let round = self.label.is_none();
        let radius = if round && macos {
            height / 2.
        } else {
            metrics.control_radius.min(height / 2.)
        };
        let (fill, text, hover, pressed) = match self.style {
            ButtonStyle::Primary => (
                Some(colors.accent),
                colors.on_accent,
                colors.accent.blend(colors.on_accent.opacity(0.08)),
                colors.accent.blend(gpui::black().opacity(0.12)),
            ),
            ButtonStyle::Normal => (
                Some(colors.control),
                colors.text,
                colors.control.blend(colors.hover),
                colors.control.blend(colors.fill),
            ),
            ButtonStyle::Flat => (None, colors.text, colors.hover, colors.fill),
        };
        let shadows = match (macos, self.style) {
            (false, _) | (true, ButtonStyle::Flat) => vec![],
            (true, ButtonStyle::Primary) => vec![
                inset_highlight(gpui::white().opacity(0.3)),
                BoxShadow::new(px(0.), px(2.), colors.accent.opacity(0.35)).blur_radius(px(8.)),
            ],
            (true, ButtonStyle::Normal) => vec![
                inset_highlight(colors.control_highlight),
                BoxShadow::new(px(0.), px(1.), gpui::black().opacity(0.11)).blur_radius(px(5.)),
                BoxShadow::new(px(0.), px(0.), gpui::black().opacity(0.05)).spread_radius(px(0.5)),
            ],
        };
        let disabled = self.disabled;
        let label = self.accessibility_label.or_else(|| self.label.clone());

        BaseButton::new(self.id)
            .disabled(disabled)
            .when_some(label, |b, label| b.accessibility_label(label))
            .when_some(self.on_click, |b, handler| b.on_click(handler))
            .h(height)
            .when(round, |b| b.w(height))
            .when(!round, |b| {
                b.px(if self.style == ButtonStyle::Primary {
                    px(18.)
                } else {
                    px(12.)
                })
            })
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(px(6.))
            .rounded(radius)
            .text_color(text)
            .font_weight(FontWeight::MEDIUM)
            .when_some(fill, |b, fill| b.bg(fill))
            .when(macos && self.style == ButtonStyle::Normal, |b| {
                b.border(px(0.5)).border_color(colors.control_edge)
            })
            .shadow(shadows)
            .when(!disabled, |b| {
                b.hover(move |s| s.bg(hover)).active(move |s| s.bg(pressed))
            })
            .styles(|s| s.disabled(|s| s.opacity(0.45)))
            .when_some(self.icon, |b, name| b.child(icon(name)))
            .when_some(self.label, |b, label| b.child(div().child(label)))
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
