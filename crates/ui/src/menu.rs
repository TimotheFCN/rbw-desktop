//! Popover menus hanging from a control, such as the account switcher or the
//! editor's folder menu.
//!
//! The owner keeps whether the menu is open and a focus handle for it, puts
//! [`popover`] inside the element the menu hangs from, and focuses the handle
//! when it opens. A click outside, Escape, or choosing an item closes it
//! through [`Popover::on_dismiss`]. Tab and the arrow keys move between the
//! items without leaving the menu. A [`Popover::dialog`] holds controls
//! instead, such as the generator's options.

use std::rc::Rc;

use gpui::base::{Button as BaseButton, FocusTrapElement as _};
use gpui::{
    AnyElement, App, ClickEvent, ElementId, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Pixels, Point, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, actions, anchored, deferred, div, point,
    prelude::FluentBuilder as _, px, relative,
};
use smallvec::SmallVec;

use crate::icon::{IconName, icon};
use crate::motion as m;
use crate::scroll::scroll_area;
use crate::sheet::surface;
use crate::theme::ActiveTheme;

actions!(popover, [Dismiss, NextItem, PreviousItem]);

const CONTEXT: &str = "Popover";
const DIALOG_CONTEXT: &str = "PopoverDialog";

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(DIALOG_CONTEXT)),
        KeyBinding::new("down", NextItem, Some(CONTEXT)),
        KeyBinding::new("up", PreviousItem, Some(CONTEXT)),
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

#[derive(IntoElement)]
pub struct Popover {
    id: ElementId,
    open: bool,
    opening: Opening,
    focus: FocusHandle,
    label: SharedString,
    dialog: bool,
    children: SmallVec<[AnyElement; 6]>,
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
        children: SmallVec::new(),
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
}

impl ParentElement for Popover {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Popover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let presence = m::presence((self.id.clone(), "presence"), self.open, window, cx);
        if !presence.should_render() {
            return div().into_any_element();
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
        let dismiss = self.on_dismiss;
        let (outside, escape) = (dismiss.clone(), dismiss);
        let focus = self.focus.clone();
        let (next_focus, previous_focus) = (focus.clone(), focus.clone());
        let dialog = self.dialog;
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
                step(&next_focus, window, cx, Window::focus_next)
            })
            .on_action(move |_: &PreviousItem, window, cx| {
                step(&previous_focus, window, cx, Window::focus_prev)
            })
            // A click outside only closes the menu, as a native menu does: it
            // doesn't also reach what is under the pointer.
            .on_mouse_down_out(move |_, window, cx| {
                cx.stop_propagation();
                if let Some(dismiss) = &outside {
                    dismiss(window, cx);
                }
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
                            .children(self.children),
                    ),
            )
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

/// Moves the focus to the next or previous item, round inside the menu.
fn step(menu: &FocusHandle, window: &mut Window, cx: &mut App, advance: fn(&mut Window, &mut App)) {
    let mut visited = vec![];
    loop {
        advance(window, cx);
        let focused = window.focused(cx);
        if menu.contains_focused(window, cx) && focused.as_ref() != Some(menu) {
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

    /// Marks an item that shows more choices, with a chevron at its end.
    pub fn submenu(mut self) -> Self {
        self.submenu = true;
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
