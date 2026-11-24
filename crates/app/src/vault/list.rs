//! The item list with its search field.

use gpui::{
    AnyElement, AppContext as _, ClickEvent, Context, CursorStyle, ElementId, EmptyView,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Pixels,
    StatefulInteractiveElement as _, Styled as _, Window, deferred, div,
    prelude::FluentBuilder as _, px, relative, uniform_list,
};
use rbw_desktop_core::SortOrder;
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Look, Opening, button, chrome, list_row, menu_item,
    menu_separator, motion, popover, scrollbar, search_field, section_label,
};

use super::{ListDivider, MAX_LIST_SHARE, NewItem, Only, ShowGenerator, VaultView};
use crate::backend::{self, Load};
use crate::generator;
use crate::icons;
use crate::preferences::Prefs;
use crate::secondary;

/// The space under each row of the list.
pub(super) const ROW_GAP: Pixels = px(1.);
/// The narrowest the list gets, however small the window or its share.
pub(super) const MIN_WIDTH: Pixels = px(220.);
/// How far either side of the list's edge its divider takes the pointer.
const DIVIDER_REACH: Pixels = px(3.);

impl VaultView {
    pub(super) fn render_list(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let metrics = theme.metrics;
        let add_style = match theme.look {
            Look::MacOs => ButtonStyle::Normal,
            Look::Gnome => ButtonStyle::Flat,
        };
        let (fill, separator, dim) = (colors.list, colors.separator, colors.text_secondary);
        let separator_width = px(if theme.look == Look::Gnome { 1. } else { 0.5 });

        let toolbar = chrome::drag_region(
            "list-toolbar",
            div()
                .h(metrics.toolbar_height)
                .flex_none()
                .flex()
                .items_center()
                .gap(px(8.))
                .px(px(12.))
                .child(search_field(&self.search))
                .child(self.render_sort_button(add_style, cx))
                .child(self.render_generator_button(add_style, cx))
                .child(
                    button("new-item")
                        .icon(IconName::Plus)
                        .style(add_style)
                        .accessibility_label("New Item")
                        .shortcut(secondary!("n"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.new_item(&NewItem, window, cx)),
                        ),
                ),
            window,
            cx,
        );

        let body = match &self.items {
            Load::Failed(error) => message(backend::describe(error), dim),
            Load::Ready(items) if items.is_empty() => message("No Items", dim),
            Load::Ready(_) if self.visible.is_empty() => message("No Results", dim),
            Load::Loading | Load::Ready(_) => uniform_list(
                "items",
                self.visible.len(),
                cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                    let account = this.account.id.clone();
                    let list_focused = this.list_focus.is_focused(window);
                    range
                        .map(|position| {
                            let items = this.items.ready().map(Vec::as_slice).unwrap_or_default();
                            let item = items[this.visible[position]].clone();
                            let selected = this.selected.as_ref() == Some(&item.id);
                            let copied = this.copied.as_ref() == Some(&item.id);
                            let icon = icons::icon(&account, &item, cx);
                            let (id, menu_id) = (item.id.clone(), item.id.clone());
                            div()
                                .px(px(7.))
                                .pb(ROW_GAP)
                                .on_mouse_down(
                                    MouseButton::Right,
                                    cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                                        this.open_context_menu(
                                            menu_id.clone(),
                                            event.position,
                                            window,
                                            cx,
                                        );
                                    }),
                                )
                                .child(
                                    list_row(
                                        gpui::ElementId::Name(format!("item-{}", item.id.0).into()),
                                        item.name.clone(),
                                        item.subtitle.clone().map(Into::into),
                                        selected,
                                    )
                                    .icon(icon)
                                    .copied(copied)
                                    .in_focusable_list(list_focused)
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            window.focus(&this.list_focus, cx);
                                            this.select_by(Some(id.clone()), true, cx);
                                        },
                                    )),
                                )
                        })
                        .collect()
                }),
            )
            .track_scroll(&self.list_scroll)
            .flex_1()
            .into_any_element(),
        };
        let bar = scrollbar("items-bar", &self.list_scroll, cx);
        // Another choice in the sidebar shows other items: they fade up in
        // place. Typing a search changes them without it.
        let body = motion::refresh(
            ElementId::Name(format!("items-{:?}", self.filter).into()),
            div()
                .flex_1()
                .min_h_0()
                .flex()
                .flex_col()
                .child(body)
                .child(bar),
        );

        // The list takes its share of the room, between its minimum and its
        // maximum; a small window shrinks it before the item.
        div()
            .id("list")
            .key_context("VaultList")
            .track_focus(&self.list_focus)
            .relative()
            .w(relative(self.list_share(cx)))
            .min_w(MIN_WIDTH)
            .max_w(relative(MAX_LIST_SHARE))
            .h_full()
            .flex()
            .flex_col()
            .bg(fill)
            .border_r(separator_width)
            .border_color(separator)
            .child(toolbar)
            .children(self.render_only(cx))
            .child(body)
            .child(self.render_divider(cx))
            .into_any_element()
    }
}

