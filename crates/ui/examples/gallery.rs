//! Every component of the design system on one page, for screenshots:
//! `tools/shot --theme gnome-dark gallery out.png`.

use gpui::base::input::InputState;
use gpui::base::slider::SliderState;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, IntoElement, ParentElement as _,
    Render, Styled as _, Window, div, px, size,
};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, ConfirmDialog, Confirmation, IconName, Theme, ToastSlot,
    app_icon, avatar, button, card, chrome, copied_badge, field_error, field_row, form_row, icon,
    list_row, menu_item, menu_separator, nav_row, password_text, progress_ring, search_field,
    section_label, segmented_control, slider, surface, switch, text_field, tile, toggle_row,
};

struct Gallery {
    search: Entity<InputState>,
    password: Entity<InputState>,
    name: Entity<InputState>,
    length: Entity<SliderState>,
    segment: usize,
    switch_on: bool,
    toast: ToastSlot,
    alert: Entity<ConfirmDialog>,
}

impl Gallery {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_appearance(window, |_, window, cx| {
            Theme::follow_window_appearance(window, cx)
        })
        .detach();
        let mut this = Self {
            search: cx.new(|cx| InputState::new(window, cx).placeholder("Search")),
            password: cx.new(|cx| InputState::new(window, cx).placeholder("Master password")),
            name: cx.new(|cx| InputState::new(window, cx).default_value("Netflix")),
            length: cx.new(|_| SliderState::new().min(8.).max(64.).default_value(20.)),
            segment: 0,
            switch_on: true,
            toast: ToastSlot::default(),
            alert: cx.new(|cx| {
                ConfirmDialog::new(
                    Confirmation {
                        title: "Delete “Netflix”?".into(),
                        message: "It moves to the trash of your vault.".into(),
                        confirm: "Delete".into(),
                        destructive: true,
                    },
                    cx,
                )
            }),
        };
        this.toast.show(
            "The server can't be reached.",
            |this: &mut Self| &mut this.toast,
            cx,
        );
        this
    }
}

