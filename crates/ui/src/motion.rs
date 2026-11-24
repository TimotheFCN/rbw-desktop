//! Every duration, curve and spring the interface moves with.
//!
//! Motion explains a change and stays out of the way, as in the platform's
//! own apps:
//!
//! * What moves or changes size travels on a spring ([`SELECTION`],
//!   [`RESIZE`], [`SETTLE`]), so that a new target takes over from where it
//!   is and how fast it goes, never from the start.
//! * What appears in place fades, briefly ([`ARRIVE`], [`replace`]); what
//!   leaves goes faster than it came ([`DISMISS`], [`QUICK`]).
//! * Distances are short: a few points for anything that enters
//!   ([`enter`], [`presence`]). A selection travels to any row on the same
//!   spring, which keeps a long way short in time, and comes in from the
//!   edge of a list when it was off screen ([`slide`]).
//! * Nothing replays: element animations are keyed by what they show, so
//!   drawing the same thing again starts nothing.
//!
//! Transitions, springs and presence go through gpui-base's primitives;
//! content arriving and screens changing use GPUI's own animation, keyed by
//! what they show. Both follow the system's reduced-motion setting: what
//! moves or resizes is at its end at once, while fades keep their timing,
//! since opacity is not motion.

use std::rc::Rc;
use std::time::Duration;

use gpui::base::{
    Easing, Keyframe, Keyframes, MotionReveal, Presence, PresenceSample, ReducedMotion,
    ScrollbarMotion, Spring, Timing, Transition, animate_keyframes, spring,
};
use gpui::{
    Animation, AnimationElement, AnimationExt as _, AnyElement, App, Bounds, ContentMask, Element,
    ElementId, GlobalElementId, InspectorElementId, IntoElement, LayoutId, Pixels, Point, Style,
    Styled, Window, px, relative,
};

/// Feedback leaving: a highlight leaving a row, a "Copied" going.
pub const QUICK: Duration = Duration::from_millis(120);
/// Something appearing in its place: a website's icon, an item's fields
/// once fetched, a new value.
pub const ARRIVE: Duration = Duration::from_millis(200);
/// A sheet, an alert or a popover arriving, on [`spring_curve`].
pub const PRESENT: Duration = Duration::from_millis(320);
/// A sheet, an alert or a popover leaving: faster than arriving, as on
/// macOS.
pub const DISMISS: Duration = Duration::from_millis(160);
/// One screen replacing another in a window, such as the vault after
/// unlocking: the old one fades out in its first part, the new one comes
/// in after it.
pub const SCREEN: Duration = Duration::from_millis(340);
/// A scroll the app makes, such as a page from a click in a scroll bar's
/// track, on [`spring_curve`].
pub const SCROLL: Duration = Duration::from_millis(300);
/// How long a "Copied" confirmation stays before fading.
pub const CONFIRMATION_HOLD: Duration = Duration::from_millis(1400);

/// A selection moving: a list's highlight to the next row, a segmented
/// control's thumb, a switch's knob. Critically damped: quick, and a
/// second change mid-way redirects it without overshoot.
pub const SELECTION: Spring = Spring::new(Duration::from_millis(240));
/// Something changing size to fit what it holds, such as the generator's
/// value when its length changes, or a card that gains a row. Critically
/// damped: a size has its neighbours to keep clear of.
pub const RESIZE: Spring = Spring::new(Duration::from_millis(300)).with_epsilon(0.5);
/// Something let go that returns to its place, such as the list's divider
/// put back with a double-click: with a slight overshoot, as a thing with
/// weight settles.
pub const SETTLE: Spring = Spring::new(Duration::from_millis(420))
    .with_damping(0.8)
    .with_epsilon(0.0005);

/// Decelerates hard and settles gently, like AppKit's default curve.
pub fn ease_out() -> Easing {
    Easing::CubicBezier {
        x1: 0.2,
        y1: 0.8,
        x2: 0.2,
        y2: 1.0,
    }
}

/// Accelerates away, for things leaving.
pub fn ease_in() -> Easing {
    Easing::CubicBezier {
        x1: 0.4,
        y1: 0.0,
        x2: 1.0,
        y2: 1.0,
    }
}

/// A critically damped spring's way to its target, over a duration: fast
/// at first, with the long, soft landing of a physical thing, which a
/// Bézier curve doesn't have. For what arrives with a duration rather than
/// a target to follow, such as a sheet.
pub fn spring_curve() -> Easing {
    // How many radians of the spring's frequency the duration holds: by
    // its end, less than 0.2% of the way is left, which the curve closes.
    const OMEGA: f32 = 8.5;
    let end = 1. - (1. + OMEGA) * (-OMEGA).exp();
    Easing::Custom(Rc::new(move |t| {
        (1. - (1. + OMEGA * t) * (-OMEGA * t).exp()) / end
    }))
}

