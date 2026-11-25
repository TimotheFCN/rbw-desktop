mod accounts;
mod alerts;
mod backend;
mod clipboard;
mod command_line;
mod demo;
#[cfg(feature = "frame-stats")]
mod frame_stats;
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
mod touch_id;
mod tray;
mod vault;
mod window;

use std::sync::Arc;

use gpui::{App, KeyBinding, Menu, MenuItem, QuitMode, actions};
use rbw_desktop_core::{APP_NAME, Backend, Error};

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
    let instance = if demo {
        None
    } else {
        match instance::claim() {
            instance::Claim::First(instance) => Some(instance),
            instance::Claim::HandedOver => return,
            instance::Claim::Alone => None,
        }
    };
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
        #[cfg(feature = "frame-stats")]
        frame_stats::init(cx);
        // Without a menu bar item to come back to, closing the last
        // window ends the app.
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() && !tray::keeps_app_running(cx) {
                cx.quit();
            }
        })
        .detach();
        if let Some(instance) = instance {
            instance::listen(instance, cx);
        }
        let missing = match backend {
            Ok(backend) => {
                start(backend, cx);
                None
            }
            Err(error) => Some(error),
        };
        match missing {
            None if demo::wants("panel") => panel::open(panel::Anchor::Centered, cx),
            // Where the window was last comes with the preferences.
            None => preferences::when_read(cx, |cx| open_windows(None, cx)),
            missing => open_windows(missing, cx),
        }
        cx.activate(true);
    });
}

/// Opens the main window, and Settings over it if the demo asks.
fn open_windows(missing: Option<Error>, cx: &mut App) {
    window::open(missing, cx);
    if demo::wants("settings") {
        settings::open(None, cx);
    }
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
        Menu::new(APP_NAME).items([
            MenuItem::action(format!("About {APP_NAME}"), settings::About),
            MenuItem::separator(),
            MenuItem::action("Settings…", settings::OpenSettings),
            MenuItem::separator(),
            MenuItem::action(format!("Quit {APP_NAME}"), Quit),
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
            MenuItem::separator(),
            MenuItem::action("Copy Password", vault::CopyPassword),
            MenuItem::action("Copy Username", vault::CopyUsername),
            MenuItem::action("Copy Verification Code", vault::CopyCode),
            MenuItem::action("Password Generator", vault::ShowGenerator),
            MenuItem::separator(),
            MenuItem::action("Sync Now", vault::Sync),
            MenuItem::action("Lock", vault::Lock),
        ]),
    ]
}
