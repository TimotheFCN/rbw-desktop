//! Popover menus hanging from a control, such as the account switcher or the
//! editor's folder menu.
//!
//! The owner keeps whether the menu is open and a focus handle for it, puts
//! [`popover`] inside the element the menu hangs from, and focuses the handle
//! when it opens. A click outside, Escape, or choosing an item closes it
//! through [`Popover::on_dismiss`]. Tab and the arrow keys move between the
//! items without leaving the menu; [`Popover::submenu`] opens more items
//! beside it. A [`Popover::dialog`] holds controls instead, such as the
//! generator's options.
//!
//! A popover is in every frame of its owner, open or not. Its items can be
//! made only while it shows, with [`Popover::when_shown`], so that a closed
//! menu costs nothing of what fills it.

use std::rc::Rc;
use std::time::Duration;

use gpui::base::{Button as BaseButton, FocusTrapElement as _};
use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, ElementId, Entity, FocusHandle, FontWeight,
    InteractiveElement, IntoElement, KeyBinding, MouseMoveEvent, ParentElement, Pixels, Point,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Task, Window, actions, anchored,
    canvas, deferred, div, point, prelude::FluentBuilder as _, px, relative,
};

use crate::icon::{IconName, icon};
use crate::motion as m;
use crate::scroll::scroll_area;
use crate::sheet::surface;
use crate::theme::ActiveTheme;

actions!(
    popover,
    [Dismiss, NextItem, PreviousItem, OpenSubmenu, CloseSubmenu]
);

const CONTEXT: &str = "Popover";
const DIALOG_CONTEXT: &str = "PopoverDialog";
const SUBMENU_CONTEXT: &str = "Submenu";

/// How long the pointer rests on a row before its submenu opens, or on
/// another row before an open one closes: long enough that a pointer
/// passing over a row opens nothing.
const SUBMENU_DELAY: Duration = Duration::from_millis(120);
/// How long a submenu stays open while the pointer heads for it across
/// other rows.
const SUBMENU_GRACE: Duration = Duration::from_millis(350);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(DIALOG_CONTEXT)),
        KeyBinding::new("down", NextItem, Some(CONTEXT)),
        KeyBinding::new("up", PreviousItem, Some(CONTEXT)),
        KeyBinding::new("right", OpenSubmenu, Some(CONTEXT)),
        KeyBinding::new("left", CloseSubmenu, Some(SUBMENU_CONTEXT)),
    ]);
}

/// Where a popover opens from the element it hangs from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opening {
    /// Under the element, from its leading edge, like a pop-up button's menu.
    Below,
    /// Under the element and centred on it, for an element in the middle of
    /// the window.
    BelowCentered,
    /// Under the element, from its trailing edge, for an element at the end
    /// of a row.
    BelowEnd,
    /// Over the element, like a menu from the bottom of a sidebar.
    Above,
    /// From a point in the window, such as where a row was right-clicked:
    /// a context menu.
    At(Point<Pixels>),
}

type DismissHandler = Rc<dyn Fn(&mut Window, &mut App)>;
type Build = Box<dyn FnOnce(Popover, &mut Window, &mut App) -> Popover>;

enum Child {
    Element(AnyElement),
    /// A row that opens more choices beside the menu.
    Submenu(MenuItem, Vec<AnyElement>),
}

#[derive(IntoElement)]
pub struct Popover {
    id: ElementId,
    open: bool,
    opening: Opening,
    focus: FocusHandle,
    label: SharedString,
    dialog: bool,
    children: Vec<Child>,
    /// What adds its items once it shows, from [`Popover::when_shown`].
    builds: Vec<Build>,
    on_dismiss: Option<DismissHandler>,
}

/// A menu that shows while `open`, and animates in and out. `label` names it
/// for assistive technologies.
pub fn popover(
    id: impl Into<ElementId>,
    open: bool,
    focus: &FocusHandle,
    label: impl Into<SharedString>,
) -> Popover {
    Popover {
        id: id.into(),
        open,
        opening: Opening::Below,
        focus: focus.clone(),
        label: label.into(),
        dialog: false,
        children: vec![],
        builds: vec![],
        on_dismiss: None,
    }
}