/// How long a scroll bar stays after the scrolling stops, or the pointer
/// leaves it.
const SCROLLBAR_HOLD: Duration = Duration::from_millis(900);

/// Scroll bars: in quickly while scrolling, held a moment after it, then
/// faded out slowly; widening under the pointer as quickly as hover
/// feedback. Reduced motion shows and hides them at once.
pub fn scrollbar() -> ScrollbarMotion {
    ScrollbarMotion::default()
        .with_idle(SCROLLBAR_HOLD)
        .with_enter(QUICK)
        .with_exit(PRESENT)
        .with_expand(QUICK)
}

/// A fade that keeps its timing under reduced motion: opacity is not motion.
pub fn fade(duration: Duration) -> Transition {
    Transition::new(duration)
        .easing(ease_out())
        .reduced_motion(ReducedMotion::Continue)
}

/// Samples an enter/exit transition for something that appears and leaves,
/// such as a sheet or a confirmation. Its progress runs 0 → 1 while arriving
/// on [`spring_curve`] and back to 0, faster, while leaving; reversed
/// mid-way, it turns from where it is. The caller keeps rendering it until
/// [`PresenceSample::should_render`] turns false.
pub fn presence(
    id: impl Into<ElementId>,
    present: bool,
    window: &mut Window,
    cx: &mut App,
) -> PresenceSample {
    let transition = if present {
        Transition::new(PRESENT).easing(spring_curve())
    } else {
        Transition::new(DISMISS).easing(ease_in())
    };
    Presence::new(id.into(), present)
        .transition(transition)
        .sample(window, cx)
}

/// Content replacing other content in the same place, such as the item
/// pane showing another item, a new value or a revealed password: it fades
/// up from faint, without moving, so the change is seen and nothing
/// shifts, and stepping quickly through a list never shows an empty pane.
/// A new `id` restarts it, so pass one that names the content. Under
/// reduced motion the content shows at once.
pub fn replace<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    element: E,
) -> AnimationElement<E> {
    let easing = ease_out();
    element.with_animation(
        id,
        Animation::new(ARRIVE).with_easing(move |t| easing.sample(t)),
        |element, t| element.opacity(0.5 + 0.5 * t),
    )
}

/// Something arriving where there was nothing, such as an item's fields
/// once fetched or a new row: it fades in. A new `id` restarts it.
pub fn appear<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    element: E,
) -> AnimationElement<E> {
    let easing = ease_out();
    element.with_animation(
        id,
        Animation::new(ARRIVE).with_easing(move |t| easing.sample(t)),
        |element, t| element.opacity(t),
    )
}

/// A part replacing another in place, such as the next step of signing in,
/// or a field appearing in a form: it fades in while settling from `from`,
/// a few points off, into place, on [`spring_curve`]. A new `id` restarts
/// it, so pass one that names the part.
pub fn enter<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    element: E,
    from: Point<Pixels>,
) -> AnimationElement<E> {
    let curve = spring_curve();
    element.with_animation(
        id,
        Animation::new(PRESENT).with_easing(move |t| curve.sample(t)),
        move |element, t| settled(element, t, from),
    )
}

/// The share of a screen change in which the screen leaving fades out,
/// before the new one fades in: the two are never seen at once.
const HANDOVER: f32 = 0.3;

/// A screen replacing another in the window, such as the vault after
/// unlocking: it waits for [`screen_leave`] to clear the old one, then
/// fades in while settling from `from`. Both last [`SCREEN`]; a new `id`
/// restarts it.
pub fn screen_replace<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    element: E,
    from: Point<Pixels>,
) -> AnimationElement<E> {
    let curve = spring_curve();
    element.with_animation(id, Animation::new(SCREEN), move |element, t| {
        let t = curve.sample(((t - HANDOVER) / (1. - HANDOVER)).clamp(0., 1.));
        settled(element, t, from)
    })
}

/// The screen [`screen_replace`] replaces, fading out in the first part of
/// the change.
pub fn screen_leave<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    element: E,
) -> AnimationElement<E> {
    let easing = ease_in();
    element.with_animation(id, Animation::new(SCREEN), move |element, t| {
        element.opacity(1. - easing.sample((t / HANDOVER).min(1.)))
    })
}

