//! The name of an icon button, shown when the pointer rests on it, as
//! macOS and GNOME do for toolbar buttons.

use gpui::{
    AnyView, App, AppContext as _, BoxShadow, Context, IntoElement, ParentElement, Render,
    SharedString, Styled, Window, div, px,
};

use crate::theme::ActiveTheme;

struct Tooltip(SharedString);

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = &cx.theme().colors;
        div()
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .bg(colors.frost)
            .border(px(0.5))
            .border_color(colors.separator)
            .shadow(vec![
                BoxShadow::new(px(0.), px(2.), colors.shadow.opacity(0.4)).blur_radius(px(6.)),
            ])
            .text_size(px(12.))
            .text_color(colors.text)
            .child(self.0.clone())
    }
}

/// Builds the tooltip saying `label`, for `tooltip()`.
pub(crate) fn tooltip(label: SharedString) -> impl Fn(&mut Window, &mut App) -> AnyView {
    move |_, cx| cx.new(|_| Tooltip(label.clone())).into()
}
