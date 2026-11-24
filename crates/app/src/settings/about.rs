//! About: the app's version, the rbw it runs on, its licence and where it
//! lives, and whether the rbw command line needs updating.

use gpui::{Context, IntoElement, ParentElement as _, Render, Styled as _, Window, div, px};
use rbw_desktop_core::{APP_NAME, OutdatedRbw};
use rbw_desktop_ui::{
    ActiveTheme as _, ButtonStyle, app_icon, button, screen_title, setting_row, settings_group,
};

use super::note;
use crate::backend;

const WEBSITE: &str = env!("CARGO_PKG_REPOSITORY");

pub struct AboutPage {
    /// `None` until rbw answers, or if it can't.
    rbw_version: Option<String>,
    /// The older rbw on `PATH`, when the app uses its own instead.
    outdated: Option<OutdatedRbw>,
}

impl AboutPage {
    pub fn new(cx: &mut Context<Self>) -> Self {
        backend::request(
            cx,
            |backend| Ok((backend.rbw_version()?, backend.outdated_rbw())),
            |this, result, cx| match result {
                Ok((version, outdated)) => {
                    this.rbw_version = Some(version);
                    this.outdated = outdated;
                    cx.notify();
                }
                Err(error) => log::warn!("asking rbw its version: {error}"),
            },
        )
        .detach();
        Self {
            rbw_version: None,
            outdated: None,
        }
    }
}

/// What to do about an older rbw command line, in plain words.
fn outdated_note(outdated: &OutdatedRbw) -> String {
    format!(
        "The rbw command line at {} is too old ({}), so {APP_NAME} uses its own. \
         Update it: while the two differ, each locks your vault whenever the other is used.",
        outdated.path.display(),
        outdated.version.trim(),
    )
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
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
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
                    .children(self.outdated.as_ref().map(|outdated| {
                        note(outdated_note(outdated), cx)
                            .text_size(cx.theme().metrics.small_text_size)
                    })),
            )
    }
}