fn settled<E: Styled>(element: E, t: f32, from: Point<Pixels>) -> E {
    element
        .opacity(t.min(1.))
        .left(from.x * (1. - t))
        .top(from.y * (1. - t))
}

/// Something that shows or hides below others, such as a line of choices
/// under a search field: it opens to its height while fading in, and
/// closes faster, so that what is under it moves with it rather than jump.
/// While `shown` is false, keep passing the child it showed, until this
/// gives `None`.
pub fn reveal(
    id: impl Into<ElementId>,
    shown: bool,
    child: impl IntoElement + Styled,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    let id = id.into();
    let presence = presence((id.clone(), "presence"), shown, window, cx);
    presence.should_render().then(|| {
        let p = presence.progress;
        MotionReveal::new(id, p, child.opacity(p).into_any_element()).into_any_element()
    })
}

/// A share of the window that something let go returns to, such as the
/// list's width put back by a double-click on its divider: it settles on
/// the [`SETTLE`] spring, and follows at once while `held`, as under a
/// drag.
pub fn settle(
    id: impl Into<ElementId>,
    share: f32,
    held: bool,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    spring(id.into(), share, SETTLE.with_travel(!held), window, cx)
}

/// Where a selection's highlight was and is going, kept between frames.
#[derive(Clone, Copy)]
struct Slide {
    /// The row it goes to.
    target: f32,
    /// Where it was drawn last.
    at: f32,
    /// How far the highlight is from the spring's own position: moved when
    /// the highlight starts its way from closer than where it was.
    shift: f32,
}

/// Where a selection moving along a list is, in rows: it travels to its
/// row on the [`SELECTION`] spring from wherever it is, near or far, and a
/// new row redirects it on the way, so that arrow keys held down keep it
/// close behind. From a row off screen it comes in from the edge on that
/// side: `on_screen` says which rows show once the list has scrolled to
/// the selection, for a list that shows only some. After the list changed
/// under it (`jump`) it is on its row at once, since where it was means
/// nothing any more. Draw each row's share of it from how far it is from
/// that row.
pub fn slide(
    id: impl Into<ElementId>,
    row: f32,
    jump: bool,
    on_screen: Option<std::ops::Range<f32>>,
    window: &mut Window,
    cx: &mut App,
) -> f32 {
    let id = id.into();
    let state = window.use_keyed_state((id.clone(), "slide"), cx, |_, _| Slide {
        target: row,
        at: row,
        shift: 0.,
    });
    let mut slide = *state.read(cx);
    if slide.target != row && !jump {
        // The spring keeps its speed; only where it starts moves.
        slide.shift += slide_start(slide.at, on_screen) - slide.at;
    }
    slide.target = row;
    let spring_at = spring(
        (id, "slide"),
        row - slide.shift,
        SELECTION.with_travel(!jump),
        window,
        cx,
    );
    slide.at = spring_at + slide.shift;
    state.update(cx, |state, _| *state = slide);
    slide.at
}

/// Where a highlight at `at` starts its way from: where it is, or just
/// past the edge of the rows on screen when it is further.
fn slide_start(at: f32, on_screen: Option<std::ops::Range<f32>>) -> f32 {
    match on_screen {
        Some(rows) => at.clamp(rows.start - 1., rows.end),
        None => at,
    }
}

/// The rows a list shows once it has scrolled to `row`, the least it can,
/// from those it `drew` last: for [`slide`].
pub fn rows_on_screen(drew: &std::ops::Range<usize>, row: usize) -> Option<std::ops::Range<f32>> {
    let count = drew.len();
    if count == 0 {
        return None;
    }
    let start = if row < drew.start {
        row
    } else if row >= drew.end {
        row + 1 - count
    } else {
        drew.start
    };
    Some(start as f32..(start + count) as f32)
}

/// Grows or shrinks to fit `child` on the [`RESIZE`] spring when what it
/// holds changes height, such as the generator's value when its length
/// changes; content that is more than it shows is clipped meanwhile. It
/// takes the height of its first content at once.
pub fn grow(id: impl Into<ElementId>, child: impl IntoElement) -> Grow {
    Grow {
        id: id.into(),
        child: child.into_any_element(),
    }
}

pub struct Grow {
    id: ElementId,
    child: AnyElement,
}

#[derive(Clone, Copy, Default)]
struct GrowState {
    /// The child's height when it was last laid out.
    natural: Option<Pixels>,
}

