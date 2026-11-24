//! Surfaces of system material: the sidebar, sheets and popovers.
//!
//! On macOS 26 and later the sidebar is real Liquid Glass: an
//! `NSGlassEffectView` placed under GPUI's Metal view at the sidebar's bounds,
//! which the window's transparent background lets through. Everywhere else,
//! and for sheets and popovers (which sit over GPUI-drawn content that an
//! underlay could not show through), the surface is a translucent fill with a
//! hairline edge and the shadows of the mockups.
//!
//! The hosting technique follows gpui-macos (MIT, Copyright 2026 gpui-macos
//! contributors), `src/native/macos.rs`.

use gpui::{
    AnyElement, App, BoxShadow, Div, ElementId, IntoElement, ParentElement, RenderOnce,
    StyleRefinement, Styled, Window, div, point, px,
};

use crate::theme::{ActiveTheme, Look};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Material {
    Sidebar,
    Sheet,
    Popover,
}

#[derive(IntoElement)]
pub struct Glass {
    id: ElementId,
    material: Material,
    base: Div,
}

pub fn glass(id: impl Into<ElementId>, material: Material) -> Glass {
    Glass {
        id: id.into(),
        material,
        base: div(),
    }
}

impl Styled for Glass {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for Glass {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for Glass {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let raised = theme.metrics.raised_controls;
        let fill = match self.material {
            Material::Sidebar => colors.sidebar,
            Material::Sheet | Material::Popover => colors.frost.into(),
        };
        let edge = colors.sidebar_edge;
        let shadow = match self.material {
            Material::Sidebar if raised => vec![
                BoxShadow::new(px(0.), px(2.), colors.shadow.opacity(0.26)).blur_radius(px(12.)),
                BoxShadow::new(px(0.), px(0.), colors.shadow.opacity(0.15)).spread_radius(px(0.5)),
            ],
            Material::Sidebar => vec![],
            Material::Sheet | Material::Popover => vec![
                BoxShadow::new(px(0.), px(24.), colors.shadow).blur_radius(px(50.)),
                BoxShadow::new(px(0.), px(0.), colors.shadow.opacity(0.3)).spread_radius(px(0.5)),
            ],
        };
        let highlight = (theme.look == Look::MacOs).then(|| BoxShadow {
            color: colors.control_highlight,
            offset: point(px(0.), px(1.)),
            blur_radius: px(0.),
            spread_radius: px(0.),
            inset: true,
        });
        let base = self
            .base
            .shadow(shadow.into_iter().chain(highlight).collect());
        let base = if theme.look == Look::Gnome && self.material == Material::Sidebar {
            base
        } else {
            base.border(px(0.5)).border_color(edge)
        };

        #[cfg(target_os = "macos")]
        if self.material == Material::Sidebar
            && let Some(glass) = native::GlassView::for_element(self.id.clone(), window, cx)
        {
            return base.child(glass);
        }
        // The id and window only serve the macOS glass.
        let _ = (self.id, window);
        base.bg(fill)
    }
}

#[cfg(target_os = "macos")]
mod native {
    use std::cell::Cell;

    use gpui::{App, Bounds, ElementId, IntoElement, Pixels, Styled, Window, canvas};
    use objc2::rc::Retained;
    use objc2::runtime::AnyClass;
    use objc2_app_kit::{NSGlassEffectView, NSView, NSWindowOrderingMode};
    use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};

    use crate::theme::ActiveTheme;

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

    pub(super) struct GlassView {
        view: Retained<NSGlassEffectView>,
        placed: Cell<Option<(NSRect, f64)>>,
    }

    impl GlassView {
        /// An element that keeps real glass under its bounds, or `None` when
        /// this macOS has none. The glass view lives in GPUI's keyed element
        /// state, so it goes away when the element stops rendering.
        pub(super) fn for_element(
            id: ElementId,
            window: &mut Window,
            cx: &mut App,
        ) -> Option<impl IntoElement> {
            let state = window.use_keyed_state((id, "glass"), cx, |window, _| Self::new(window));
            state.read(cx).as_ref()?;
            let radius = cx.theme().metrics.sidebar_radius;
            Some(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        if let Some(glass) = state.read(cx) {
                            glass.place(bounds, radius, window);
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
        }

        fn new(window: &Window) -> Option<Self> {
            let mtm = MainThreadMarker::new()?;
            // NSGlassEffectView exists from macOS 26.
            AnyClass::get(c"NSGlassEffectView")?;
            let renderer = renderer_view(window)?;
            // SAFETY: reading the superview on the main thread, which is the
            // only thread that changes the view hierarchy.
            let parent = unsafe { renderer.superview() }?;
            let view = NSGlassEffectView::new(mtm);
            view.setContentView(Some(&NSView::new(mtm)));
            parent.addSubview_positioned_relativeTo(
                &view,
                NSWindowOrderingMode::Below,
                Some(&renderer),
            );
            Some(Self {
                view,
                placed: Cell::new(None),
            })
        }

        fn place(&self, bounds: Bounds<Pixels>, radius: Pixels, window: &Window) {
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
            let radius = f64::from(f32::from(radius));
            // Setting the frame on every frame would make AppKit re-layout
            // and recomposite the glass while unrelated content animates.
            if self.placed.get() != Some((frame, radius)) {
                self.view.setFrame(frame);
                self.view.setCornerRadius(radius);
                self.placed.set(Some((frame, radius)));
            }
        }
    }

    impl Drop for GlassView {
        fn drop(&mut self) {
            self.view.removeFromSuperview();
        }
    }
}