impl Popover {
    pub fn opening(mut self, opening: Opening) -> Self {
        self.opening = opening;
        self
    }

    pub fn on_dismiss(mut self, handler: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Some(Rc::new(handler));
        self
    }

    /// Holds controls rather than menu items, such as the generator: it is
    /// a dialog to assistive technologies, its controls keep the arrow keys,
    /// and it takes the width of its content.
    pub fn dialog(mut self) -> Self {
        self.dialog = true;
        self
    }

    /// Fills the popover with `build` while it shows, opening or closing,
    /// and not at all while it is closed: for a menu whose items take work
    /// to make, such as one row per folder or account, each with its click.
    /// `build` gets the popover and adds to it with [`child`], [`children`]
    /// and [`submenu`] as anywhere else; it runs when the popover is drawn,
    /// after its owner's `render`, so it reads what it needs from an entity
    /// rather than borrowing the owner:
    ///
    /// ```ignore
    /// let view = cx.entity();
    /// popover("folder-menu", self.menu_open, &self.menu_focus, "Folder")
    ///     .when_shown(move |menu, _, cx| {
    ///         view.update(cx, |view, cx| view.folder_items(menu, cx))
    ///     })
    /// ```
    ///
    /// Items added both ways keep their order: these come after those added
    /// before the popover was drawn.
    ///
    /// [`child`]: ParentElement::child
    /// [`children`]: ParentElement::children
    /// [`submenu`]: Popover::submenu
    pub fn when_shown(
        mut self,
        build: impl FnOnce(Self, &mut Window, &mut App) -> Self + 'static,
    ) -> Self {
        self.builds.push(Box::new(build));
        self
    }

    /// A row that opens `items` beside the menu, as a native submenu does:
    /// when the pointer rests on it, the submenu stays open while the
    /// pointer heads for it, and closes when another row is chosen; Right,
    /// Return or a click open it from the keyboard, Left closes it.
    pub fn submenu(
        mut self,
        item: MenuItem,
        items: impl IntoIterator<Item = impl IntoElement>,
    ) -> Self {
        let items = items.into_iter().map(IntoElement::into_any_element);
        self.children.push(Child::Submenu(item, items.collect()));
        self
    }
}

impl ParentElement for Popover {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children
            .extend(elements.into_iter().map(Child::Element));
    }
}

/// Which of a popover's submenus shows, and what the pointer did, kept
/// while the popover shows.
#[derive(Default)]
struct Submenus {
    /// The submenu shown, by its order among the popover's submenus.
    open: Option<usize>,
    /// The keyboard opened it: its first item takes the keyboard once it
    /// is drawn.
    focus_first: bool,
    /// Opens or closes a submenu once the pointer has rested.
    timer: Option<Task<()>>,
    /// Where the rows that open submenus, the menu and the open submenu
    /// were drawn last.
    rows: Vec<Bounds<Pixels>>,
    menu: Option<Bounds<Pixels>>,
    submenu: Option<Bounds<Pixels>>,
    pointer: Option<Point<Pixels>>,
}

impl Submenus {
    /// Shows submenu `index`, or none, at once.
    fn show(&mut self, index: Option<usize>, cx: &mut Context<Self>) {
        self.timer = None;
        if self.open != index {
            self.open = index;
            self.submenu = None;
            cx.notify();
        }
    }

    /// Shows submenu `index`, or none, once the pointer has rested for
    /// `delay`, unless it moves on meanwhile.
    fn show_after(
        &mut self,
        index: Option<usize>,
        delay: Duration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let state = cx.entity().downgrade();
        self.timer = Some(window.spawn(cx, async move |cx| {
            cx.background_executor().timer(delay).await;
            state.update(cx, |state, cx| state.show(index, cx)).ok();
        }));
    }

