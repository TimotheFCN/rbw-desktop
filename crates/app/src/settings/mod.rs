//! The Settings window, Preferences on GNOME: General, Accounts, SSH Agent
//! and About. On macOS a sidebar lists the pages, as System Settings does;
//! on GNOME a view switcher at the top of the window does, as Adwaita's
//! preferences do. Changes apply at once; there is nothing to save.

mod about;
mod accounts;
mod general;
mod ssh;
mod store;

use gpui::base::Button as BaseButton;
use gpui::{
    AnyElement, AnyView, App, AppContext as _, Context, ElementId, Entity, FocusHandle, Focusable,
    FontWeight, InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, WindowOptions, actions,
    div, prelude::FluentBuilder as _, px, size,
};
use rbw_desktop_core::APP_ID;
use rbw_desktop_ui::{
    ActiveTheme as _, IconName, Look, Theme, chrome, icon, motion, nav_row, row_focus_ring,
    scroll_area, selection_track, sidebar,
};

use crate::CloseWindow;
use crate::alerts::{self, Alerts};
use crate::backend;
use crate::preferences::{self, Prefs};
use crate::secondary;
use about::AboutPage;
use accounts::AccountsPage;
use general::GeneralPage;
use ssh::SshPage;
use store::{SaveFailed, Store};

actions!(
    settings,
    [
        /// Opens the Settings window, or brings it forward.
        OpenSettings,
        /// Opens the Settings window on its About page.
        About,
        CloseSettings,
    ]
);

const CONTEXT: &str = "Settings";

pub fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(secondary!(","), OpenSettings, None)]);
    // An Adwaita dialog closes with Escape; a Mac's settings window doesn't.
    if cx.theme().look == Look::Gnome {
        cx.bind_keys([KeyBinding::new("escape", CloseSettings, Some(CONTEXT))]);
    }
    cx.on_action(|_: &OpenSettings, cx| open(None, cx));
    cx.on_action(|_: &About, cx| open(Some(Page::About), cx));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Accounts,
    SshAgent,
    About,
}

const PAGES: [(Page, IconName, &str); 4] = [
    (Page::General, IconName::Gear, "General"),
    (Page::Accounts, IconName::Identity, "Accounts"),
    (Page::SshAgent, IconName::Terminal, "SSH Agent"),
    (Page::About, IconName::Info, "About"),
];

/// The window's title, which each platform names its own way.
pub fn title(cx: &App) -> &'static str {
    match cx.theme().look {
        Look::MacOs => "Settings",
        Look::Gnome => "Preferences",
    }
}

/// Opens the Settings window on `page`, or brings it forward if it is open.
/// Without a backend, such as while rbw is missing, there is nothing to set.
pub fn open(page: Option<Page>, cx: &mut App) {
    if !backend::ready(cx) {
        return;
    }
    if let Some((root, settings)) = crate::window::find_window::<SettingsWindow>(cx) {
        root.update(cx, |_, window, cx| {
            window.activate_window();
            if let Some(page) = page {
                settings.update(cx, |settings, cx| settings.show(page, window, cx));
            }
        })
        .ok();
        return;
    }
    let window_size = match cx.theme().look {
        Look::MacOs => size(px(700.), px(500.)),
        Look::Gnome => size(px(640.), px(560.)),
    };
    let options = WindowOptions {
        app_id: Some(APP_ID.into()),
        ..chrome::window_options(title(cx), window_size, size(px(560.), px(400.)), cx)
    };
    crate::dock::show();
    let opened = gpui::open_window(options, cx, |window, cx| {
        cx.new(|cx| SettingsWindow::new(page.unwrap_or(Page::General), window, cx))
    });
    if let Err(error) = opened {
        log::error!("opening the settings: {error:#}");
    }
}

/// Each page is made the first time it shows, and asks rbw only then.
pub struct SettingsWindow {
    page: Page,
    /// Each account's settings, read once a page that shows them does.
    store: Option<Entity<Store>>,
    general: Option<Entity<GeneralPage>>,
    accounts: Option<Entity<AccountsPage>>,
    ssh: Option<Entity<SshPage>>,
    about: Option<Entity<AboutPage>>,
    alerts: Entity<Alerts>,
    focus: FocusHandle,
}

