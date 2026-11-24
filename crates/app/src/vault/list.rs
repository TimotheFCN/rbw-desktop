//! The item list with its search field.

use gpui::{
    AnyElement, Context, InteractiveElement as _, IntoElement, MouseButton, MouseDownEvent,
    ParentElement as _, Styled as _, Window, div, px, uniform_list,
};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Look, button, chrome, list_row, search_field,
};

use super::{NewItem, VaultView};
use crate::backend::{self, Load};
use crate::icons;

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
                .child(
                    button("new-item")
                        .icon(IconName::Plus)
                        .style(add_style)
                        .accessibility_label("New Item")
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
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| {
                    let account = this.account.id.clone();
                    range
                        .map(|position| {
                            let items = this.items.ready().map(Vec::as_slice).unwrap_or_default();
                            let item = items[this.visible[position]].clone();
                            let selected = this.selected.as_ref() == Some(&item.id);
                            let icon = icons::icon(&account, &item, cx);
                            let (id, menu_id) = (item.id.clone(), item.id.clone());
                            div()
                                .px(px(7.))
                                .pb(px(1.))
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

        div()
            .id("list")
            .key_context("VaultList")
            .track_focus(&self.list_focus)
            .flex_none()
            .w(metrics.list_width)
            .h_full()
            .flex()
            .flex_col()
            .bg(fill)
            .border_r(separator_width)
            .border_color(separator)
            .child(toolbar)
            .child(body)
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