    /// Follows the pointer: the row it rests on opens its submenu, another
    /// closes it, and while the pointer heads for the open submenu, across
    /// other rows, the submenu waits.
    fn pointer_moved(&mut self, at: Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let from = self.pointer.replace(at);
        let heading = match (from, self.submenu) {
            (Some(from), Some(submenu)) => heads_for(from, at, submenu),
            _ => false,
        };
        let row = self.rows.iter().position(|row| row.contains(&at));
        if row.is_some() && row == self.open
            || self.submenu.is_some_and(|submenu| submenu.contains(&at))
        {
            self.timer = None;
        } else if heading {
            if self.timer.is_none() {
                self.show_after(row, SUBMENU_GRACE, window, cx);
            }
        } else if let Some(row) = row {
            self.show_after(Some(row), SUBMENU_DELAY, window, cx);
        } else if self.menu.is_some_and(|menu| menu.contains(&at)) {
            // Another row of the menu.
            self.show_after(None, SUBMENU_DELAY, window, cx);
        } else {
            // Outside both menus, the submenu stays, as a native one does.
            self.timer = None;
        }
    }
}

/// Whether the pointer, moving from `from` to `to`, heads for the
/// submenu: it is inside the triangle between where it was and the
/// submenu's near edge.
fn heads_for(from: Point<Pixels>, to: Point<Pixels>, submenu: Bounds<Pixels>) -> bool {
    let edge = if from.x <= submenu.left() {
        submenu.left()
    } else {
        submenu.right()
    };
    let (a, b, c) = (
        from,
        point(edge, submenu.top()),
        point(edge, submenu.bottom()),
    );
    let side = |p: Point<Pixels>, q: Point<Pixels>, r: Point<Pixels>| {
        (q.x - p.x).as_f32() * (r.y - p.y).as_f32() - (q.y - p.y).as_f32() * (r.x - p.x).as_f32()
    };
    let (d1, d2, d3) = (side(to, a, b), side(to, b, c), side(to, c, a));
    let negative = d1 < 0. || d2 < 0. || d3 < 0.;
    let positive = d1 > 0. || d2 > 0. || d3 > 0.;
    !(negative && positive)
}

/// Records an element's bounds as it is drawn.
fn measure(
    state: &Entity<Submenus>,
    record: impl Fn(&mut Submenus, Bounds<Pixels>) + 'static,
) -> impl IntoElement {
    let state = state.clone();
    canvas(
        move |bounds, _, cx| state.update(cx, |state, _| record(state, bounds)),
        |_, _, _, _| {},
    )
    .absolute()
    .inset_0()
}

impl RenderOnce for Popover {
    fn render(mut self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let presence = m::presence((self.id.clone(), "presence"), self.open, window, cx);
        if !presence.should_render() {
            return div().into_any_element();
        }
        // A build may add another.
        while !self.builds.is_empty() {
            for build in std::mem::take(&mut self.builds) {
                self = build(self, window, cx);
            }
        }
        let p = presence.progress;
        let gap = px(4.);
        let max_height = window.viewport_size().height - px(16.);
        // It moves in from a few points towards the control it belongs to.
        let shift = px(6.) * (1. - p);
        let mut window_position = None;
        let (anchor, place, offset) = match self.opening {
            Opening::Below => (
                gpui::Anchor::TopLeft,
                div().absolute().left_0().top(relative(1.)),
                point(px(0.), gap - shift),
            ),
            Opening::BelowCentered => (
                gpui::Anchor::TopCenter,
                div().absolute().left(relative(0.5)).top(relative(1.)),
                point(px(0.), gap - shift),
            ),
            Opening::BelowEnd => (
                gpui::Anchor::TopRight,
                div().absolute().right_0().top(relative(1.)),
                point(px(0.), gap - shift),
            ),
            Opening::Above => (
                gpui::Anchor::BottomLeft,
                div().absolute().left_0().top_0(),
                point(px(0.), shift - gap),
            ),
            Opening::At(position) => {
                window_position = Some(position);
                (
                    gpui::Anchor::TopLeft,
                    div().absolute().left_0().top_0(),
                    point(px(0.), gap - shift),
                )
            }
        };
        let submenus = window.use_keyed_state((self.id.clone(), "submenus"), cx, |_, _| {
            Submenus::default()
        });
        let count = self
            .children
            .iter()
            .filter(|child| matches!(child, Child::Submenu(..)))
            .count();
        let focus_of = |name: &'static str, index: usize, window: &mut Window, cx: &mut App| {
            window
                .use_keyed_state((self.id.clone(), format!("{name}-{index}")), cx, |_, cx| {
                    cx.focus_handle()
                })
                .read(cx)
                .clone()
        };
        let row_focus: Vec<FocusHandle> = (0..count)
            .map(|index| focus_of("submenu-row", index, window, cx))
            .collect();
        let submenu_focus: Vec<FocusHandle> = (0..count)
            .map(|index| focus_of("submenu", index, window, cx))
            .collect();
        let state = submenus.read(cx);
        let shown = state.open.filter(|_| self.open);
        let row_bounds = state.rows.clone();

