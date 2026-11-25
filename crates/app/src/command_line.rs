//! The command line tool and another `rbw`: how the app names them, and
//! the main window's notice of an `rbw` that would lock the vault.

use std::path::Path;

use gpui::{Context, IntoElement, ParentElement as _, Render, Styled as _, Task, Window, div};
use rbw_desktop_core::{APP_NAME, OtherRbw};
use rbw_desktop_ui::{ButtonStyle, button, notice};

use crate::backend;
use crate::preferences::{self, Prefs};
use crate::settings::{self, Page};

/// A path as people read it, with `~` for their home folder.
pub fn shown(path: &Path) -> String {
    let home = std::env::var_os("HOME").filter(|home| !home.is_empty());
    match home.and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

/// What another `rbw` does and what to do about it, for Settings, next to
/// the command line tool.
pub fn settings_advice(other: &OtherRbw) -> String {
    let path = shown(&other.path);
    let version = other
        .version
        .as_ref()
        .map(|version| format!(" ({version})"))
        .unwrap_or_default();
    format!(
        "Another rbw{version} is installed at {path}. It and {APP_NAME} lock each other’s \
         vault: remove it and use the command line tool instead."
    )
}

/// The same, under the main window's notice, whose title says that another
/// rbw is installed.
fn notice_advice(other: &OtherRbw) -> String {
    let path = shown(&other.path);
    let version = other
        .version
        .as_ref()
        .map(|version| format!(" ({version})"))
        .unwrap_or_default();
    format!(
        "The rbw at {path}{version} and {APP_NAME} lock each other’s vault. Remove it and \
         use {APP_NAME}’s command line tool instead."
    )
}

/// Says once, in the main window, that another `rbw` would lock the vault:
/// once for each one found, until the person dismisses it or opens
/// Settings from it. Settings says it for as long as it is there.
pub struct OtherRbwNotice {
    other: Option<OtherRbw>,
    open: bool,
    _task: Task<()>,
}

impl OtherRbwNotice {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let task = backend::request(
            cx,
            |backend| backend.other_rbw(),
            |this, result, cx| match result {
                Ok(Some(other)) => {
                    // The main window opens once the preferences are read.
                    let dismissed = Prefs::global(cx).read(cx).get().dismissed_rbw.clone();
                    if dismissed.as_ref() != Some(&other) {
                        this.other = Some(other);
                        this.open = true;
                        cx.notify();
                    }
                }
                Ok(None) => {}
                Err(error) => log::warn!("looking for another rbw: {error}"),
            },
        );
        Self {
            other: None,
            open: false,
            _task: task,
        }
    }

    fn dismiss(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        if let Some(other) = self.other.clone() {
            preferences::update(cx, move |preferences| {
                preferences.dismissed_rbw = Some(other.clone())
            });
        }
        cx.notify();
    }
}

impl Render for OtherRbwNotice {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let floating = self.other.as_ref().and_then(|other| {
            notice(
                "other-rbw",
                "Another rbw is installed",
                notice_advice(other),
            )
            .action(
                button("other-rbw-settings")
                    .label(format!("Open {}", settings::title(cx)))
                    .style(ButtonStyle::Normal)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.dismiss(cx);
                        settings::open(Some(Page::General), cx);
                    })),
            )
            .on_dismiss(cx.listener(|this, _, _, cx| this.dismiss(cx)))
            .floating(self.open, window, cx)
        });
        // Over the whole window, like the alerts, without taking a click
        // outside the notice.
        div().absolute().inset_0().children(floating)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_which_rbw_and_what_to_do() {
        let homebrew = OtherRbw {
            path: "/opt/homebrew/bin/rbw".into(),
            version: Some("1.18.1".into()),
        };
        assert_eq!(
            settings_advice(&homebrew),
            format!(
                "Another rbw (1.18.1) is installed at /opt/homebrew/bin/rbw. It and {APP_NAME} \
                 lock each other’s vault: remove it and use the command line tool instead."
            )
        );
        assert_eq!(
            notice_advice(&homebrew),
            format!(
                "The rbw at /opt/homebrew/bin/rbw (1.18.1) and {APP_NAME} lock each other’s \
                 vault. Remove it and use {APP_NAME}’s command line tool instead."
            )
        );
        let original = OtherRbw {
            path: "/usr/bin/rbw".into(),
            version: None,
        };
        assert!(
            settings_advice(&original).starts_with("Another rbw is installed at /usr/bin/rbw.")
        );
        assert!(notice_advice(&original).starts_with("The rbw at /usr/bin/rbw and "));
    }

    #[test]
    fn paths_in_the_home_folder_start_with_a_tilde() {
        let Some(home) = std::env::var_os("HOME").filter(|home| !home.is_empty()) else {
            return;
        };
        let home = Path::new(&home);
        assert_eq!(shown(&home.join(".local/bin/rbw")), "~/.local/bin/rbw");
        assert_eq!(shown(Path::new("/usr/local/bin/rbw")), "/usr/local/bin/rbw");
    }
}
