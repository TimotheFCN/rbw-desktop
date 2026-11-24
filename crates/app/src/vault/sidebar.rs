//! The sidebar: what to show, and the account at the bottom.

use gpui::base::Button as BaseButton;
use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use rbw_desktop_core::{APP_NAME, ItemKind};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Look, Material, Opening, avatar, button, chrome,
    glass, glass_surround, menu_item, menu_separator, nav_row, popover, row_focus_ring,
    section_label,
};

use super::{Filter, Lock, ShowGenerator, VaultView};
use crate::accounts;
use crate::settings::{About, OpenSettings};

const KINDS: [(ItemKind, IconName, &str); 5] = [
    (ItemKind::Login, IconName::Key, "Logins"),
    (ItemKind::Card, IconName::Card, "Cards"),
    (ItemKind::Identity, IconName::Identity, "Identities"),
    (ItemKind::SecureNote, IconName::Note, "Notes"),
    (ItemKind::SshKey, IconName::Terminal, "SSH Keys"),
];

impl VaultView {
    /// GNOME's primary menu: the generator, Preferences and About, which a
    /// Mac has in its menu bar.
    fn render_main_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity().downgrade();
        let menu = popover(
            "main-menu",
            self.main_menu_open,
            &self.main_menu_focus,
            "Main Menu",
        )
        .opening(Opening::Below)
        .on_dismiss(move |window, cx| {
            this.update(cx, |this, cx| this.close_main_menu(window, cx))
                .ok();
        })
        .child(
            menu_item("generator", "Password Generator")
                .on_click(Self::main_menu_choice(Box::new(ShowGenerator), cx)),
        )
        .child(menu_separator(cx))
        .child(
            menu_item("preferences", "Preferences")
                .on_click(Self::main_menu_choice(Box::new(OpenSettings), cx)),
        )
        .child(
            menu_item("about", format!("About {APP_NAME}"))
                .on_click(Self::main_menu_choice(Box::new(About), cx)),
        );
        div()
            .relative()
            .pr(px(4.))
            .child(
                button("main-menu-button")
                    .icon(IconName::Menu)
                    .style(ButtonStyle::Flat)
                    .accessibility_label("Main Menu")
                    .on_click(cx.listener(|this, _, window, cx| {
                        if this.main_menu_open {
                            this.close_main_menu(window, cx);
                        } else {
                            this.main_menu_open = true;
                            window.focus(&this.main_menu_focus, cx);
                            cx.notify();
                        }
                    })),
            )
            .child(menu)
            .into_any_element()
    }

    /// A main menu item's click: closes the menu, then does `action`.
    fn main_menu_choice(
        action: Box<dyn gpui::Action>,
        cx: &mut Context<Self>,
    ) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static {
        cx.listener(move |this: &mut Self, _, window, cx| {
            this.close_main_menu(window, cx);
            window.dispatch_action(action.boxed_clone(), cx);
        })
    }

    fn close_main_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.main_menu_open = false;
        window.focus(&self.list_focus, cx);
        cx.notify();
    }

    pub(super) fn render_sidebar(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let metrics = theme.metrics;
        let look = theme.look;
        let window_fill = theme.colors.window;
        let corner = chrome::corner_radius(window, cx);
        // A flush sidebar meets the window's rounded corners; a floating one
        // keeps its own.
        let glass_radius = if metrics.sidebar_inset > px(0.) {
            metrics.sidebar_radius
        } else {
            corner
        };
        // Only the kinds the vault holds are worth a row.
        let kinds: Vec<_> = KINDS
            .iter()
            .filter(|(kind, ..)| {
                self.items
                    .ready()
                    .is_some_and(|items| items.iter().any(|item| item.kind == *kind))
            })
            .collect();

        let row = |id: &'static str, icon, label: &str, filter: Filter, cx: &mut Context<Self>| {
            let selected = self.filter == filter;
            nav_row(id, icon, label.to_string(), selected)
                .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter.clone(), cx)))
        };

        let header = match look {
            Look::MacOs => chrome::traffic_lights(cx).unwrap_or_else(|| div().into_any_element()),
            Look::Gnome => {
                let main_menu = self.render_main_menu(cx);
                chrome::drag_region(
                    "sidebar-header",
                    div()
                        .relative()
                        .h(metrics.toolbar_height)
                        .flex()
                        .items_center()
                        .child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .font_weight(FontWeight::BOLD)
                                .child("Vault"),
                        )
                        .children(chrome::window_buttons(chrome::Side::Start, window, cx))
                        .child(div().flex_1())
                        .child(main_menu),
                    window,
                    cx,
                )
                .into_any_element()
            }
        };

        let mut rows = vec![
            row("all", IconName::Grid, "All Items", Filter::All, cx).into_any_element(),
            row(
                "favorites",
                IconName::Star,
                "Favorites",
                Filter::Favorites,
                cx,
            )
            .into_any_element(),
        ];
        if !kinds.is_empty() {
            rows.push(section_label("Types", cx).into_any_element());
        }
        for (kind, icon, label) in kinds {
            let id = match kind {
                ItemKind::Login => "logins",
                ItemKind::Card => "cards",
                ItemKind::Identity => "identities",
                ItemKind::SecureNote => "notes",
                ItemKind::SshKey => "ssh-keys",
            };
            rows.push(row(id, *icon, label, Filter::Kind(*kind), cx).into_any_element());
        }
        if !self.folders.is_empty() {
            rows.push(section_label("Folders", cx).into_any_element());
        }
        for folder in self.folders.clone() {
            let selected = self.filter == Filter::Folder(folder.id.clone());
            let filter = Filter::Folder(folder.id.clone());
            rows.push(
                nav_row(
                    gpui::ElementId::Name(format!("folder-{}", folder.id.0).into()),
                    IconName::Folder,
                    folder.name,
                    selected,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.set_filter(filter.clone(), cx)))
                .into_any_element(),
            );
        }

        let name = accounts::name(&self.account);
        let shown = accounts::short_name(&self.account, &self.accounts);
        let theme = cx.theme();
        let (hover, pressed, dim, separator) = (
            theme.colors.hover,
            theme.colors.fill,
            theme.colors.text_secondary,
            theme.colors.separator,
        );
        let switcher = BaseButton::new("account")
            .accessibility_label(format!("{name}, accounts"))
            .on_click(cx.listener(|this, _, window, cx| accounts::toggle(this, window, cx)))
            .flex_1()
            .min_w_0()
            .justify_start()
            .gap(px(8.))
            .p(px(6.))
            .rounded(px(10.))
            .hover(move |s| s.bg(hover))
            .active(move |s| s.bg(pressed))
            .focus_visible(row_focus_ring(cx))
            .when(self.account_menu_open, |b| b.bg(pressed))
            .child(avatar(&name, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(shown),
            );
        let menu = accounts::menu(self, &self.accounts, &self.account.id, Opening::Above, cx);
        // Rows scrolling under the account stop short of it; GNOME divides
        // a sidebar's bottom bar with a hairline across the sidebar.
        let account = div()
            .relative()
            .mt_auto()
            .pt(px(4.))
            .when(look == Look::Gnome, |d| {
                d.mt(px(2.))
                    .pt(px(6.))
                    .mx(px(-6.))
                    .px(px(6.))
                    .border_t_1()
                    .border_color(separator)
            })
            .flex()
            .items_center()
            .gap(px(2.))
            .child(switcher)
            .child(
                div().pr(px(4.)).text_color(dim).child(
                    button("lock")
                        .icon(IconName::Lock)
                        .style(ButtonStyle::Flat)
                        .size(px(26.))
                        .accessibility_label("Lock")
                        .on_click(cx.listener(|this, _, window, cx| this.lock(&Lock, window, cx))),
                ),
            )
            .child(menu);

        div()
            .id("sidebar")
            .flex_none()
            .w(metrics.sidebar_width)
            .h_full()
            .p(metrics.sidebar_inset)
            .rounded_l(corner)
            // The window's colour around the floating sidebar. On macOS the
            // sidebar's glass is under this pane, which a background would
            // cover.
            .map(|d| {
                if cfg!(target_os = "macos") {
                    d.relative().child(glass_surround(
                        metrics.sidebar_inset,
                        glass_radius,
                        window_fill,
                    ))
                } else {
                    d.bg(window_fill)
                }
            })
            .child(
                glass("sidebar-glass", Material::Sidebar)
                    .size_full()
                    .flex()
                    .flex_col()
                    .rounded_l(glass_radius)
                    .when(look == Look::MacOs, |d| d.rounded_r(glass_radius))
                    .px(match look {
                        Look::MacOs => px(7.),
                        Look::Gnome => px(6.),
                    })
                    .pb(px(7.))
                    .when(look == Look::Gnome, |d| {
                        d.border_r_1().border_color(cx.theme().colors.separator)
                    })
                    .child(header)
                    .child(
                        div()
                            .id("sidebar-rows")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .flex()
                            .flex_col()
                            .gap(px(1.))
                            .children(rows),
                    )
                    .child(account),
            )
            .into_any_element()
    }
}
