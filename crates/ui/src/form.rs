//! Settings pages: groups of rows, each a title with an optional line
//! under it and the control that sets it, as in macOS's grouped forms and
//! GNOME's preferences rows.

use gpui::{
    AnyElement, App, FontWeight, IntoElement, ParentElement, SharedString, Styled, div,
    prelude::FluentBuilder as _, px, relative,
};

use crate::card::card;
use crate::theme::{ActiveTheme, Look};

/// A group of rows: an optional heading over a card holding them.
pub fn settings_group(
    title: Option<SharedString>,
    rows: impl IntoIterator<Item = AnyElement>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let heading = title.map(|title| {
        div()
            .px(px(4.))
            .pb(px(7.))
            .text_size(px(13.))
            .font_weight(match theme.look {
                Look::MacOs => FontWeight::SEMIBOLD,
                Look::Gnome => FontWeight::BOLD,
            })
            .child(title)
    });
    div()
        .flex()
        .flex_col()
        .children(heading)
        .child(card().rows(rows))
        .into_any_element()
}

/// A row of a settings group: what it sets, with an optional sentence
/// under it, and the control at its end.
pub fn setting_row(
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    control: impl IntoElement,
    cx: &App,
) -> AnyElement {
    setting_row_with(title, subtitle.map(|s| div().child(s)), control, cx)
}

/// [`setting_row`] with a line under the title made of more than text,
/// such as a fingerprint in a quieter colour.
pub fn setting_row_with(
    title: impl Into<SharedString>,
    subtitle: Option<impl IntoElement>,
    control: impl IntoElement,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let gnome = theme.look == Look::Gnome;
    let has_subtitle = subtitle.is_some();
    div()
        .flex()
        .items_center()
        .gap(px(12.))
        .min_h(if gnome { px(52.) } else { px(40.) })
        .px(if gnome { px(14.) } else { px(13.) })
        .py(px(if has_subtitle { 8. } else { 6. }))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .line_height(relative(1.35))
                .child(title.into())
                .when_some(subtitle, |d, subtitle| {
                    d.child(
                        div()
                            .text_size(theme.metrics.small_text_size)
                            .text_color(theme.colors.text_secondary)
                            .child(subtitle),
                    )
                }),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(control),
        )
        .into_any_element()
}