impl Element for Grow {
    /// The child's layout, and the height shown.
    type RequestLayoutState = (LayoutId, Option<Pixels>);
    /// Whether the child is clipped.
    type PrepaintState = bool;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let natural = window.with_element_state(
            global_id.expect("Grow has an id"),
            |state: Option<GrowState>, _| {
                let state = state.unwrap_or_default();
                (state.natural, state)
            },
        );
        // Until the child was laid out once, its own height.
        let shown = natural.map(|natural| spring("height", natural, RESIZE, window, cx));
        let child = self.child.request_layout(window, cx);
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        if let Some(shown) = shown {
            style.size.height = shown.into();
        }
        (window.request_layout(style, [child], cx), (child, shown))
    }

    fn prepaint(
        &mut self,
        global_id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        (child, shown): &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        let natural = window.layout_bounds(*child).size.height;
        let changed = window.with_element_state(
            global_id.expect("Grow has an id"),
            |state: Option<GrowState>, _| {
                let mut state = state.unwrap_or_default();
                let changed = state.natural != Some(natural);
                state.natural = Some(natural);
                (changed, state)
            },
        );
        if changed {
            window.request_animation_frame();
        }
        // Clipped only while it is smaller than what it holds, so that
        // shadows and focus rings outside the content show otherwise.
        let clip = shown.is_some_and(|shown| shown < natural);
        window.with_content_mask(clip.then_some(ContentMask { bounds }), |window| {
            self.child.prepaint(window, cx);
        });
        clip
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        clip: &mut bool,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.with_content_mask(clip.then_some(ContentMask { bounds }), |window| {
            self.child.paint(window, cx);
        });
    }
}

/// How far a field shakes sideways after a wrong entry, such as a wrong
/// password: a few quick, fading swings. Each new `generation` shakes again;
/// zero never shakes. Reduced motion keeps it still, so pair it with a
/// message.
pub fn shake(
    id: impl Into<ElementId>,
    generation: usize,
    window: &mut Window,
    cx: &mut App,
) -> Pixels {
    if generation == 0 {
        return px(0.);
    }
    let progress = animate_keyframes(
        ElementId::from((id.into(), format!("shake-{generation}"))),
        &linear(),
        Timing::new(SHAKE),
        window,
        cx,
    )
    .value;
    let swing = (progress * 3. * std::f32::consts::TAU).sin();
    px(7.) * swing * (1. - progress)
}

const SHAKE: Duration = Duration::from_millis(420);

/// The current turn, 0 to 1, of something spinning while work goes on. A
/// spinner is the one thing that keeps moving under reduced motion, slower,
/// since a still one would read as stuck.
pub fn spin(id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> f32 {
    let period = if cx.reduce_motion() {
        Duration::from_millis(2400)
    } else {
        Duration::from_millis(800)
    };
    let started = window.use_keyed_state(ElementId::from((id.into(), "spin")), cx, |_, cx| {
        cx.background_executor().now()
    });
    let elapsed = cx
        .background_executor()
        .now()
        .saturating_duration_since(*started.read(cx));
    window.request_animation_frame();
    (elapsed.as_secs_f32() / period.as_secs_f32()).fract()
}

fn linear() -> Keyframes<f32> {
    Keyframes::try_new([Keyframe::new(0., 0.), Keyframe::new(1., 1.)])
        .expect("two frames from 0 to 1")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_selection_off_screen_comes_in_from_the_edge() {
        // On screen, it starts where it is.
        assert_eq!(slide_start(4., Some(2. ..12.)), 4.);
        // Above or below the rows on screen, just past the edge.
        assert_eq!(slide_start(-30., Some(2. ..12.)), 1.);
        assert_eq!(slide_start(400., Some(2. ..12.)), 12.);
        // A list with every row on screen.
        assert_eq!(slide_start(400., None), 400.);
    }

    #[test]
    fn a_list_scrolls_the_least_to_show_its_selection() {
        assert_eq!(rows_on_screen(&(10..20), 12), Some(10. ..20.));
        assert_eq!(rows_on_screen(&(10..20), 25), Some(16. ..26.));
        assert_eq!(rows_on_screen(&(10..20), 3), Some(3. ..13.));
        assert_eq!(rows_on_screen(&(0..0), 3), None);
    }

    #[test]
    fn the_spring_curve_runs_from_start_to_end_without_passing_it() {
        let curve = spring_curve();
        assert_eq!(curve.sample(0.), 0.);
        assert!((curve.sample(1.) - 1.).abs() < 1e-6);
        let samples: Vec<f32> = (0..=20).map(|i| curve.sample(i as f32 / 20.)).collect();
        assert!(samples.windows(2).all(|pair| pair[0] <= pair[1]));
        // Most of the way early, as a spring goes.
        assert!(curve.sample(0.3) > 0.7);
    }
}