impl VaultView {
    /// The divider between the list and the item: dragged, it moves the
    /// list's edge; a double-click puts it back. It straddles the edge,
    /// drawn last, so that neither the item nor the list's scroll bar
    /// takes the pointer from it.
    fn render_divider(&self, cx: &mut Context<Self>) -> AnyElement {
        deferred(
            div()
                .id("list-divider")
                .absolute()
                .top_0()
                .bottom_0()
                .right(-DIVIDER_REACH)
                .w(DIVIDER_REACH * 2.)
                .cursor(CursorStyle::ResizeLeftRight)
                .block_mouse_except_scroll()
                .on_drag(ListDivider, |_, _, _, cx| cx.new(|_| EmptyView))
                .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                    if event.click_count() == 2 {
                        this.reset_list_share(cx);
                    }
                })),
        )
        .into_any_element()
    }

    /// The menu next to the search field: how the list is sorted, and what
    /// it shows only. Its choices for the list's contents leave out what no
    /// item has.
    fn render_sort_button(&self, style: ButtonStyle, cx: &mut Context<Self>) -> AnyElement {
        let order = Prefs::global(cx).read(cx).get().sort;
        let only = self.only;
        let items = self.items.ready().map(Vec::as_slice).unwrap_or_default();
        let has_codes = items.iter().any(|item| item.has_code);
        let has_shared = items.iter().any(|item| item.organization.is_some());
        let this = cx.entity().downgrade();
        let sort =
            |id: &'static str, label: &'static str, choice: SortOrder, cx: &mut Context<Self>| {
                menu_item(id, label)
                    .checked(order == choice)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.close_sort_menu(window, cx);
                        this.set_sort(choice, cx);
                    }))
            };
        let show = |id: &'static str,
                    label: &'static str,
                    on: bool,
                    change: fn(&mut Only),
                    cx: &mut Context<Self>| {
            menu_item(id, label)
                .checked(on)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.close_sort_menu(window, cx);
                    let mut only = this.only;
                    change(&mut only);
                    this.set_only(only, cx);
                }))
        };
        let menu = popover(
            "sort-menu",
            self.sort_menu_open,
            &self.sort_menu_focus,
            "Sort and Filter",
        )
        .opening(Opening::BelowEnd)
        .on_dismiss(move |window, cx| {
            this.update(cx, |this, cx| this.close_sort_menu(window, cx))
                .ok();
        })
        .child(section_label("Sort By", cx))
        .child(sort("sort-name", "Name", SortOrder::Name, cx))
        .child(sort(
            "sort-modified",
            "Last Modified",
            SortOrder::Modified,
            cx,
        ))
        .child(sort(
            "sort-recent",
            "Recently Used",
            SortOrder::RecentlyUsed,
            cx,
        ))
        .when(has_codes || has_shared || only.any(), |menu| {
            menu.child(menu_separator(cx))
                .child(section_label("Show Only", cx))
        })
        .when(has_codes || only.code, |menu| {
            menu.child(show(
                "only-code",
                "With Verification Code",
                only.code,
                |only| only.code = !only.code,
                cx,
            ))
        })
        .when(has_shared || only.shared, |menu| {
            menu.child(show(
                "only-shared",
                "Shared",
                only.shared,
                |only| only.shared = !only.shared,
                cx,
            ))
        });
        div()
            .relative()
            .child(
                button("sort-menu-button")
                    .icon(IconName::Filter)
                    .style(style)
                    .accessibility_label("Sort and Filter")
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.sort_menu_open {
                            this.close_sort_menu(window, cx);
                        } else {
                            this.sort_menu_open = true;
                            window.focus(&this.sort_menu_focus, cx);
                            cx.notify();
                        }
                    })),
            )
            .child(menu)
            .into_any_element()
    }

    fn close_sort_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sort_menu_open = false;
        window.focus(&self.list_focus, cx);
        cx.notify();
    }

    /// What the list shows only, under its toolbar while it narrows it,
    /// each with a button that shows everything again.
    fn render_only(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let only = self.only;
        if !only.any() {
            return None;
        }
        let colors = &cx.theme().colors;
        let (fill, dim) = (colors.fill, colors.text_secondary);
        let chip = |id: &'static str,
                    label: &'static str,
                    clear: fn(&mut Only),
                    cx: &mut Context<Self>| {
            div()
                .flex()
                .items_center()
                .gap(px(2.))
                .h(px(24.))
                .pl(px(10.))
                .pr(px(3.))
                .rounded_full()
                .bg(fill)
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .child(label)
                .child(
                    div().text_color(dim).child(
                        button(id)
                            .icon(IconName::Close)
                            .style(ButtonStyle::Flat)
                            .size(px(18.))
                            .accessibility_label(format!("Stop Showing Only {label}"))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let mut only = this.only;
                                clear(&mut only);
                                this.set_only(only, cx);
                            })),
                    ),
                )
        };
        Some(
            div()
                .flex_none()
                .flex()
                .flex_wrap()
                .gap(px(6.))
                .px(px(12.))
                .pb(px(8.))
                .when(only.code, |d| {
                    d.child(chip(
                        "clear-only-code",
                        "With Verification Code",
                        |only| only.code = false,
                        cx,
                    ))
                })
                .when(only.shared, |d| {
                    d.child(chip(
                        "clear-only-shared",
                        "Shared",
                        |only| only.shared = false,
                        cx,
                    ))
                })
                .into_any_element(),
        )
    }

    /// The generator's button, with its popover hanging from it.
    fn render_generator_button(&self, style: ButtonStyle, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        let popover = popover(
            "generator-popover",
            self.generator_open,
            &self.generator_focus,
            "Password Generator",
        )
        .dialog()
        .opening(Opening::Below)
        .on_dismiss(move |window, cx| {
            this.update(cx, |this, cx| this.close_generator(window, cx))
                .ok();
        })
        .children(
            self.generator
                .clone()
                .map(|generator| div().w(generator::POPOVER_WIDTH).child(generator)),
        );
        div()
            .relative()
            .child(
                button("generator")
                    .icon(IconName::Dice)
                    .style(style)
                    .accessibility_label("Password Generator")
                    .shortcut(secondary!("shift-g"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_generator(&ShowGenerator, window, cx)
                    })),
            )
            .child(popover)
            .into_any_element()
    }
}

fn message(text: &'static str, color: gpui::Hsla) -> AnyElement {
    div()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .px(px(16.))
        .text_color(color)
        .child(text)
        .into_any_element()
}
