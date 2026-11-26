//! The row of a list the pointer is on, for the lists that show it
//! themselves: the item list's hover, and the menu bar panel's selection,
//! which follows the pointer.
//!
//! GPUI tells which row is under the pointer from where the frame before
//! drew the rows. A list that scrolls while the pointer moves has moved its
//! rows since, so that row flips between neighbours until both stop. So
//! these lists hold their pointer's row while they scroll, as native lists
//! do, and settle it once the scroll stops, from where the pointer is and
//! where the list has scrolled to.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui::{
    Bounds, Context, Div, InteractiveElement as _, Pixels, Point, Stateful,
    StatefulInteractiveElement as _, UniformListScrollHandle, Window,
};

/// How long after its last wheel event a list still counts as scrolling,
/// as long as GPUI holds its own hover still after one.
const SCROLL_HOLD: Duration = Duration::from_millis(100);

/// The rows of a `uniform_list`, for [`follow_pointer`].
#[derive(Clone, Copy, Debug)]
pub struct ListRows {
    pub count: usize,
    /// How tall a row is. The pointer in the gap between two rows stays
    /// on the row above, so that crossing from one row to the next goes
    /// straight from one to the other; past the last row it is on none.
    pub height: Pixels,
    /// How far apart the rows' tops are.
    pub pitch: Pixels,
}

/// Where the pointer is on a list, kept by [`follow_pointer`]: whether it
/// is over the list, and whether the list is scrolling. A handle, as a
/// scroll handle is: keep one in the view and pass it on each render.
///
/// It is not the view's state: a wheel event that only scrolls the list
/// changes nothing the view draws, so it must not update the view, or GPUI
/// would draw every row of the list again for it.
#[derive(Clone, Default)]
pub struct ListPointer(Rc<PointerState>);

#[derive(Default)]
struct PointerState {
    over: Cell<bool>,
    /// The wheel's last event, while the list scrolls.
    scrolled: Cell<Option<Instant>>,
    /// Where the list had scrolled to at the wheel's last event.
    offset: Cell<Point<Pixels>>,
}

/// Follows the pointer over `list`, the element around a `uniform_list`
/// that `scroll` scrolls: `on_row` hears the row under the pointer each
/// time the pointer moves, or comes onto the list or leaves it (`None`),
/// except while the list scrolls. A scroll starting hears `None`, and the
/// scroll stopping hears the row it left under the pointer, once.
pub fn follow_pointer<V: 'static>(
    list: Stateful<Div>,
    pointer: &ListPointer,
    scroll: &UniformListScrollHandle,
    rows: ListRows,
    on_row: fn(&mut V, Option<usize>, &mut Window, &mut Context<V>),
    cx: &mut Context<V>,
) -> Stateful<Div> {
    let view = cx.entity().downgrade();
    let report = move |row: Option<usize>, window: &mut Window, cx: &mut gpui::App| {
        view.update(cx, |view, cx| on_row(view, row, window, cx))
            .ok();
    };
    let under = {
        let (pointer, scroll) = (pointer.clone(), scroll.clone());
        move |window: &Window| {
            let state = scroll.0.borrow();
            let handle = &state.base_handle;
            pointer.0.over.get().then(|| {
                row_at(
                    window.mouse_position(),
                    handle.bounds(),
                    handle.offset().y,
                    rows,
                )
            })?
        }
    };
    let (hover_pointer, hover_report, hover_under) =
        (pointer.clone(), report.clone(), under.clone());
    let (move_pointer, move_report, move_under) = (pointer.clone(), report.clone(), under.clone());
    let (wheel_pointer, wheel_scroll) = (pointer.clone(), scroll.clone());
    list.on_hover(move |over, window, cx| {
        hover_pointer.0.over.set(*over);
        if !hover_pointer.scrolling() {
            hover_report(hover_under(window), window, cx);
        }
    })
    .on_mouse_move(move |_, window, cx| {
        if !move_pointer.scrolling() {
            move_report(move_under(window), window, cx);
        }
    })
    .on_scroll(move |_, window, cx| {
        let state = &wheel_pointer.0;
        let offset = wheel_scroll.0.borrow().base_handle.offset();
        if state.offset.replace(offset) == offset {
            // A wheel past the end of the list moves nothing.
            return;
        }
        if state.scrolled.replace(Some(Instant::now())).is_some() {
            return;
        }
        report(None, window, cx);
        let (pointer, report, under) = (wheel_pointer.clone(), report.clone(), under.clone());
        window
            .spawn(cx, async move |cx| {
                // Until the wheel has rested long enough.
                while let Some(last) = pointer.0.scrolled.get() {
                    let rest = SCROLL_HOLD.saturating_sub(last.elapsed());
                    if rest.is_zero() {
                        pointer.0.scrolled.set(None);
                        break;
                    }
                    cx.background_executor().timer(rest).await;
                }
                cx.update(|window, cx| report(under(window), window, cx))
                    .ok();
            })
            .detach();
    })
}

