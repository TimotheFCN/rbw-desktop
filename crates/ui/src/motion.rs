//! Every duration, curve and spring the interface moves with.
//!
//! Motion explains a change and stays out of the way: short, eased out,
//! interruptible from wherever it is. All of it goes through gpui-base's
//! transition, spring and presence primitives, which follow the system's
//! reduced-motion setting: spatial motion then jumps to its end, and fades
//! keep their timing only where noted.

use std::time::Duration;

use gpui::base::{
    Easing, Keyframe, Keyframes, Presence, PresenceSample, ReducedMotion, Spring, Timing,
    Transition, animate_keyframes,
};
use gpui::{
    Animation, AnimationElement, AnimationExt as _, App, ElementId, IntoElement, Pixels, Point,
    Styled, Window, px,
};

/// Hover and press feedback, and a highlight leaving a row.
pub const QUICK: Duration = Duration::from_millis(120);
/// Content replacing other content, like the detail pane changing item.
pub const STANDARD: Duration = Duration::from_millis(220);
/// A sheet or popover arriving.
pub const PRESENT: Duration = Duration::from_millis(300);
/// A sheet or popover leaving: faster than arriving, as on macOS.
pub const DISMISS: Duration = Duration::from_millis(200);
/// How long a "Copied" confirmation stays before fading.
pub const CONFIRMATION_HOLD: Duration = Duration::from_millis(1400);

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

/// A selection highlight following the pointer between rows: critically
/// damped, so a quick second click redirects it without overshoot.
pub const SELECTION: Spring = Spring::new(Duration::from_millis(260)).with_epsilon(0.25);

/// A fade that keeps its timing under reduced motion: opacity is not motion.
pub fn fade(duration: Duration) -> Transition {
    Transition::new(duration)
        .easing(ease_out())
        .reduced_motion(ReducedMotion::Continue)
}

/// Samples an enter/exit transition for something that appears and leaves,
/// such as a sheet or a confirmation. Its progress runs 0 → 1 while arriving
/// and back to 0 while leaving, eased each way; the caller keeps rendering it
/// until [`PresenceSample::should_render`] turns false.
pub fn presence(
    id: impl Into<ElementId>,
    present: bool,
    window: &mut Window,
    cx: &mut App,
) -> PresenceSample {
    let transition = if present {
        Transition::new(PRESENT).easing(ease_out())
    } else {
        Transition::new(DISMISS).easing(ease_in())
    };
    Presence::new(id.into(), present)
        .transition(transition)
        .sample(window, cx)
}

/// Content arriving in place of other content, like the detail pane showing
/// another item: it settles into place from a few points lower while fading
/// up. It starts partly visible, so stepping quickly through a list never
/// shows an empty pane. A new `id` restarts it, so pass one that names the
/// content.
pub fn content_enter<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    element: E,
) -> AnimationElement<E> {
    let easing = ease_out();
    element.with_animation(
        id,
        Animation::new(STANDARD).with_easing(move |t| easing.sample(t)),
        |element, t| element.opacity(0.4 + 0.6 * t).mt(px(4.) * (1. - t)),
    )
}

/// New content in the same place and shape as the old, such as a freshly
/// generated password: it fades up quickly from faint, without moving, so
/// the change is seen and nothing shifts. A new `id` restarts it. Under
/// reduced motion the content shows at once.
pub fn refresh<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    element: E,
) -> AnimationElement<E> {
    let easing = ease_out();
    element.with_animation(
        id,
        Animation::new(STANDARD).with_easing(move |t| easing.sample(t)),
        |element, t| element.opacity(0.25 + 0.75 * t),
    )
}

/// A screen or a step replacing another in place, such as the vault after
/// unlocking or the next step of signing in. It is drawn over what it
/// replaces and fades in from nothing while settling from `from`, a few
/// points off, into place. Under reduced motion it only fades. A new `id`
/// restarts it, so pass one that names the screen.
pub fn screen_enter<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    element: E,
    from: Point<Pixels>,
    cx: &App,
) -> AnimationElement<E> {
    let easing = ease_out();
    let from = if cx.reduce_motion() {
        Point::default()
    } else {
        from
    };
    element.with_animation(
        id,
        Animation::new(STANDARD).with_easing(move |t| easing.sample(t)),
        move |element, t| {
            element
                .opacity(t)
                .left(from.x * (1. - t))
                .top(from.y * (1. - t))
        },
    )
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
