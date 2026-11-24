//! The ring that shows which control has the keyboard, as each look draws
//! it. It shows only when the keyboard moved the focus there.

use gpui::{App, BoxShadow, Hsla, StyleRefinement, Styled as _, point, px};

use crate::theme::{ActiveTheme, Look};

/// Around a control: on macOS outside its edge, three points of the accent
/// at half strength; on GNOME inside it, two points, as Adwaita's outline.
pub(crate) fn control_ring(cx: &App) -> impl Fn(StyleRefinement) -> StyleRefinement + 'static {
    let theme = cx.theme();
    let color = theme.colors.focus_ring;
    let gnome = theme.look == Look::Gnome;
    move |style| style.shadow(vec![ring(color, gnome, if gnome { 2. } else { 3. })])
}

/// The ring of a row of the theme's colour, for a row the app builds on
/// gpui-base's button, such as the sidebar's account.
pub fn row_focus_ring(cx: &App) -> impl Fn(StyleRefinement) -> StyleRefinement + 'static {
    row_ring(cx.theme().colors.focus_ring)
}

/// Inside a row, whose list or card would clip a ring outside it.
pub(crate) fn row_ring(color: Hsla) -> impl Fn(StyleRefinement) -> StyleRefinement + 'static {
    move |style| style.shadow(inner_ring(color))
}

/// The ring inside a row, for a row that shows its list's focus.
pub(crate) fn inner_ring(color: Hsla) -> Vec<BoxShadow> {
    vec![ring(color, true, 2.)]
}

fn ring(color: Hsla, inside: bool, width: f32) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(width),
        inset: inside,
    }
}