impl ListPointer {
    fn scrolling(&self) -> bool {
        self.0.scrolled.get().is_some()
    }
}

/// The row under `pointer`, in a list at `list` scrolled by `offset` (up
/// from its top, so negative once it scrolled down). The gap between two
/// rows belongs to the row above it: were it on no row, the pointer
/// crossing it would leave the row and come onto the next, and the list
/// would draw twice for one move.
fn row_at(
    pointer: Point<Pixels>,
    list: Bounds<Pixels>,
    offset: Pixels,
    rows: ListRows,
) -> Option<usize> {
    if !list.contains(&pointer) {
        return None;
    }
    let y = pointer.y - list.top() - offset;
    if y < Pixels::ZERO {
        return None;
    }
    let row = (y / rows.pitch).floor() as usize;
    if row >= rows.count {
        return None;
    }
    let within = y - rows.pitch * row as f32;
    (row + 1 < rows.count || within < rows.height).then_some(row)
}

#[cfg(test)]
mod tests {
    use gpui::{point, px, size};

    use super::*;

    #[test]
    fn the_row_under_the_pointer_is_where_the_list_has_scrolled_to() {
        let list = Bounds::new(point(px(200.), px(50.)), size(px(300.), px(400.)));
        let rows = ListRows {
            count: 100,
            height: px(46.),
            pitch: px(48.),
        };
        let at = |y: f32, offset: f32| row_at(point(px(300.), px(y)), list, px(offset), rows);
        // At the top: the rows from the list's top.
        assert_eq!(at(51., 0.), Some(0));
        assert_eq!(at(50. + 48. * 3. + 10., 0.), Some(3));
        // In the gap under a row: still on it, rather than on none
        // between two rows.
        assert_eq!(at(50. + 46., 0.), Some(0));
        assert_eq!(at(50. + 47.9, 0.), Some(0));
        assert_eq!(at(50. + 48., 0.), Some(1));
        assert_eq!(at(50. + 48. * 3. + 47., -0.5), Some(3));
        // Scrolled down by two rows and a half: the same place is on
        // another row.
        assert_eq!(at(51., -120.), Some(2));
        assert_eq!(at(50. + 48. * 3. + 10., -120.), Some(5));
        // Off the list, or past its last row.
        assert_eq!(at(20., 0.), None);
        assert_eq!(row_at(point(px(100.), px(60.)), list, px(0.), rows), None);
        let few = ListRows { count: 2, ..rows };
        assert_eq!(row_at(point(px(300.), px(160.)), list, px(0.), few), None);
        // The gap under the last row is past the list's end.
        let last = |y: f32| row_at(point(px(300.), px(y)), list, px(0.), few);
        assert_eq!(last(50. + 48. + 45.), Some(1));
        assert_eq!(last(50. + 48. + 46.), None);
        assert_eq!(last(50. + 48. + 47.), None);
        // A list with no rows has none to be on.
        let none = ListRows { count: 0, ..rows };
        assert_eq!(row_at(point(px(300.), px(51.)), list, px(0.), none), None);
    }
}
