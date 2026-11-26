//! Scroll bars, on every list and pane that scrolls: overlay bars that show
//! while scrolling and fade out after it. On macOS the bar also shows while
//! the pointer is over its track, widens there over a track of its own, and
//! a click in the track pages, as in AppKit; on GNOME it is Adwaita's thin
//! indicator, which widens into a bar under the pointer, and a click in the
//! track moves the thumb there, as in GTK. The thumb drags in both.
//!
//! [`scroll_area`] is a column that scrolls with its bar; [`scrollbar`] adds
//! the bar to something that scrolls on its own, such as a `uniform_list`.
//! Behaviour comes from gpui-base's `Scrollbar`; its looks and timing come
//! from [`style`], which the theme projects onto gpui-base.

use gpui::base::{Scrollbar, ScrollbarHandle, ScrollbarMode, ScrollbarStyles, ScrollbarTheme};
use std::time::Instant;

use gpui::{
    AnyElement, App, Bounds, Div, ElementId, Entity, HitboxBehavior, InteractiveElement as _,
    IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, RenderOnce, ScrollHandle,
    StatefulInteractiveElement as _, StyleRefinement, Styled, Window, canvas, div, point, px,
};

use crate::motion;
use crate::theme::{ActiveTheme, Appearance, Look, Theme};

/// The strip along the right edge that the bar takes, and that takes the
/// pointer while the bar shows.
const TRACK: Pixels = px(14.);
/// The shortest the thumb gets, however long the content.
const MIN_THUMB: Pixels = px(28.);

/// How the look draws scroll bars, for gpui-base's scrollbars.
pub(crate) fn style(theme: &Theme) -> ScrollbarTheme {
    let ink = theme.colors.text;
    let mode = match theme.look {
        Look::MacOs => ScrollbarMode::Hover,
        Look::Gnome => ScrollbarMode::Scrolling,
    };
    let styles = match theme.look {
        // A rounded knob that widens over a translucent track with a
        // hairline edge.
        Look::MacOs => {
            let (track, edge) = match theme.appearance {
                Appearance::Light => (gpui::white().opacity(0.8), gpui::black().opacity(0.08)),
                Appearance::Dark => (gpui::black().opacity(0.35), gpui::white().opacity(0.1)),
            };
            ScrollbarStyles::default()
                .track(|track| track.width(TRACK))
                .track_hover(|style| style.bg(track).border_color(edge))
                .track_active(|style| style.bg(track).border_color(edge))
                .thumb(|thumb| {
                    thumb
                        .bg(ink.opacity(0.4))
                        .width(px(7.))
                        .inset(px(3.))
                        .radius(px(4.))
                        .min_length(MIN_THUMB)
                })
                .thumb_hover(|thumb| thumb.bg(ink.opacity(0.5)).width(px(9.)).inset(px(2.5)))
                .thumb_active(|thumb| thumb.bg(ink.opacity(0.55)).width(px(9.)).inset(px(2.5)))
        }
        // A thin indicator, then Adwaita's 8-pixel slider.
        Look::Gnome => ScrollbarStyles::default()
            .track(|track| {
                track
                    .width(TRACK)
                    .bg(gpui::transparent_black())
                    .border_color(gpui::transparent_black())
            })
            .thumb(|thumb| {
                thumb
                    .bg(ink.opacity(0.2))
                    .width(px(3.))
                    .inset(px(3.))
                    .radius(px(4.))
                    .min_length(MIN_THUMB)
            })
            .thumb_hover(|thumb| thumb.bg(ink.opacity(0.4)).width(px(8.)))
            .thumb_active(|thumb| thumb.bg(ink.opacity(0.6)).width(px(8.))),
    };
    ScrollbarTheme::new()
        .with_mode(mode)
        .with_motion(motion::scrollbar())
        .with_styles(styles)
}

