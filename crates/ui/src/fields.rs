//! Text entry: the search field and the free-standing text field. Fields in a
//! form sit in card rows instead; see [`crate::card::form_row`].

use gpui::base::input::{Input, InputBase, InputState};
use gpui::{
    AnyElement, App, BoxShadow, ElementId, Entity, Focusable, HighlightStyle, Hsla,
    InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, StyledText, Window, div, point,
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
            .map(|d| focus_ring(d, focused, macos, colors.focus_ring))
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
/// screen. `label` names it for assistive technologies; on screen its
/// placeholder does. `trailing` holds an optional button inside its end.
#[derive(IntoElement)]
pub struct TextField {
    id: ElementId,
    state: Entity<InputState>,
    label: SharedString,
    trailing: Option<AnyElement>,
}

pub fn text_field(
    id: impl Into<ElementId>,
    state: &Entity<InputState>,
    label: impl Into<SharedString>,
) -> TextField {
    TextField {
        id: id.into(),
        state: state.clone(),
        label: label.into(),
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
        InputBase::new(self.id)
            .accessibility_label(self.label)
            .focused(focused)
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
            .map(|d| focus_ring(d, focused, macos, colors.focus_ring))
            .child(div().flex_1().min_w_0().child(Input::new(&self.state)))
            .children(self.trailing)
    }
}

/// A sentence under a field saying what is wrong with what was entered, or
/// why the step it belongs to failed.
pub fn field_error(message: impl Into<SharedString>, cx: &App) -> gpui::Div {
    div()
        .text_size(cx.theme().metrics.small_text_size)
        .text_color(cx.theme().colors.danger)
        .child(message.into())
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

/// The keyboard focus ring around a focused field. macOS draws the accent
/// at half strength, three points wide, outside the edge. Adwaita draws it
/// two points wide inside the edge: GPUI paints a shadow under the whole
/// field, where GNOME's translucent fill would let it show, so GNOME's ring
/// is a border that is always there, clear until focused, and moves nothing
/// when it appears.
fn focus_ring<E: Styled>(element: E, focused: bool, macos: bool, color: Hsla) -> E {
    match (macos, focused) {
        (true, true) => element.border_color(Hsla { a: 1., ..color }).shadow(vec![
            BoxShadow::new(px(0.), px(0.), color).spread_radius(px(3.)),
        ]),
        (true, false) => element,
        (false, focused) => element.border_2().border_color(if focused {
            color
        } else {
            gpui::transparent_black()
        }),
    }
}

/// A password as text, its digits and symbols coloured apart from its
/// letters so that each character reads unambiguously.
pub fn password_text(value: &str, cx: &App) -> StyledText {
    let colors = &cx.theme().colors;
    let highlights = value
        .char_indices()
        .filter_map(|(start, c)| {
            let color = if c.is_ascii_digit() {
                colors.accent_text
            } else if c.is_ascii_punctuation() || (!c.is_alphanumeric() && !c.is_whitespace()) {
                colors.symbol
            } else {
                return None;
            };
            Some((start..start + c.len_utf8(), HighlightStyle::color(color)))
        })
        .collect::<Vec<_>>();
    StyledText::new(value.to_string()).with_highlights(highlights)
}