impl SettingsWindow {
    fn new(page: Page, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_appearance(window, |_, window, cx| {
            Theme::follow_window_appearance(window, cx);
        })
        .detach();
        // What the SSH agent is doing, or which rbw a terminal finds, can
        // change while the window is in the background.
        cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                return;
            }
            match this.page {
                Page::SshAgent => {
                    if let Some(ssh) = &this.ssh {
                        ssh.update(cx, |ssh, cx| ssh.refresh(cx));
                    }
                }
                // The command line tool or another rbw may have changed in a
                // terminal.
                Page::General => {
                    if let Some(general) = &this.general {
                        general.update(cx, |general, cx| general.refresh_command_line(cx));
                    }
                }
                _ => {}
            }
        })
        .detach();
        cx.subscribe_in(
            &Prefs::global(cx),
            window,
            |_, _, preferences::SaveFailed(error), window, cx| {
                alerts::toast(backend::describe(error), window, cx);
            },
        )
        .detach();
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // They may have changed while no window asked for them; an answer
        // already on the way will do.
        crate::accounts::refresh_if_idle(cx);
        let mut this = Self {
            page,
            store: None,
            general: None,
            accounts: None,
            ssh: None,
            about: None,
            alerts: Alerts::new(window, cx),
            focus,
        };
        this.make_page(page, window, cx);
        this
    }

    /// The accounts' settings, read from now on.
    fn store(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Entity<Store> {
        if let Some(store) = &self.store {
            return store.clone();
        }
        let store = cx.new(Store::new);
        cx.subscribe_in(&store, window, |_, _, SaveFailed(error), window, cx| {
            alerts::toast(backend::describe(error), window, cx);
        })
        .detach();
        self.store = Some(store.clone());
        store
    }

    /// Makes the page if it hasn't shown yet.
    fn make_page(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        match page {
            Page::General if self.general.is_none() => {
                let store = self.store(window, cx);
                self.general = Some(cx.new(|cx| GeneralPage::new(store, cx)));
            }
            Page::Accounts if self.accounts.is_none() => {
                let store = self.store(window, cx);
                self.accounts = Some(cx.new(|cx| AccountsPage::new(store, cx)));
            }
            Page::SshAgent if self.ssh.is_none() => {
                let store = self.store(window, cx);
                self.ssh = Some(cx.new(|cx| SshPage::new(store, window, cx)));
            }
            Page::About if self.about.is_none() => {
                self.about = Some(cx.new(AboutPage::new));
            }
            _ => {}
        }
        if let Some(ssh) = &self.ssh {
            ssh.update(cx, |ssh, cx| ssh.set_shown(page == Page::SshAgent, cx));
        }
    }

    fn show(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != page {
            self.page = page;
            self.make_page(page, window, cx);
            cx.notify();
        }
    }

    fn close(&mut self, _: &CloseSettings, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn page_view(&self) -> AnyElement {
        let view: Option<AnyView> = match self.page {
            Page::General => self.general.clone().map(Into::into),
            Page::Accounts => self.accounts.clone().map(Into::into),
            Page::SshAgent => self.ssh.clone().map(Into::into),
            Page::About => self.about.clone().map(Into::into),
        };
        view.map_or_else(|| div().into_any_element(), IntoElement::into_any_element)
    }

    fn page_name(&self) -> &'static str {
        PAGES
            .iter()
            .find(|(page, ..)| *page == self.page)
            .map_or("", |(_, _, name)| name)
    }

    /// The page, scrolling, and fading in when it replaces another.
    fn render_body(&self, max_width: Option<gpui::Pixels>) -> AnyElement {
        scroll_area("settings-body")
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .justify_center()
                    .px(px(20.))
                    .pt(px(6.))
                    .pb(px(24.))
                    .child(motion::replace(
                        ("settings-page", self.page as usize),
                        div()
                            .w_full()
                            .min_w_0()
                            .when_some(max_width, |d, width| d.max_w(width))
                            .child(self.page_view()),
                    )),
            )
            .into_any_element()
    }

    /// macOS: the pages in the sidebar, the page's name over its body.
    fn render_macos(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let metrics = theme.metrics;
        let fill = theme.colors.window;
        let corner = chrome::corner_radius(window, cx);
        let shown = PAGES
            .iter()
            .find(|(page, ..)| *page == self.page)
            .map(|(.., label)| ElementId::from(*label));
        let track = selection_track("settings-selection", shown.as_ref(), window, cx);
        let rows = PAGES.map(|(page, icon_name, label)| {
            nav_row(label, icon_name, label, self.page == page)
                .track(&track)
                .on_click(cx.listener(move |this, _, window, cx| this.show(page, window, cx)))
                .into_any_element()
        });
        let sidebar = sidebar("settings-sidebar")
            .children(chrome::traffic_lights(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .px(metrics.sidebar_padding)
                    .children(rows),
            );
        let toolbar = chrome::drag_region(
            "settings-toolbar",
            div()
                .h(metrics.toolbar_height)
                .flex_none()
                .flex()
                .items_center()
                .px(px(20.))
                .text_size(px(15.))
                .font_weight(FontWeight::BOLD)
                .child(self.page_name()),
            window,
            cx,
        );
        div()
            .size_full()
            .flex()
            .child(sidebar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(fill)
                    .rounded_r(corner)
                    .child(toolbar)
                    .child(self.render_body(Some(px(520.)))),
            )
            .into_any_element()
    }

    /// GNOME: a header bar with the view switcher, the page under it.
    fn render_gnome(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let colors = &theme.colors;
        let (hover, selected) = (colors.hover, colors.fill);
        let height = theme.metrics.toolbar_height;
        let switcher = div()
            .id("view-switcher")
            .role(gpui::Role::TabList)
            .flex()
            .gap(px(4.))
            .children(PAGES.map(|(page, icon_name, label)| {
                let current = self.page == page;
                BaseButton::new(label)
                    .role(gpui::Role::Tab)
                    .selected(current)
                    .accessibility_label(label)
                    .on_click(cx.listener(move |this, _, window, cx| this.show(page, window, cx)))
                    .flex()
                    .items_center()
                    .gap(px(7.))
                    .h(px(34.))
                    .px(px(12.))
                    .rounded(theme.metrics.control_radius)
                    .font_weight(FontWeight::BOLD)
                    .when(current, |b| b.bg(selected))
                    .when(!current, |b| b.hover(move |s| s.bg(hover)))
                    .focus_visible(row_focus_ring(cx))
                    .child(icon(icon_name).size(px(16.)))
                    .child(SharedString::from(label))
            }));
        let header = chrome::drag_region(
            "settings-header",
            div()
                .h(height)
                .flex_none()
                .flex()
                .items_center()
                .px(px(6.))
                .children(chrome::window_buttons(chrome::Side::Start, window, cx))
                .child(div().flex_1().flex().justify_center().child(switcher))
                .children(chrome::window_buttons(chrome::Side::End, window, cx)),
            window,
            cx,
        );
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(self.render_body(Some(px(560.))))
            .into_any_element()
    }
}

impl Focusable for SettingsWindow {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SettingsWindow {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let layout = match cx.theme().look {
            Look::MacOs => self.render_macos(window, cx),
            Look::Gnome => self.render_gnome(window, cx),
        };
        let content = div()
            .id("settings")
            .key_context(CONTEXT)
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::close))
            .on_action(|_: &CloseWindow, window, _| window.remove_window())
            .relative()
            .size_full()
            .child(layout)
            .child(self.alerts.clone());
        chrome::window_frame(content, window, cx)
    }
}

/// A sentence of a settings page, outside any group: what a page is for,
/// or what to do next.
fn note(text: impl Into<SharedString>, cx: &App) -> gpui::Div {
    div()
        .px(px(4.))
        .text_color(cx.theme().colors.text_secondary)
        .line_height(gpui::relative(1.4))
        .child(text.into())
}
