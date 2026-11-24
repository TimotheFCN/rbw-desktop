//! The item list with its search field.

use std::ops::Range;
use std::time::Duration;

use gpui::{
    AnyElement, AppContext as _, ClickEvent, Context, CursorStyle, ElementId, EmptyView,
    InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent, ParentElement as _, Pixels,
    StatefulInteractiveElement as _, Styled, Window, div, prelude::FluentBuilder as _, px,
    relative, uniform_list,
};
use rbw_desktop_core::SortOrder;
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, Highlight, IconName, Look, Opening, ROW_HEIGHT, button, chrome,
    list_row, menu_item, menu_separator, motion, popover, scrollbar, search_field, section_label,
};

use super::{ListDivider, MAX_LIST_SHARE, NewItem, Only, ShowGenerator, VaultView};
use crate::backend::{self, Load};
use crate::generator;
use crate::icons::{self, Icons};
use crate::preferences::Prefs;
use crate::secondary;

/// The space under each row of the list.
pub(super) const ROW_GAP: Pixels = px(1.);
/// The narrowest the list gets, however small the window or its share.
pub(super) const MIN_WIDTH: Pixels = px(240.);
/// How many screens of rows either side of those drawn have their icons
/// read ahead.
const PREFETCH_SCREENS: usize = 2;
/// How long the pointer rests on a row before its item is fetched ahead.
const POINTER_REST: Duration = Duration::from_millis(60);
/// How far either side of the list's edge its divider takes the pointer.
const DIVIDER_REACH: Pixels = px(3.);

impl VaultView {
    /// The list, taking `share` of the room right of the sidebar.
    pub(super) fn render_list(
        &mut self,
        share: f32,
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

        let jump = std::mem::take(&mut self.highlight_jumps);
        self.highlight_at = self.selected_index().map(|index| {
            let on_screen = motion::rows_on_screen(&self.drew, index);
            motion::slide("list-selection", index as f32, jump, on_screen, window, cx)
        });
        let body = match &self.items {
            Load::Failed(error) => message(backend::describe(error), dim),
            Load::Ready(items) if items.is_empty() => message("No Items", dim),
            Load::Ready(_) if self.visible.is_empty() => message("No Results", dim),
            Load::Loading | Load::Ready(_) => uniform_list(
                "items",
                self.visible.len(),
                cx.processor(|this, range: std::ops::Range<usize>, window, cx| {
                    let account = this.account.id.clone();
                    this.drew = range.clone();
                    this.prefetch_icons(&range, cx);
                    let list_focused = this.list_focus.is_focused(window);
                    range
                        .map(|position| {
                            let items = this.items.ready().map(Vec::as_slice).unwrap_or_default();
                            let item = items[this.visible[position]].clone();
                            let selected = this.selected.as_ref() == Some(&item.id);
                            let highlight = this.highlight_at.and_then(|at| {
                                Highlight::at(at - position as f32, ROW_HEIGHT + ROW_GAP)
                            });
                            let copied = this.copied.as_ref() == Some(&item.id);
                            let icon = icons::icon(&account, &item, cx);
                            let (id, menu_id) = (item.id.clone(), item.id.clone());
                            let arrived = this.arrived.contains(&item.id);
                            let row = div()
                                .id(ElementId::Name(format!("row-{}", item.id.0).into()))
                                .px(px(7.))
                                .pb(ROW_GAP)
                                .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                    if *hovered {
                                        this.pointer_rests(position, cx);
                                    }
                                }))
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
                                    .highlight(highlight)
                                    .copied(copied)
                                    .in_focusable_list(list_focused)
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            window.focus(&this.list_focus, cx);
                                            this.select_by(Some(id.clone()), true, cx);
                                        },
                                    )),
                                );
                            if arrived {
                                motion::appear(
                                    ElementId::Name(format!("arrived-{}", item.id.0).into()),
                                    row,
                                )
                                .into_any_element()
                            } else {
                                row.into_any_element()
                            }
                        })
                        .collect()
                }),
            )
            .track_scroll(&self.list_scroll)
            .flex_1()
            .into_any_element(),
        };
        let bar = scrollbar("items-bar", &self.list_scroll, cx);
        // Another choice in the sidebar or the menu shows other items at
        // once, their icons with them: a fade over the whole list would
        // read as a flicker.
        let body = div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(body)
            .child(bar);

        let only = self.render_only(window, cx);
        list_width(share, div().id("list"))
            .key_context("VaultList")
            .track_focus(&self.list_focus)
            .relative()
            .h_full()
            .flex()
            .flex_col()
            .bg(fill)
            .border_r(separator_width)
            .border_color(separator)
            .child(toolbar)
            .children(only)
            .child(body)
            .into_any_element()
    }
}

