//! The sidebar: what to show, and the account at the bottom.

use gpui::{
    AnyElement, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _,
    StatefulInteractiveElement as _, Styled as _, Window, div, prelude::FluentBuilder as _, px,
};
use rbw_desktop_core::ItemKind;
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, IconName, Look, Material, avatar, button, chrome, glass,
    nav_row, section_label,
};

use super::{Filter, Lock, VaultView};

const KINDS: [(ItemKind, IconName, &str); 5] = [
    (ItemKind::Login, IconName::Key, "Logins"),
    (ItemKind::Card, IconName::Card, "Cards"),
    (ItemKind::Identity, IconName::Identity, "Identities"),
    (ItemKind::SecureNote, IconName::Note, "Notes"),
    (ItemKind::SshKey, IconName::Terminal, "SSH Keys"),
];

impl VaultView {
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
            Look::Gnome => chrome::drag_region(
                "sidebar-header",
                div()
                    .h(metrics.toolbar_height)
                    .flex()
                    .items_center()
                    .justify_center()
                    .font_weight(FontWeight::BOLD)
                    .children(chrome::window_buttons(chrome::Side::Start, window, cx))
                    .child("Vault"),
                window,
                cx,
            )
            .into_any_element(),
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

        let email = self.account.email.clone().unwrap_or_default();
        let theme = cx.theme();
        let account = div()
            .mt_auto()
            .flex()
            .items_center()
            .gap(px(8.))
            .px(px(6.))
            .py(px(6.))
            .child(avatar(&email, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(email),
            )
            .child(
                div().text_color(theme.colors.text_secondary).child(
                    button("lock")
                        .icon(IconName::Lock)
                        .style(ButtonStyle::Flat)
                        .size(px(26.))
                        .accessibility_label("Lock")
                        .on_click(cx.listener(|this, _, window, cx| this.lock(&Lock, window, cx))),
                ),
            );

        div()
            .id("sidebar")
            .flex_none()
            .w(metrics.sidebar_width)
            .h_full()
            .p(metrics.sidebar_inset)
            .rounded_l(corner)
            // On macOS the window's own material shows around the floating
            // sidebar; elsewhere the window colour does.
            .when(!cfg!(target_os = "macos"), |d| d.bg(window_fill))
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
