//! A pop-up button: the current choice, which opens a menu of the others,
//! such as how long the vault stays unlocked.
//!
//! [`Select`] is a view that keeps its choices, whether its menu is open and
//! where the keyboard goes; the owner creates it with [`Select::new`],
//! renders the entity, and hears of a new choice through [`Selected`].

use gpui::base::Button as BaseButton;
use gpui::{
    BoxShadow, Context, EventEmitter, FocusHandle, InteractiveElement, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};

use crate::icon::{IconName, icon};
use crate::menu::{menu_item, popover};
use crate::theme::{ActiveTheme, Look};

/// The index of the choice just made.
pub struct Selected(pub usize);

pub struct Select {
    /// Names the control for assistive technologies.
    label: SharedString,
    options: Vec<SharedString>,
    selected: usize,
    open: bool,
    button: FocusHandle,
    menu: FocusHandle,
}

impl EventEmitter<Selected> for Select {}

impl Select {
    pub fn new(
        label: impl Into<SharedString>,
        options: impl IntoIterator<Item = impl Into<SharedString>>,
        selected: usize,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            label: label.into(),
            options: options.into_iter().map(Into::into).collect(),
            selected,
            open: false,
            button: cx.focus_handle(),
            menu: cx.focus_handle(),
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// Shows another choice, without telling the owner.
    pub fn set_selected(&mut self, selected: usize, cx: &mut Context<Self>) {
        if self.selected != selected {
            self.selected = selected;
            cx.notify();
        }
    }

    pub fn set_options(
        &mut self,
        options: impl IntoIterator<Item = impl Into<SharedString>>,
        selected: usize,
        cx: &mut Context<Self>,
    ) {
        self.options = options.into_iter().map(Into::into).collect();
        self.selected = selected;
        cx.notify();
    }

    fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.open {
            self.close(window, cx);
        } else {
            self.open = true;
            window.focus(&self.menu, cx);
            cx.notify();
        }
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        window.focus(&self.button, cx);
        cx.notify();
    }

    fn choose(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.close(window, cx);
        if index != self.selected {
            self.selected = index;
            cx.emit(Selected(index));
        }
    }
}

impl Render for Select {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let macos = theme.look == Look::MacOs;
        let (fill, hover, pressed, ring) = (
            colors.control,
            colors.control.blend(colors.hover),
            colors.control.blend(colors.fill),
            colors.focus_ring,
        );
        let edge = colors.control_edge;
        let highlight = colors.control_highlight;
        let value = self.options.get(self.selected).cloned().unwrap_or_default();
        let this = cx.entity().downgrade();
        let menu = popover("select-menu", self.open, &self.menu, self.label.clone())
            .on_dismiss(move |window, cx| {
                this.update(cx, |this, cx| this.close(window, cx)).ok();
            })
            .children(self.options.iter().enumerate().map(|(index, option)| {
                menu_item(("option", index), option.clone())
                    .checked(index == self.selected)
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.choose(index, window, cx)),
                    )
            }));
        div().relative().flex_none().child(
            BaseButton::new("select")
                .track_focus(&self.button)
                .role(gpui::Role::ComboBox)
                .accessibility_label(format!("{}, {value}", self.label))
                .on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx)))
                .flex()
                .items_center()
                .gap(px(6.))
                .h(if macos { px(24.) } else { px(34.) })
                .pl(px(if macos { 9. } else { 12. }))
                .pr(px(if macos { 5. } else { 9. }))
                .rounded(if macos {
                    px(6.)
                } else {
                    theme.metrics.control_radius
                })
                .bg(fill)
                .hover(move |s| s.bg(hover))
                .active(move |s| s.bg(pressed))
                .when(macos, |b| {
                    b.border(px(0.5)).border_color(edge).shadow(vec![
                        BoxShadow {
                            color: highlight,
                            offset: gpui::point(px(0.), px(1.)),
                            blur_radius: px(0.),
                            spread_radius: px(0.),
                            inset: true,
                        },
                        BoxShadow::new(px(0.), px(1.), gpui::black().opacity(0.11))
                            .blur_radius(px(3.)),
                    ])
                })
                .focus_visible(move |s| {
                    s.shadow(vec![
                        BoxShadow::new(px(0.), px(0.), ring).spread_radius(px(3.)),
                    ])
                })
                .child(value)
                .child(
                    icon(if macos {
                        IconName::ChevronsUpDown
                    } else {
                        IconName::Down
                    })
                    .size(px(13.)),
                )
                .child(menu),
        )
    }
}
