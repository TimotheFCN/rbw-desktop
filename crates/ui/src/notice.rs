//! A notice: something the person should know about without being asked,
//! that stays until they act on it or dismiss it. A window floats it at its
//! bottom with [`Notice::floating`]; it never takes the keyboard.

use gpui::{
    AnyElement, App, ClickEvent, ElementId, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div, prelude::FluentBuilder as _, px, relative,
};

use crate::button::{Button, ButtonStyle, button};
use crate::icon::{IconName, icon};
use crate::motion as m;
use crate::sheet::surface;
use crate::theme::ActiveTheme;

type DismissHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

#[derive(IntoElement)]
pub struct Notice {
    id: ElementId,
    title: SharedString,
    message: SharedString,
    action: Option<Button>,
    on_dismiss: Option<DismissHandler>,
}

/// A notice with a warning sign: a title, a sentence or two under it.
pub fn notice(
    id: impl Into<ElementId>,
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
) -> Notice {
    Notice {
        id: id.into(),
        title: title.into(),
        message: message.into(),
        action: None,
        on_dismiss: None,
    }
}

impl Notice {
    /// The one thing to do about it, such as opening Settings.
    pub fn action(mut self, action: Button) -> Self {
        self.action = Some(action);
        self
    }

    /// Shows a close button, which calls `handler`.
    pub fn on_dismiss(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_dismiss = Some(Box::new(handler));
        self
    }

    /// The notice at the bottom of the window, over what is there: it rises
    /// in while `open` and sinks away once it isn't. `None` once it has
    /// gone.
    pub fn floating(self, open: bool, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
        let presence = m::presence((self.id.clone(), "presence"), open, window, cx);
        if !presence.should_render() {
            return None;
        }
        let p = presence.progress;
        Some(
            div()
                .absolute()
                .bottom(px(10.) + px(10.) * p)
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .px(px(16.))
                .child(div().opacity(p).child(self))
                .into_any_element(),
        )
    }
}

impl RenderOnce for Notice {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let close = self.on_dismiss.map(|handler| {
            button((self.id.clone(), "dismiss"))
                .icon(IconName::Close)
                .style(ButtonStyle::Flat)
                .size(px(22.))
                .accessibility_label("Dismiss")
                .on_click(handler)
        });
        surface()
            .w(px(400.))
            .max_w_full()
            .rounded(theme.metrics.card_radius + px(4.))
            .flex_row()
            .gap(px(10.))
            .pl(px(14.))
            .pr(px(8.))
            .py(px(12.))
            .child(
                div()
                    .flex_none()
                    .pt(px(1.))
                    .text_color(colors.warning)
                    .child(icon(IconName::Warning)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .line_height(relative(1.35))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(self.title),
                    )
                    .child(
                        div()
                            .text_size(theme.metrics.small_text_size)
                            .text_color(colors.text_secondary)
                            .child(self.message),
                    )
                    .when_some(self.action, |d, action| {
                        d.child(div().mt(px(8.)).flex().child(action))
                    }),
            )
            .children(close.map(|close| div().flex_none().child(close)))
    }
}
