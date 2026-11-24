//! A window's sidebar: flush with the window's left, top and bottom edges,
//! the full height of the window, a hairline and a soft shade where it meets
//! the content.
//!
//! On a Mac it is the system's sidebar material: an `NSVisualEffectView`
//! placed under GPUI's Metal view at the sidebar's bounds, which blends
//! with what is behind the window, takes its tint from the desktop and turns
//! grey while the window is inactive, as Finder's does. GPUI paints nothing
//! there. Elsewhere, and in the macOS look previewed on Linux, it is a fill.
//!
//! The hosting technique follows gpui-macos (MIT, Copyright 2026 gpui-macos
//! contributors), `src/native/macos.rs`.

use gpui::{
    AnyElement, App, Div, ElementId, IntoElement, ParentElement, RenderOnce, StyleRefinement,
    Styled, Window, div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
};

use crate::chrome;
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
            .rounded_l(corner)
            .border_r(px(if look == Look::Gnome { 1. } else { 0.5 }))
            .border_color(separator);
        let base = match look {
            // GNOME's sidebar is a flat pane.
            Look::Gnome => base.bg(fill),
            // The content casts a faint shade onto the sidebar.
            Look::MacOs => base.child(div().absolute().top_0().bottom_0().right_0().w(px(8.)).bg(
                linear_gradient(
                    90.,
                    linear_color_stop(shade.opacity(0.), 0.),
                    linear_color_stop(shade.opacity(0.1), 1.),
                ),
            )),
        };

        #[cfg(target_os = "macos")]
        if look == Look::MacOs
            && let Some(material) = native::Material::for_element(self.id.clone(), window, cx)
        {
            return base.child(material);
        }
        // The id and window only serve the macOS material.
        let _ = (self.id, window);
        base.when(look == Look::MacOs, |base| base.bg(fill))
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::cell::Cell;

    use gpui::{App, Bounds, ElementId, IntoElement, Pixels, Styled, Window, canvas};
    use objc2::rc::Retained;
    use objc2_app_kit::{
        NSAppearance, NSAppearanceCustomization as _, NSAppearanceNameAqua,
        NSAppearanceNameDarkAqua, NSView, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
        NSVisualEffectState, NSVisualEffectView, NSWindowOrderingMode,
    };
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use crate::theme::{ActiveTheme, Appearance};

    fn renderer_view(window: &Window) -> Option<Retained<NSView>> {
        let handle = HasWindowHandle::window_handle(window).ok()?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return None;
        };
        // SAFETY: the AppKit handle points at the live NSView GPUI renders
        // into for as long as `window` is borrowed; retaining it keeps it
        // valid after that, and this runs on the main thread.
        unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }
    }

    pub(super) struct Material {
        view: Retained<NSVisualEffectView>,
        placed: Cell<Option<NSRect>>,
        appearance: Cell<Option<Appearance>>,
    }

    impl Material {
        /// An element that keeps the sidebar material under its bounds. The
        /// view lives in GPUI's keyed element state, so it goes away when
        /// the element stops rendering.
        pub(super) fn for_element(
            id: ElementId,
            window: &mut Window,
            cx: &mut App,
        ) -> Option<impl IntoElement> {
            let state = window.use_keyed_state((id, "material"), cx, |window, _| Self::new(window));
            state.read(cx).as_ref()?;
            Some(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let appearance = cx.theme().appearance;
                        if let Some(material) = state.read(cx) {
                            material.place(bounds, appearance, window);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            )
        }

        fn new(window: &Window) -> Option<Self> {
            let mtm = MainThreadMarker::new()?;
            let renderer = renderer_view(window)?;
            // SAFETY: reading the superview on the main thread, which is the
            // only thread that changes the view hierarchy.
            let parent = unsafe { renderer.superview() }?;
            let view = NSVisualEffectView::new(mtm);
            view.setMaterial(NSVisualEffectMaterial::Sidebar);
            view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            view.setState(NSVisualEffectState::FollowsWindowActiveState);
            parent.addSubview_positioned_relativeTo(
                &view,
                NSWindowOrderingMode::Below,
                Some(&renderer),
            );
            Some(Self {
                view,
                placed: Cell::new(None),
                appearance: Cell::new(None),
            })
        }

        fn place(&self, bounds: Bounds<Pixels>, appearance: Appearance, window: &Window) {
            // The material follows the app's look, which may be pinned to
            // light or dark while the system is the other.
            if self.appearance.get() != Some(appearance) {
                // SAFETY: AppKit's appearance names are immutable statics.
                let name = unsafe {
                    match appearance {
                        Appearance::Light => NSAppearanceNameAqua,
                        Appearance::Dark => NSAppearanceNameDarkAqua,
                    }
                };
                self.view
                    .setAppearance(NSAppearance::appearanceNamed(name).as_deref());
                self.appearance.set(Some(appearance));
            }
            let Some(renderer) = renderer_view(window) else {
                return;
            };
            // SAFETY: as in `new`.
            let Some(parent) = (unsafe { renderer.superview() }) else {
                return;
            };
            let (w, h) = (
                f64::from(f32::from(bounds.size.width)),
                f64::from(f32::from(bounds.size.height)),
            );
            let top = f64::from(f32::from(bounds.origin.y));
            let y = if renderer.isFlipped() {
                top
            } else {
                renderer.bounds().size.height - top - h
            };
            let frame = parent.convertRect_fromView(
                NSRect::new(
                    NSPoint::new(f64::from(f32::from(bounds.origin.x)), y),
                    NSSize::new(w, h),
                ),
                Some(&renderer),
            );
            // Setting the frame on every frame would make AppKit re-layout
            // and recomposite the material while unrelated content animates.
            if self.placed.get() != Some(frame) {
                self.view.setFrame(frame);
                self.placed.set(Some(frame));
            }
        }
    }

    impl Drop for Material {
        fn drop(&mut self) {
            self.view.removeFromSuperview();
        }
    }
}
