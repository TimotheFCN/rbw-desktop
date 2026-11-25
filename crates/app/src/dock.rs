//! The app's place in the macOS Dock. It is there while a window is open.
//! Once only the menu bar item is left, the app leaves the Dock and the
//! app switcher, as other apps that live in the menu bar do, and comes
//! back with its next window. Other systems have nothing of the kind.

use gpui::App;

use crate::{panel, tray};

/// Before a window opens: in the Dock, with the app's menus.
pub fn show() {
    set(true);
}

/// After a window closed: out of the Dock if the menu bar item, with or
/// without its panel, is all that is left. Not while the item itself is
/// being updated, since this asks whether it shows.
pub fn follow(cx: &App) {
    let panel = panel::window(cx);
    let has_windows = cx.windows().into_iter().any(|window| Some(window) != panel);
    set(has_windows || !tray::keeps_app_running(cx));
}

#[cfg(target_os = "macos")]
fn set(shown: bool) {
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
    use objc2_foundation::MainThreadMarker;

    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(main_thread);
    let policy = if shown {
        NSApplicationActivationPolicy::Regular
    } else {
        NSApplicationActivationPolicy::Accessory
    };
    if app.activationPolicy() != policy && !app.setActivationPolicy(policy) {
        log::warn!("the system refused to change whether the app is in the Dock");
    }
}

#[cfg(not(target_os = "macos"))]
fn set(_: bool) {}
