//! Every duration, curve and spring the interface moves with.
//!
//! Motion explains a change and stays out of the way: short, eased out,
//! interruptible from wherever it is. All of it goes through gpui-base's
//! transition, spring and presence primitives, which follow the system's
//! reduced-motion setting: spatial motion then jumps to its end, and fades
//! keep their timing only where noted.

use std::time::Duration;

use gpui::base::{Easing, Presence, PresenceSample, ReducedMotion, Spring, Transition};
use gpui::{
    Animation, AnimationElement, AnimationExt as _, App, ElementId, IntoElement, Styled, Window, px,
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

/// Movement, which is skipped under reduced motion.
pub fn movement(duration: Duration) -> Transition {
    Transition::new(duration).easing(ease_out())
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
/// another item: it fades in while rising a few points. A new `id` restarts
/// it, so pass one that names the content.
pub fn content_enter<E: IntoElement + Styled + 'static>(
    id: impl Into<ElementId>,
    element: E,
) -> AnimationElement<E> {
    let easing = ease_out();
    element.with_animation(
        id,
        Animation::new(STANDARD).with_easing(move |t| easing.sample(t)),
        |element, t| element.opacity(t).mt(px(6.) * (1. - t)),
    )
}