        let dismiss = self.on_dismiss;
        let focus = self.focus.clone();
        let dialog = self.dialog;
        let mut shown_submenus = vec![];
        let mut index = 0;
        let mut rows = vec![];
        for child in self.children {
            match child {
                Child::Element(element) => rows.push(element),
                Child::Submenu(mut item, items) => {
                    let at = index;
                    index += 1;
                    let open = shown == Some(at);
                    let open_state = submenus.clone();
                    let opener = move |_: &mut Window, cx: &mut App| {
                        open_state.update(cx, |state, cx| {
                            state.show(Some(at), cx);
                            state.focus_first = true;
                        });
                    };
                    let opener = Rc::new(opener);
                    let (on_click, on_right) = (opener.clone(), opener);
                    item.submenu = true;
                    item.highlighted = open;
                    item.focus = Some(row_focus[at].clone());
                    item.on_click = Some(Box::new(move |_, window, cx| on_click(window, cx)));
                    rows.push(
                        div()
                            .relative()
                            .on_action(move |_: &OpenSubmenu, window, cx| on_right(window, cx))
                            .child(item)
                            .child(measure(&submenus, move |state, bounds| {
                                if state.rows.len() <= at {
                                    state.rows.resize(at + 1, Bounds::default());
                                }
                                state.rows[at] = bounds;
                            }))
                            .into_any_element(),
                    );
                    let appear =
                        m::presence((self.id.clone(), format!("submenu-{at}")), open, window, cx);
                    if let (true, Some(row)) = (appear.should_render(), row_bounds.get(at).copied())
                    {
                        // Once drawn, its items can take the keyboard.
                        if open && std::mem::take(&mut submenus.as_mut(cx).focus_first) {
                            let focus = submenu_focus[at].clone();
                            window.defer(cx, move |window, cx| {
                                window.focus(&focus, cx);
                                step(&focus, &[], window, cx, Window::focus_next);
                            });
                        }
                        let target = Submenu {
                            popover: self.id.clone(),
                            index: at,
                            state: submenus.clone(),
                            row_focus: row_focus[at].clone(),
                            focus: submenu_focus[at].clone(),
                        };
                        shown_submenus.push(target.render(
                            items,
                            row,
                            p * appear.progress,
                            dismiss.clone(),
                            window,
                            cx,
                        ));
                    }
                }
            }
        }

