//! About: the app's version, the rbw it runs on, its licence and where it
//! lives.

use gpui::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px};
use rbw_desktop_core::APP_NAME;
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, app_icon, button, screen_title, setting_row, settings_group,
};

use crate::backend;

const WEBSITE: &str = env!("CARGO_PKG_REPOSITORY");

pub struct AboutPage {
    /// `None` until rbw answers, or if it can't.
    rbw_version: Option<String>,
}

impl AboutPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        backend::request(
            cx,
            |backend| backend.rbw_version(),
            |this, result, cx| match result {
                Ok(version) => {
                    this.rbw_version = Some(version);
                    cx.notify();
                }
                Err(error) => log::warn!("asking rbw its version: {error}"),
            },
        )
        .detach();
        Self { rbw_version: None }
    }
}

impl Render for AboutPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let dim = cx.theme().colors.text_secondary;
        let accent = cx.theme().colors.accent_text;
        let website = WEBSITE.trim_start_matches("https://");
        div()
            .flex()
            .flex_col()
            .gap(px(22.))
            .child(
                div()
                    .pt(px(8.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(app_icon(cx))
                    .child(screen_title(APP_NAME).mt(px(12.)))
                    .child(
                        div()
                            .mt(px(2.))
                            .text_color(dim)
                            .child(format!("Version {}", env!("CARGO_PKG_VERSION"))),
                    ),
            )
            .child(settings_group(
                None,
                [
                    setting_row(
                        "rbw",
                        None,
                        div()
                            .text_color(dim)
                            .child(self.rbw_version.clone().unwrap_or_default()),
                        cx,
                    ),
                    setting_row(
                        "License",
                        None,
                        div().text_color(dim).child(env!("CARGO_PKG_LICENSE")),
                        cx,
                    ),
                    setting_row(
                        "Website",
                        None,
                        div().text_color(accent).child(
                            button("website")
                                .label(website)
                                .style(ButtonStyle::Link)
                                .on_click(|_, _, cx| cx.open_url(WEBSITE)),
                        ),
                        cx,
                    ),
                ],
                cx,
            ))
    }
}