impl VaultView {
    /// Reads the icons of the rows a few screens either side of those
    /// drawn from rbw's cache, so that scrolling finds them there.
    fn prefetch_icons(&mut self, drawn: &Range<usize>, cx: &mut Context<Self>) {
        let ahead = drawn.len() * PREFETCH_SCREENS;
        let around = drawn.start.saturating_sub(ahead)..(drawn.end + ahead).min(self.visible.len());
        if self.prefetched == around {
            return;
        }
        self.prefetched = around.clone();
        let items = self.items.ready().map(Vec::as_slice).unwrap_or_default();
        let rows = self.visible[around].iter().map(|&index| &items[index]);
        Icons::global(cx).update(cx, |icons, cx| icons.prefetch(&self.account.id, rows, cx));
    }

    /// The pointer came onto a row: once it rests there, the row's item is
    /// fetched ahead, so that a click shows it complete at once. Rows only
    /// passed over aren't.
    fn pointer_rests(&mut self, row: usize, cx: &mut Context<Self>) {
        self.hover_task = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(POINTER_REST).await;
            this.update(cx, |this, cx| this.prefetch_rows([row], cx))
                .ok();
        });
    }

    /// The divider between the list and the item: dragged, it moves the
    /// list's edge; a double-click puts it back. It is drawn over both
    /// panes, after them, so that neither the item nor the list's scroll
    /// bar takes the pointer from it, and under anything that opens over
    /// the window, such as a sheet or a menu.
    pub(super) fn render_divider(&self, share: f32, cx: &mut Context<Self>) -> AnyElement {
        // A line that shows it can be moved, under the pointer and while
        // it is dragged.
        let line = cx.theme().colors.text_tertiary.opacity(0.45);
        let dragged = self.dragged_share.is_some();
        let strip = div()
            .id("list-divider")
            .group("list-divider")
            .flex_none()
            .ml(-DIVIDER_REACH)
            .w(DIVIDER_REACH * 2.)
            .h_full()
            .flex()
            .justify_center()
            .child(
                div()
                    .w(px(2.))
                    .h_full()
                    .when(dragged, |d| d.bg(line))
                    .group_hover("list-divider", move |s| s.bg(line)),
            )
            .cursor(CursorStyle::ResizeLeftRight)
            .block_mouse_except_scroll()
            .on_drag(ListDivider, |_, _, _, cx| cx.new(|_| EmptyView))
            .on_click(cx.listener(|this, event: &ClickEvent, _, cx| {
                if event.click_count() == 2 {
                    this.reset_list_share(cx);
                }
            }));
        // Room as wide as the list, which takes no pointer, then the strip
        // across its edge.
        div()
            .absolute()
            .inset_0()
            .flex()
            .child(list_width(share, div().flex_none()))
            .child(strip)
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
                    // The list shows only some items.
                    .toggled(only.any())
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
    /// each with a button that shows everything again. The line opens and
    /// closes, moving the list down and up with it.
    fn render_only(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.only.any() {
            self.only_shown = self.only;
        }
        let only = self.only_shown;
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
        let chips = div()
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
            });
        motion::reveal("only", self.only.any(), chips, window, cx)
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

/// The list's width: its share of the panes' row, between its minimum
/// and its maximum, so that a small window shrinks it before the item.
fn list_width<E: Styled>(share: f32, element: E) -> E {
    element
        .w(relative(share))
        .min_w(MIN_WIDTH)
        .max_w(relative(MAX_LIST_SHARE))
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
