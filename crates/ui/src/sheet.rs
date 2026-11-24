//! Modal sheets and alerts, and the surface of sheets and popovers.
//!
//! A window that presents sheets keeps a [`SheetSlot`] and renders it as one
//! of its last children; one that asks for confirmations keeps another,
//! [`SheetSlot::alert`], rendered after it. The slot animates the sheet in
//! and out and drops the sheet's view once it has left, so nothing typed into
//! it outlives it.

use gpui::{
    AnyElement, AnyView, App, ElementId, InteractiveElement, IntoElement, ParentElement, Pixels,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder as _, px,
};

use crate::glass::{Glass, Material, glass};
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
                .when(from_top, |d| d.items_start().pt(px(34.)))
                .when(!from_top, |d| d.items_center())
                .bg(scrim.opacity(p))
                .occlude()
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    surface(id, Material::Sheet)
                        .w(width)
                        .rounded(radius)
                        .p(px(16.))
                        .mt(offset)
                        .opacity(p)
                        .child(view),
                )
                .into_any_element(),
        )
    }
}

/// The translucent surface of sheets and popovers, without placement.
pub fn surface(id: impl Into<ElementId>, material: Material) -> Glass {
    glass(id, material).flex().flex_col()
}
