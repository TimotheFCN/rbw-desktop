//! The system's materials under what GPUI draws, on a Mac: an
//! `NSVisualEffectView` placed under GPUI's Metal view at an element's
//! bounds, blending with what is behind the window and taking its tint, in
//! the app's light or dark appearance. GPUI paints nothing there. With
//! Reduce Transparency, or where the system draws no blur, the view is the
//! material's solid colour.
//!
//! The hosting technique follows gpui-macos (MIT, Copyright 2026 gpui-macos
//! contributors), `src/native/macos.rs`.

use gpui::{AnyElement, App, ElementId, Window};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    /// A sidebar, and the window behind a screen of its own such as the
    /// lock screen: grey while the window is inactive.
    Sidebar,
    /// A panel that drops from the menu bar.
    Popover,
}

/// An element, absolutely placed over all of its parent, that keeps
/// `material` under the parent on a Mac. `None` elsewhere, and where the
/// view can't be made: the caller then paints a fill.
pub fn backdrop(
    id: impl Into<ElementId>,
    material: Material,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    #[cfg(target_os = "macos")]
    {
        use gpui::IntoElement as _;
        native::View::for_element(id.into(), material, window, cx)
            .map(|view| view.into_any_element())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (id, material, window, cx);
        None
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

    use super::Material;
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

    pub(super) struct View {
        view: Retained<NSVisualEffectView>,
        placed: Cell<Option<NSRect>>,
        appearance: Cell<Option<Appearance>>,
    }

    impl View {
        /// An element that keeps `material` under its bounds. The view lives
        /// in GPUI's keyed element state, so it goes away when the element
        /// stops rendering.
        pub(super) fn for_element(
            id: ElementId,
            material: Material,
            window: &mut Window,
            cx: &mut App,
        ) -> Option<impl IntoElement> {
            let state = window.use_keyed_state((id, "material"), cx, move |window, _| {
                Self::new(material, window)
            });
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

        fn new(material: Material, window: &Window) -> Option<Self> {
            let mtm = MainThreadMarker::new()?;
            let renderer = renderer_view(window)?;
            // SAFETY: reading the superview on the main thread, which is the
            // only thread that changes the view hierarchy.
            let parent = unsafe { renderer.superview() }?;
            let view = NSVisualEffectView::new(mtm);
            let (material, state) = match material {
                // Grey while the window is inactive, as Finder's sidebar.
                Material::Sidebar => (
                    NSVisualEffectMaterial::Sidebar,
                    NSVisualEffectState::FollowsWindowActiveState,
                ),
                Material::Popover => (NSVisualEffectMaterial::Popover, NSVisualEffectState::Active),
            };
            view.setMaterial(material);
            view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            view.setState(state);
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

    impl Drop for View {
        fn drop(&mut self) {
            self.view.removeFromSuperview();
        }
    }
}