        let (outside, escape) = (dismiss.clone(), dismiss);
        let (next_focus, previous_focus) = (focus.clone(), focus.clone());
        let (skip_next, skip_previous) = (submenu_focus.clone(), submenu_focus);
        let (moved, clicked) = (submenus.clone(), submenus.clone());
        let menu = div()
            .id(self.id.clone())
            .role(if dialog {
                gpui::Role::Dialog
            } else {
                gpui::Role::Menu
            })
            .aria_label(self.label)
            .key_context(if dialog { DIALOG_CONTEXT } else { CONTEXT })
            .track_focus(&self.focus)
            .occlude()
            .opacity(p)
            .on_action(move |_: &Dismiss, window, cx| {
                if let Some(dismiss) = &escape {
                    dismiss(window, cx);
                }
            })
            .on_action(move |_: &NextItem, window, cx| {
                step(&next_focus, &skip_next, window, cx, Window::focus_next)
            })
            .on_action(move |_: &PreviousItem, window, cx| {
                step(
                    &previous_focus,
                    &skip_previous,
                    window,
                    cx,
                    Window::focus_prev,
                )
            })
            // A click outside only closes the menu, as a native menu does: it
            // doesn't also reach what is under the pointer.
            .on_mouse_down_out(move |event, window, cx| {
                let in_submenu = clicked
                    .read(cx)
                    .submenu
                    .is_some_and(|submenu| submenu.contains(&event.position));
                if in_submenu {
                    return;
                }
                cx.stop_propagation();
                if let Some(dismiss) = &outside {
                    dismiss(window, cx);
                }
            })
            // The pointer is followed over the submenu too, which is
            // outside the menu.
            .when(count > 0, |menu| {
                menu.child(
                    canvas(
                        |_, _, _| {},
                        move |_, _, window, _| {
                            let moved = moved.clone();
                            window.on_mouse_event(
                                move |event: &MouseMoveEvent, phase, window, cx| {
                                    if phase.bubble() {
                                        moved.update(cx, |state, cx| {
                                            state.pointer_moved(event.position, window, cx)
                                        });
                                    }
                                },
                            );
                        },
                    )
                    .absolute()
                    .size_0(),
                )
            })
            .child(
                surface()
                    .map(|surface| {
                        if dialog {
                            surface.p(px(12.)).rounded(px(14.))
                        } else {
                            surface
                                .min_w(px(200.))
                                .max_w(px(280.))
                                .p(px(5.))
                                .rounded(px(10.))
                        }
                    })
                    // A menu taller than the window scrolls.
                    .max_h(max_height)
                    .child(
                        scroll_area((self.id.clone(), "items"))
                            .min_h_0()
                            .flex()
                            .flex_col()
                            .children(rows),
                    )
                    .child(measure(&submenus, |state, bounds| {
                        state.menu = Some(bounds)
                    })),
            )
            .children(shown_submenus)
            .focus_trap((self.id, "trap"), &focus);
        place
            .child(
                deferred(
                    anchored()
                        .anchor(anchor)
                        .when_some(window_position, |a, position| {
                            a.position_mode(gpui::AnchoredPositionMode::Window)
                                .position(position)
                        })
                        .offset(offset)
                        .snap_to_window_with_margin(px(8.))
                        .child(menu),
                )
                .with_priority(1),
            )
            .into_any_element()
    }
}

/// A submenu of a popover, and what it needs to be drawn.
struct Submenu {
    popover: ElementId,
    index: usize,
    state: Entity<Submenus>,
    /// The row that opens it, which Left gives the keyboard back to.
    row_focus: FocusHandle,
    focus: FocusHandle,
}

impl Submenu {
    /// The submenu, beside its row: after it, or before it when the window
    /// has no room after it.
    fn render(
        self,
        items: Vec<AnyElement>,
        row: Bounds<Pixels>,
        progress: f32,
        dismiss: Option<DismissHandler>,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let width = self
            .state
            .read(cx)
            .submenu
            .map_or(px(200.), |bounds| bounds.size.width);
        // Its first item level with the row.
        let (gap, padding) = (px(2.), px(5.));
        let fits_after = row.right() + gap + width + px(8.) <= window.viewport_size().width;
        let (anchor, x) = if fits_after {
            (gpui::Anchor::TopLeft, row.right() + gap)
        } else {
            (gpui::Anchor::TopRight, row.left() - gap)
        };
        let max_height = window.viewport_size().height - px(16.);
        let (closing, row_focus) = (self.state.clone(), self.row_focus);
        let focus = self.focus;
        let (next_focus, previous_focus) = (focus.clone(), focus.clone());
        let escape = dismiss;
        deferred(
            anchored()
                .anchor(anchor)
                .position_mode(gpui::AnchoredPositionMode::Window)
                .position(point(x, row.top() - padding))
                .snap_to_window_with_margin(px(8.))
                .child(
                    div()
                        .id((self.popover.clone(), format!("submenu-{}", self.index)))
                        .role(gpui::Role::Menu)
                        .key_context(SUBMENU_CONTEXT)
                        .track_focus(&focus)
                        .occlude()
                        .opacity(progress)
                        .on_action(move |_: &CloseSubmenu, window, cx| {
                            closing.update(cx, |state, cx| state.show(None, cx));
                            window.focus(&row_focus, cx);
                        })
                        .on_action(|_: &OpenSubmenu, _, _| {})
                        .on_action(move |_: &NextItem, window, cx| {
                            step(&next_focus, &[], window, cx, Window::focus_next)
                        })
                        .on_action(move |_: &PreviousItem, window, cx| {
                            step(&previous_focus, &[], window, cx, Window::focus_prev)
                        })
                        .on_action(move |_: &Dismiss, window, cx| {
                            if let Some(dismiss) = &escape {
                                dismiss(window, cx);
                            }
                        })
                        .child(
                            surface()
                                .min_w(px(180.))
                                .max_w(px(280.))
                                .max_h(max_height)
                                .p(padding)
                                .rounded(px(10.))
                                .child(
                                    scroll_area((
                                        self.popover.clone(),
                                        format!("submenu-items-{}", self.index),
                                    ))
                                    .min_h_0()
                                    .flex()
                                    .flex_col()
                                    .children(items),
                                )
                                .child(measure(&self.state, |state, bounds| {
                                    state.submenu = Some(bounds)
                                })),
                        ),
                ),
        )
        .with_priority(2)
        .into_any_element()
    }
}