/// The scroll bar of what `handle` scrolls, over its right edge. Put it
/// after the content it scrolls, in any parent: it finds its place from the
/// handle.
pub fn scrollbar<H: ScrollbarHandle + Clone>(
    id: impl Into<ElementId>,
    handle: &H,
    cx: &App,
) -> AnyElement {
    let id = id.into();
    let paging = (cx.theme().look == Look::MacOs)
        .then(|| page_on_click((id.clone(), "paging").into(), handle.clone()));
    div()
        .absolute()
        .inset_0()
        .child(Scrollbar::vertical(handle).id(id))
        .children(paging)
        .into_any_element()
}

/// A page scroll under way, from a click in the track.
#[derive(Clone, Copy)]
struct Paging {
    from: Pixels,
    to: Pixels,
    started: Instant,
    /// The offset it set last: another means the content was scrolled
    /// meanwhile, which stops it.
    last: Pixels,
}

/// A click in the track, off the thumb, scrolls a page towards it, as
/// AppKit's scrollers do, gliding there rather than jumping. It takes the
/// click before the bar's own handler, which would move the thumb there.
fn page_on_click<H: ScrollbarHandle + Clone>(id: ElementId, handle: H) -> impl IntoElement {
    let prepaint_handle = handle.clone();
    canvas(
        move |_, window, cx| {
            let paging = window.use_keyed_state(id, cx, |_, _| None::<Paging>);
            glide(&paging, &prepaint_handle, window, cx);
            let track = track(&prepaint_handle)?;
            let hitbox = window.insert_hitbox(track, HitboxBehavior::Normal);
            Some((hitbox, track, paging))
        },
        move |_, hit, window, _| {
            let Some((hitbox, track, paging)) = hit else {
                return;
            };
            let handle = handle.clone();
            window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
                if !phase.capture()
                    || event.button != MouseButton::Left
                    || !hitbox.is_hovered(window)
                {
                    return;
                }
                let Some(to) = page(&handle, track, event.position.y) else {
                    return;
                };
                let from = handle.offset().y;
                let started = cx.background_executor().now();
                paging.update(cx, |paging, _| {
                    *paging = Some(Paging {
                        from,
                        to,
                        started,
                        last: from,
                    })
                });
                cx.stop_propagation();
                window.refresh();
            });
        },
    )
    .absolute()
    .inset_0()
}

/// Moves a page scroll on to where it is due in this frame.
fn glide(
    paging: &Entity<Option<Paging>>,
    handle: &impl ScrollbarHandle,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(run) = *paging.read(cx) else {
        return;
    };
    let offset = handle.offset();
    if offset.y != run.last {
        paging.update(cx, |paging, _| *paging = None);
        return;
    }
    let elapsed = cx
        .background_executor()
        .now()
        .saturating_duration_since(run.started);
    let t = if cx.reduce_motion() {
        1.
    } else {
        (elapsed.as_secs_f32() / motion::SCROLL.as_secs_f32()).min(1.)
    };
    let y = run.from + (run.to - run.from) * motion::spring_curve().sample(t);
    handle.set_offset(point(offset.x, y));
    paging.update(cx, |paging, _| {
        *paging = (t < 1.).then_some(Paging { last: y, ..run })
    });
    // The content was drawn before this moved it, so a frame more draws
    // where it is now; once at its end and drawn there, none is needed.
    if t < 1. || y != offset.y {
        window.request_animation_frame();
    }
}

/// The track of a handle's viewport, when its content is taller than it.
fn track(handle: &impl ScrollbarHandle) -> Option<Bounds<Pixels>> {
    let viewport = handle.viewport_bounds();
    (handle.content_size().height > viewport.size.height).then(|| {
        Bounds::new(
            point(viewport.right() - TRACK, viewport.top()),
            gpui::size(TRACK, viewport.size.height),
        )
    })
}

