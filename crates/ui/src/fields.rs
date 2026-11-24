//! Text entry: the search field and the free-standing text field. Fields in a
//! form sit in card rows instead; see [`crate::card::form_row`].

use gpui::base::input::{Input, InputState};
use gpui::{
    AnyElement, App, BoxShadow, ElementId, Entity, Focusable, Hsla, InteractiveElement,
    IntoElement, ParentElement, RenderOnce, StatefulInteractiveElement, Styled, Window, div, point,
    prelude::FluentBuilder as _, px,
};

use crate::icon::{IconName, icon};
use crate::theme::{ActiveTheme, Look};

/// The vault's search field: a capsule on macOS, a filled entry on GNOME.
#[derive(IntoElement)]
pub struct SearchField {
    state: Entity<InputState>,
}

pub fn search_field(state: &Entity<InputState>) -> SearchField {
    SearchField {
        state: state.clone(),
    }
}

impl RenderOnce for SearchField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focused = self.state.read(cx).focus_handle(cx).is_focused(window);
        let theme = cx.theme();
        let colors = &theme.colors;
        let metrics = theme.metrics;
        let macos = theme.look == Look::MacOs;
        let focus = self.state.read(cx).focus_handle(cx);
        div()
            .id("search-field")
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap(px(6.))
            .h(metrics.control_height)
            .px(px(10.))
            .rounded(metrics.control_radius)
            .bg(colors.control)
            .text_color(colors.text_secondary)
            .cursor_text()
            .on_click(move |_, window, cx| window.focus(&focus, cx))
            .when(macos, |d| {
                d.border(px(0.5))
                    .border_color(colors.control_edge)
                    .shadow(vec![
                        inset(colors.control_highlight),
                        BoxShadow::new(px(0.), px(1.), gpui::black().opacity(0.11))
                            .blur_radius(px(5.)),
                        BoxShadow::new(px(0.), px(0.), gpui::black().opacity(0.05))
                            .spread_radius(px(0.5)),
                    ])
            })
            .when(focused, |d| focus_ring(d, colors.focus_ring))
            .child(icon(IconName::Search))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(colors.text)
                    .child(Input::new(&self.state)),
            )
    }
}

/// A free-standing text field, such as the master password on the lock
/// screen. `trailing` holds an optional button inside its end.
#[derive(IntoElement)]
pub struct TextField {
    id: ElementId,
    state: Entity<InputState>,
    trailing: Option<AnyElement>,
}

pub fn text_field(id: impl Into<ElementId>, state: &Entity<InputState>) -> TextField {
    TextField {
        id: id.into(),
        state: state.clone(),
        trailing: None,
    }
}

impl TextField {
    pub fn trailing(mut self, element: impl IntoElement) -> Self {
        self.trailing = Some(element.into_any_element());
        self
    }
}

impl RenderOnce for TextField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = self.state.read(cx).focus_handle(cx);
        let focused = focus.is_focused(window);
        let theme = cx.theme();
        let colors = &theme.colors;
        let macos = theme.look == Look::MacOs;
        let height = theme.metrics.control_height + px(8.);
        div()
            .id(self.id)
            .flex()
            .items_center()
            .h(height)
            .pl(px(14.))
            .pr(px(5.))
            .rounded(if macos {
                height / 2.
            } else {
                theme.metrics.control_radius
            })
            .bg(colors.field)
            .text_color(colors.text)
            .cursor_text()
            .on_click(move |_, window, cx| window.focus(&focus, cx))
            .when(macos, |d| {
                d.border(px(0.5))
                    .border_color(colors.control_edge)
                    .shadow(vec![
                        BoxShadow::new(px(0.), px(1.), gpui::black().opacity(0.11))
                            .blur_radius(px(5.)),
                        BoxShadow::new(px(0.), px(0.), gpui::black().opacity(0.06))
                            .spread_radius(px(0.5)),
                    ])
            })
            .when(focused, |d| focus_ring(d, colors.focus_ring))
            .child(div().flex_1().min_w_0().child(Input::new(&self.state)))
            .children(self.trailing)
    }
}

fn inset(color: Hsla) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), px(1.)),
        blur_radius: px(0.),
        spread_radius: px(0.),
        inset: true,
    }
}

/// The keyboard focus ring both looks draw around a focused field: the
/// accent at half strength, three points wide, outside the edge.
fn focus_ring<E: Styled>(element: E, color: Hsla) -> E {
    element.border_color(Hsla { a: 1., ..color }).shadow(vec![
        BoxShadow::new(px(0.), px(0.), color).spread_radius(px(3.)),
    ])
}