/// Moves the focus to the next or previous item, round inside the menu,
/// past the items of its submenus.
fn step(
    menu: &FocusHandle,
    skip: &[FocusHandle],
    window: &mut Window,
    cx: &mut App,
    advance: fn(&mut Window, &mut App),
) {
    let mut visited = vec![];
    loop {
        advance(window, cx);
        let focused = window.focused(cx);
        if menu.contains_focused(window, cx)
            && focused.as_ref() != Some(menu)
            && !skip.iter().any(|skip| skip.contains_focused(window, cx))
        {
            return;
        }
        if visited.contains(&focused) {
            return;
        }
        visited.push(focused);
    }
}

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// One choice in a popover menu: an optional leading icon or avatar, a
/// label with an optional second line, and a tick when it is the current
/// choice.
#[derive(IntoElement)]
pub struct MenuItem {
    id: ElementId,
    label: SharedString,
    detail: Option<SharedString>,
    leading: Option<AnyElement>,
    checked: bool,
    submenu: bool,
    /// Its submenu is open: it keeps the hover's fill.
    highlighted: bool,
    focus: Option<FocusHandle>,
    on_click: Option<ClickHandler>,
}

pub fn menu_item(id: impl Into<ElementId>, label: impl Into<SharedString>) -> MenuItem {
    MenuItem {
        id: id.into(),
        label: label.into(),
        detail: None,
        leading: None,
        checked: false,
        submenu: false,
        highlighted: false,
        focus: None,
        on_click: None,
    }
}

impl MenuItem {
    pub fn icon(self, name: IconName) -> Self {
        self.leading(icon(name))
    }

    pub fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading = Some(element.into_any_element());
        self
    }

    /// A second, quieter line under the label.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
}

impl RenderOnce for MenuItem {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let (hover, focused, dim) = (colors.hover, colors.fill, colors.text_secondary);
        let accessible = match &self.detail {
            Some(detail) => format!("{}, {detail}", self.label).into(),
            None => self.label.clone(),
        };
        BaseButton::new(self.id)
            .role(gpui::Role::MenuItem)
            .accessibility_label(accessible)
            .when_some(self.on_click, |b, handler| b.on_click(handler))
            .when_some(self.focus, |b, focus| b.track_focus(&focus))
            .when(self.highlighted, |b| b.bg(hover))
            .w_full()
            .justify_start()
            .gap(px(8.))
            .min_h(px(28.))
            .px(px(8.))
            .py(px(4.))
            .rounded(px(6.))
            .line_height(relative(1.3))
            .hover(move |s| s.bg(hover))
            .focus_visible(move |s| s.bg(focused))
            .children(self.leading)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().truncate().child(self.label))
                    .when_some(self.detail, |d, detail| {
                        d.child(
                            div()
                                .truncate()
                                .text_size(theme.metrics.small_text_size)
                                .text_color(dim)
                                .child(detail),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(15.))
                    .flex_none()
                    .font_weight(FontWeight::SEMIBOLD)
                    .when(self.checked, |d| d.child(icon(IconName::Check)))
                    .when(self.submenu, |d| {
                        d.text_color(dim).child(icon(IconName::ChevronRight))
                    }),
            )
    }
}

/// A hairline between groups of menu items.
pub fn menu_separator(cx: &App) -> impl IntoElement {
    div()
        .h(px(0.5))
        .mx(px(8.))
        .my(px(4.))
        .bg(cx.theme().colors.separator)
}
