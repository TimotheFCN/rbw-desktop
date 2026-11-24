//! Titles: a screen's, such as the lock screen's "Welcome back", and a
//! sheet's.

use gpui::{FontWeight, IntoElement, ParentElement, SharedString, Styled, div, px};

/// The title of a screen of its own, under its icon.
pub fn screen_title(title: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(px(20.))
        .font_weight(FontWeight::BOLD)
        .child(title.into())
}

/// The title at the top of a sheet.
pub fn sheet_title(title: impl Into<SharedString>) -> impl IntoElement {
    div()
        .pb(px(12.))
        .text_size(px(15.))
        .font_weight(FontWeight::BOLD)
        .child(title.into())
}
