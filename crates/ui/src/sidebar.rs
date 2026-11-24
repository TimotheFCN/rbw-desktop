//! A window's sidebar: flush with the window's left, top and bottom edges,
//! the full height of the window, a hairline and a soft shade where it meets
//! the content.
//!
//! On a Mac it is the system's sidebar material ([`Material::Sidebar`]),
//! which takes its tint from the desktop and turns grey while the window
//! is inactive, as Finder's does. Elsewhere, and in the macOS look
//! previewed on Linux, it is a fill.

use gpui::{
    AnyElement, App, Div, ElementId, IntoElement, ParentElement, RenderOnce, StyleRefinement,
    Styled, Window, div, linear_color_stop, linear_gradient, px,
};

use crate::chrome;
use crate::material::{Material, backdrop};
use crate::theme::{ActiveTheme, Look};

#[derive(IntoElement)]
pub struct Sidebar {
    id: ElementId,
    base: Div,
}

/// A sidebar of the look's width. Its children stack in a column.
pub fn sidebar(id: impl Into<ElementId>) -> Sidebar {
    Sidebar {
        id: id.into(),
        base: div(),
    }
}

impl Styled for Sidebar {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for Sidebar {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for Sidebar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let corner = chrome::corner_radius(window, cx);
        let theme = cx.theme();
        let (look, colors) = (theme.look, &theme.colors);
        let (fill, separator, shade) = (colors.sidebar, colors.separator, colors.shadow);
        let base = self
            .base
            .relative()
            .flex_none()
            .w(theme.metrics.sidebar_width)
            .h_full()
            .flex()
            .flex_col()
            .rounded_l(corner);
        if look == Look::Gnome {
            // A flat pane.
            return base
                .bg(fill)
                .border_r_1()
                .border_color(separator)
                .into_any_element();
        }
        // The content casts a faint shade onto the sidebar, over a hairline.
        // Both lie inside the sidebar's bounds, which the material fills:
        // a border would leave the desktop showing beside it.
        let edge = div()
            .absolute()
            .top_0()
            .bottom_0()
            .right_0()
            .w(px(8.))
            .flex()
            .justify_end()
            .bg(linear_gradient(
                90.,
                linear_color_stop(shade.opacity(0.), 0.),
                linear_color_stop(shade.opacity(0.1), 1.),
            ))
            .child(div().w(px(0.5)).h_full().bg(separator));
        match backdrop(self.id, Material::Sidebar, window, cx) {
            Some(material) => base.child(material).child(edge),
            None => base.bg(fill).child(edge),
        }
        .into_any_element()
    }
}
