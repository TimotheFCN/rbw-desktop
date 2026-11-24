//! Modal sheets and alerts, and the surface of sheets and popovers.
//!
//! A window that presents sheets keeps a [`SheetSlot`] and renders it as one
//! of its last children; one that asks for confirmations keeps another,
//! [`SheetSlot::alert`], rendered after it. The slot animates the sheet in
//! and out and drops the sheet's view once it has left, so nothing typed into
//! it outlives it.

use gpui::{
    AnyElement, AnyView, App, BoxShadow, Div, InteractiveElement, IntoElement, ParentElement,
    Pixels, RenderOnce, StatefulInteractiveElement, StyleRefinement, Styled, Window, div, point,
    prelude::FluentBuilder as _, px,
};

use crate::motion as m;
use crate::theme::{ActiveTheme, Look};

#[derive(Default)]
pub struct SheetSlot {
    view: Option<AnyView>,
    open: bool,
    alert: bool,
}

impl SheetSlot {
    /// A slot for alerts: small dialogs in the middle of the window, on top
    /// of any sheet.
    pub fn alert() -> Self {
        Self {
            alert: true,
            ..Self::default()
        }
    }

    pub fn present(&mut self, view: impl Into<AnyView>) {
        self.view = Some(view.into());
        self.open = true;
    }

    pub fn dismiss(&mut self) {
        self.open = false;
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// The scrim and the sheet, while either is on screen. `width` is the
    /// sheet's; its height follows its content.
    pub fn render(
        &mut self,
        width: Pixels,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        let id = if self.alert { "alert" } else { "sheet" };
        let presence = m::presence(id, self.open, window, cx);
        if !presence.should_render() {
            self.view = None;
            return None;
        }
        let view = self.view.clone()?;
        let p = presence.progress;
        let theme = cx.theme();
        let look = theme.look;
        let scrim = theme.colors.scrim;
        let radius = match look {
            Look::MacOs => px(18.),
            Look::Gnome => px(14.),
        };
        // A macOS sheet drops from the top of the window; an Adwaita dialog
        // and any alert rise into the middle.
        let from_top = look == Look::MacOs && !self.alert;
        let offset = if from_top { px(-14.) } else { px(12.) } * (1. - p);
        let margin = if from_top { px(34.) } else { px(24.) };
        // A sheet taller than the window scrolls inside.
        let max_height = window.viewport_size().height - margin - px(24.);
        Some(
            div()
                .id(if self.alert {
                    "alert-scrim"
                } else {
                    "sheet-scrim"
                })
                .absolute()
                .inset_0()
                .flex()
                .justify_center()
                .when(from_top, |d| d.items_start().pt(margin))
                .when(!from_top, |d| d.items_center())
                .bg(scrim.opacity(p))
                .occlude()
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    surface()
                        .w(width)
                        .max_h(max_height)
                        .rounded(radius)
                        .p(px(16.))
                        .relative()
                        .top(offset)
                        .opacity(p)
                        .child(view),
                )
                .into_any_element(),
        )
    }
}

/// The surface of sheets, alerts and popovers, without placement: opaque,
/// since GPUI can't blur what is under it, with a hairline edge and the
/// shadows of the mockups. Its children stack in a column.
#[derive(IntoElement)]
pub struct Surface {
    base: Div,
}

pub fn surface() -> Surface {
    Surface {
        base: div().flex().flex_col(),
    }
}

impl Styled for Surface {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for Surface {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for Surface {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let highlight = (theme.look == Look::MacOs).then(|| BoxShadow {
            color: colors.control_highlight,
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
            inset: true,
        });
        let shadows = [
            BoxShadow::new(px(0.), px(24.), colors.shadow).blur_radius(px(50.)),
            BoxShadow::new(px(0.), px(0.), colors.shadow.opacity(0.3)).spread_radius(px(0.5)),
        ];
        self.base
            .bg(colors.frost)
            .border(px(0.5))
            .border_color(colors.sidebar_edge)
            .shadow(shadows.into_iter().chain(highlight).collect())
    }
}
