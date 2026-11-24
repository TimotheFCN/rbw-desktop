mod accounts;
mod backend;
mod clipboard;
mod demo;
mod generator;
mod icons;
mod instance;
mod lock;
mod panel;
mod preferences;
mod recent;
mod reprompt;
mod settings;
mod sign_in;
mod tray;
mod vault;
mod window;

use std::sync::Arc;

use gpui::{App, KeyBinding, Menu, MenuItem, QuitMode, actions};
use rbw_desktop_core::Backend;

/// A key binding with the platform's command modifier: Command on macOS,
/// Control elsewhere.
#[macro_export]
macro_rules! secondary {
    ($key:literal) => {
        if cfg!(target_os = "macos") {
            concat!("cmd-", $key)
        } else {
            concat!("ctrl-", $key)
        }
    };
}

actions!(
    rbw_desktop,
    [
        Quit,
        /// Closes the window that has the keyboard; each window handles it.
        CloseWindow
    ]
);

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let demo = demo::words().is_some();
    if !demo && instance::hand_over() {
        return;
    }
    let backend = backend::connect();

    // The app quits by itself, below.
    let app = gpui::application()
        .with_assets(rbw_desktop_ui::Assets)
        .with_quit_mode(QuitMode::Explicit);
    // macOS: the Dock icon clicked while no window is open.
    app.on_reopen(|cx| window::with_main_window(cx, |_, _, _| {}));
    app.run(move |cx| {
        rbw_desktop_ui::init(cx);
        sign_in::init(cx);
        vault::init(cx);
        settings::init(cx);
        panel::init(cx);
        cx.bind_keys([
            KeyBinding::new(secondary!("q"), Quit, None),
            KeyBinding::new(secondary!("w"), CloseWindow, None),
        ]);
        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.set_menus(menus());
        // Without a menu bar item to come back to, closing the last
        // window ends the app.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() && !tray::keeps_app_running(cx) {
                cx.quit();
            }
        })
        .detach();
        if !demo {
            instance::listen(cx);
        }
        let missing = match backend {
            Ok(backend) => {
                start(backend, cx);
                None
            }
            Err(error) => Some(error),
        };
        if missing.is_none() && demo::wants("panel") {
            panel::open(panel::Anchor::Centered, cx);
        } else {
            window::open(missing, cx);
        }
        cx.activate(true);
    });
}

/// Starts what runs on the backend, once there is one.
pub fn start(backend: Arc<dyn Backend>, cx: &mut App) {
    backend::init(backend, cx);
    clipboard::init(cx);
    icons::init(cx);
    preferences::init(cx);
    accounts::init(cx);
    tray::init(cx);
}

fn menus() -> Vec<Menu> {
    vec![
        Menu::new("rbw-desktop").items([
            MenuItem::action("About rbw-desktop", settings::About),
            MenuItem::separator(),
            MenuItem::action("Settings…", settings::OpenSettings),
            MenuItem::separator(),
            MenuItem::action("Quit rbw-desktop", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Item", vault::NewItem),
            MenuItem::action("Delete Item…", vault::DeleteItem),
            MenuItem::separator(),
            MenuItem::action("Add Account…", window::AddAccount),
            MenuItem::separator(),
            MenuItem::action("Close Window", CloseWindow),
        ]),
        Menu::new("Vault").items([
            MenuItem::action("Find", vault::Find),
            MenuItem::action("Password Generator", vault::ShowGenerator),
            MenuItem::separator(),
            MenuItem::action("Sync Now", vault::Sync),
            MenuItem::action("Lock", vault::Lock),
        ]),
    ]
}