fn section(title: &'static str, content: impl IntoElement, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .child(
            div()
                .text_size(px(11.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(cx.theme().colors.text_secondary)
                .child(title),
        )
        .child(content)
        .into_any_element()
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let this = cx.entity().downgrade();
        let buttons = div()
            .flex()
            .gap(px(8.))
            .child(button("normal").label("Edit"))
            .child(button("primary").label("Save").style(ButtonStyle::Primary))
            .child(
                button("round")
                    .icon(IconName::Plus)
                    .accessibility_label("New Item"),
            )
            .child(
                button("flat")
                    .icon(IconName::Star)
                    .style(ButtonStyle::Flat)
                    .accessibility_label("Favorite"),
            )
            .child(button("disabled").label("Disabled").disabled(true));
        let more_buttons = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .child(
                button("busy")
                    .label("Sign In")
                    .style(ButtonStyle::Primary)
                    .busy(true),
            )
            .child(
                button("delete")
                    .label("Delete")
                    .style(ButtonStyle::Destructive),
            )
            .child(
                button("link")
                    .label("Unlock with Touch ID")
                    .icon(IconName::TouchId)
                    .style(ButtonStyle::Link),
            );

        let choices = div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                segmented_control(
                    "segments",
                    ["Login", "Card", "Identity", "Note"],
                    self.segment,
                )
                .on_select({
                    let this = this.clone();
                    move |index, _, cx| {
                        this.update(cx, |this, cx| {
                            this.segment = index;
                            cx.notify();
                        })
                        .ok();
                    }
                }),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(slider(&self.length, "Length"))
                    .child(switch("switch", self.switch_on, "Numbers").on_change(
                        move |on, _, cx| {
                            this.update(cx, |this, cx| {
                                this.switch_on = on;
                                cx.notify();
                            })
                            .ok();
                        },
                    ))
                    .child(progress_ring(0.6, cx)),
            );

        let fields = div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(div().flex().child(search_field(&self.search)))
            .child(
                text_field("password", &self.password, "Master password").trailing(
                    button("unlock")
                        .icon(IconName::ArrowRight)
                        .style(ButtonStyle::Primary)
                        .size(px(28.))
                        .accessibility_label("Unlock"),
                ),
            )
            .child(field_error("Incorrect password. Try again.", cx))
            .child(
                div()
                    .font_family(cx.theme().mono_font_family.clone())
                    .child(password_text("vT7#kq2Lw!m9Rz@eB4xN^pd", cx)),
            );

        let rows = div()
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(nav_row("nav-all", IconName::Grid, "All Items", true))
            .child(nav_row("nav-fav", IconName::Star, "Favorites", false))
            .child(section_label("Folders", cx))
            .child(nav_row("nav-folder", IconName::Folder, "Family", false))
            .child(div().h(px(8.)))
            .child(list_row(
                "row-amazon",
                "Amazon",
                Some("alex@example.org".into()),
                true,
            ))
            .child(list_row(
                "row-netflix",
                "Netflix",
                Some("family@example.org".into()),
                false,
            ));

        let cards = card()
            .row(field_row("username", "Username", div().child("alex@example.org")).copied(true))
            .row(
                field_row("password-row", "Password", div().child("••••••••••••"))
                    .action(icon(IconName::Eye))
                    .action(icon(IconName::Copy)),
            )
            .row(field_row(
                "code",
                "Verification code",
                div()
                    .flex()
                    .gap(px(7.))
                    .items_center()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("492 113")
                    .child(progress_ring(0.6, cx)),
            ));

        let sheet = surface()
            .w(px(320.))
            .rounded(px(18.))
            .p(px(16.))
            .gap(px(12.))
            .child(div().font_weight(FontWeight::BOLD).child("Sheet"))
            .child(
                card()
                    .on_surface()
                    .row(form_row(
                        "Name",
                        gpui::base::input::Input::new(&self.name),
                        cx,
                    ))
                    .row(toggle_row(
                        "Add to Favorites",
                        switch("fav", false, "Add to Favorites"),
                    )),
            );

        let popover = surface()
            .w(px(220.))
            .rounded(px(12.))
            .p(px(8.))
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(tile("Spotify"))
                    .child(tile("PayPal")),
            )
            .child(copied_badge(cx));

        let menu = surface()
            .w(px(240.))
            .p(px(5.))
            .rounded(px(10.))
            .child(
                menu_item("menu-alex", "alex@example.org")
                    .leading(avatar("alex@example.org", cx))
                    .detail("Unlocked")
                    .checked(true),
            )
            .child(
                menu_item("menu-sam", "sam@example.com")
                    .leading(avatar("sam@example.com", cx))
                    .detail("Locked"),
            )
            .child(menu_separator(cx))
            .child(menu_item("menu-add", "Add Account…").icon(IconName::Plus));
        let alert = surface()
            .w(px(300.))
            .rounded(px(18.))
            .p(px(16.))
            .child(self.alert.clone());

        let column = |children: Vec<AnyElement>| {
            div()
                .flex()
                .flex_col()
                .gap(px(22.))
                .w(px(360.))
                .children(children)
        };
        let toast = self.toast.render(window, cx);
        let content = div()
            .relative()
            .size_full()
            .flex()
            .gap(px(32.))
            .p(px(28.))
            .bg(cx.theme().colors.window)
            .child(column(vec![
                section(
                    "Buttons",
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(10.))
                        .child(buttons)
                        .child(more_buttons),
                    cx,
                ),
                section("Choices", choices, cx),
                section("Fields", fields, cx),
                section("Sidebar and list rows", rows, cx),
            ]))
            .child(column(vec![
                section("Card with field rows", cards, cx),
                section("Sheet", sheet, cx),
                section("Popover", popover, cx),
            ]))
            .child(column(vec![
                section("App icon", app_icon(cx), cx),
                section("Menu", menu, cx),
                section("Alert", alert, cx),
            ]))
            .children(toast);
        chrome::window_frame(content, window, cx)
    }
}

fn main() {
    gpui::application()
        .with_assets(rbw_desktop_ui::Assets)
        .run(|cx| {
            rbw_desktop_ui::init(cx);
            let options = chrome::window_options(
                "Gallery",
                size(px(1240.), px(780.)),
                size(px(1240.), px(600.)),
                cx,
            );
            gpui::open_window(options, cx, |window, cx| {
                cx.new(|cx| Gallery::new(window, cx))
            })
            .expect("open the gallery window");
            cx.on_window_closed(|cx, _| cx.quit()).detach();
            cx.activate(true);
        });
}