/// The offset a click at `y` in the track pages to, or `None` on the thumb,
/// or when there is nothing to scroll any more, as after the viewport grew
/// since the track was laid out.
fn page(handle: &impl ScrollbarHandle, track: Bounds<Pixels>, y: Pixels) -> Option<Pixels> {
    let viewport = handle.viewport_bounds().size.height;
    let extent = handle.content_size().height - viewport;
    if extent <= px(0.) {
        return None;
    }
    let length = track.size.height;
    let thumb = (viewport / (extent + viewport) * length)
        .max(MIN_THUMB)
        .min(length);
    let offset = handle.offset().y;
    let start = track.top() + (-offset / extent).clamp(0., 1.) * (length - thumb);
    let page = viewport * 0.9;
    if y < start {
        Some((offset + page).min(px(0.)))
    } else if y > start + thumb {
        Some((offset - page).max(-extent))
    } else {
        None
    }
}

/// A column that scrolls vertically, with its scroll bar. Style it as the
/// column; its children scroll.
#[derive(IntoElement)]
pub struct ScrollArea {
    id: ElementId,
    handle: Option<ScrollHandle>,
    base: Div,
}

pub fn scroll_area(id: impl Into<ElementId>) -> ScrollArea {
    ScrollArea {
        id: id.into(),
        handle: None,
        base: div(),
    }
}

impl ScrollArea {
    /// Scrolls with `handle`, for a view that scrolls it itself; otherwise
    /// the area keeps a handle of its own.
    pub fn track_scroll(mut self, handle: &ScrollHandle) -> Self {
        self.handle = Some(handle.clone());
        self
    }
}

impl Styled for ScrollArea {
    fn style(&mut self) -> &mut StyleRefinement {
        self.base.style()
    }
}

impl ParentElement for ScrollArea {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.base.extend(elements);
    }
}

impl RenderOnce for ScrollArea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let handle = self.handle.unwrap_or_else(|| {
            window
                .use_keyed_state((self.id.clone(), "scroll"), cx, |_, _| ScrollHandle::new())
                .read(cx)
                .clone()
        });
        let bar = scrollbar((self.id.clone(), "bar"), &handle, cx);
        self.base
            .id(self.id)
            .overflow_y_scroll()
            .track_scroll(&handle)
            .child(bar)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gpui::{Point, Size, size};

    use super::*;

    /// A viewport 100 points tall at the window's top, over content
    /// `content` tall, scrolled by `offset`.
    struct Handle {
        content: Pixels,
        offset: Cell<Pixels>,
    }

    impl ScrollbarHandle for Handle {
        fn viewport_bounds(&self) -> Bounds<Pixels> {
            Bounds::new(point(px(0.), px(0.)), size(px(200.), px(100.)))
        }

        fn offset(&self) -> Point<Pixels> {
            point(px(0.), self.offset.get())
        }

        fn set_offset(&self, offset: Point<Pixels>) {
            self.offset.set(offset.y);
        }

        fn content_size(&self) -> Size<Pixels> {
            size(px(200.), self.content)
        }
    }

    fn track() -> Bounds<Pixels> {
        Bounds::new(point(px(186.), px(0.)), size(TRACK, px(100.)))
    }

    #[test]
    fn a_click_in_the_track_pages_towards_it() {
        let handle = Handle {
            content: px(1000.),
            offset: Cell::new(px(-450.)),
        };
        // The thumb is 28 points from 36 down: under it, a page down;
        // over it, a page up; on it, nothing.
        assert_eq!(page(&handle, track(), px(90.)), Some(px(-540.)));
        assert_eq!(page(&handle, track(), px(10.)), Some(px(-360.)));
        assert_eq!(page(&handle, track(), px(55.)), None);
        // Never past either end.
        handle.offset.set(px(-880.));
        assert_eq!(page(&handle, track(), px(99.)), Some(px(-900.)));
        handle.offset.set(px(-50.));
        assert_eq!(page(&handle, track(), px(2.)), Some(px(0.)));
    }

    #[test]
    fn a_click_in_a_track_with_nothing_left_to_scroll_pages_nowhere() {
        // The viewport grew to the content's height, or past it, since the
        // track was laid out.
        for content in [px(100.), px(60.)] {
            let handle = Handle {
                content,
                offset: Cell::new(px(0.)),
            };
            assert_eq!(page(&handle, track(), px(90.)), None);
            assert_eq!(page(&handle, track(), px(10.)), None);
        }
    }
}
